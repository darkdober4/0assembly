//! Scan multi-langages : repère les fonctions susceptibles de gagner en vitesse.
//!
//! Le scanner est volontairement **générique** : il s'appuie sur les descripteurs
//! de [`crate::lang`] (commentaires, chaînes, motifs de définition, mots-clés)
//! et ne contient aucune logique propre à un langage. Ajouter un langage = ajouter
//! une entrée dans le registre.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use regex::Regex;

use crate::lang::{self, BodyStyle, LangProfile};

/// Répertoires ignorés par défaut.
pub const DEFAULT_EXCLUDES: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    ".svn",
    ".hg",
    "vendor",
    "dist",
    "build",
    "out",
    ".venv",
    "venv",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".cargo",
    ".idea",
    ".vscode",
    "coverage",
    "bin",
    "obj",
];

/// Taille maximale d'un fichier analysé (octets).
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Confiance dans le diagnostic d'un candidat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

/// Un candidat issu du scan statique.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScanHit {
    /// Chemin relatif à la racine du projet.
    pub rel_path: String,
    /// Identifiant du langage.
    pub lang: String,
    /// Nom de la fonction.
    pub fn_name: String,
    /// Première ligne (1-based).
    pub start_line: usize,
    /// Dernière ligne (1-based).
    pub end_line: usize,
    /// Nombre de lignes.
    pub lines: usize,
    /// Score de « profitabilité » estimé (statique).
    pub score: f64,
    /// Confiance du diagnostic.
    pub confidence: Confidence,
    /// Signaux détectés, lisibles par un humain.
    pub signals: Vec<String>,
    /// Compteurs bruts derrière les signaux (utilisés pour choisir la stratégie).
    pub metrics: SignalMetrics,
}

/// Compteurs bruts des signaux, mesurés **dans** la boucle.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SignalMetrics {
    /// Nombre de boucles dans la fonction.
    pub loops: usize,
    /// Nombre d'allocations après le début de la première boucle.
    pub allocs: usize,
    /// Nombre d'appels « sensibles » après le début de la première boucle.
    pub hot: usize,
    /// Nombre d'indexations de collection après le début de la première boucle.
    pub indexed: usize,
    /// Nombre d'accumulations numériques (`+=`, `-=`, ...) dans la boucle.
    pub numeric_accum: usize,
    /// Nombre de constructions de collection (`push`, `append`, ...) dans la boucle.
    pub collection_build: usize,
    /// Nombre d'opérations arithmétiques binaires dans la boucle.
    pub arith_ops: usize,
}

/// Options du scan.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Racines à parcourir (répertoires, ou fichiers).
    pub roots: Vec<PathBuf>,
    /// Noms de répertoires exclus.
    pub excludes: Vec<String>,
    /// Nombre maximal de candidats retournés.
    pub max_hits: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            excludes: DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).collect(),
            max_hits: 500,
        }
    }
}

// ---------------------------------------------------------------------------
// Masquage : commentaires et chaînes sont remplacés par des espaces.
// ---------------------------------------------------------------------------

fn starts_with(chars: &[char], at: usize, needle: &str) -> bool {
    let n: Vec<char> = needle.chars().collect();
    if at + n.len() > chars.len() {
        return false;
    }
    chars[at..at + n.len()] == n[..]
}

fn find_seq(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let n: Vec<char> = needle.chars().collect();
    if n.is_empty() || from >= chars.len() {
        return None;
    }
    (from..=chars.len().saturating_sub(n.len())).find(|&i| chars[i..i + n.len()] == n[..])
}

fn line_end(chars: &[char], from: usize) -> usize {
    (from..chars.len())
        .find(|&i| chars[i] == '\n')
        .unwrap_or(chars.len())
}

/// Délimiteur de fin d'une chaîne brute : `"` + les `#` d'ouverture.
fn raw_close(open: &str) -> String {
    let last = open.chars().last().unwrap_or('"');
    let hashes = open.chars().rev().skip(1).take_while(|c| *c == '#').count();
    format!("{last}{}", "#".repeat(hashes))
}

/// Fin d'un littéral de caractère : `'x'` (3 caractères) ou `'\n'` (4).
fn char_literal_end(chars: &[char], at: usize) -> Option<usize> {
    if at + 2 >= chars.len() || chars[at + 1] == '\'' {
        return None;
    }
    if chars[at + 1] == '\\' {
        // Séquence d'échappement : au plus 4 caractères au total.
        if at + 3 < chars.len() && chars[at + 3] == '\'' {
            return Some(at + 4);
        }
        return None;
    }
    if chars[at + 2] == '\'' {
        return Some(at + 3);
    }
    None
}

/// Longueur d'ouverture et délimiteur de fermeture d'une chaîne brute Rust.
fn rust_raw_string(chars: &[char], at: usize) -> Option<(usize, String)> {
    let mut i = at + 1;
    let mut hashes = 0usize;
    while i < chars.len() && chars[i] == '#' {
        hashes += 1;
        i += 1;
    }
    if i >= chars.len() || chars[i] != '"' {
        return None;
    }
    let close = format!("\"{}", "#".repeat(hashes));
    Some((i - at + 1, close))
}

fn mask_range(out: &mut [char], from: usize, to: usize) {
    let end = to.min(out.len());
    if from >= end {
        return;
    }
    for c in out[from..end].iter_mut() {
        if *c != '\n' {
            *c = ' ';
        }
    }
}

/// Remplace commentaires et contenus de chaînes par des espaces (les délimiteurs
/// sont conservés, les offsets restent valides).
pub fn mask_source(src: &str, language: &LangProfile) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = chars.clone();
    let n = chars.len();
    let mut i = 0usize;

    while i < n {
        let c = chars[i];
        if let Some((open, close)) = language.block_comment {
            if starts_with(&chars, i, open) {
                let start = i + open.chars().count();
                let end = find_seq(&chars, start, close).unwrap_or(n);
                mask_range(&mut out, i, end);
                i = end;
                continue;
            }
        }
        let mut handled = false;
        for lc in language.line_comments {
            if starts_with(&chars, i, lc) {
                let end = line_end(&chars, i);
                mask_range(&mut out, i, end);
                i = end;
                handled = true;
                break;
            }
        }
        if handled {
            continue;
        }
        // Littéral de caractère : `'x'` ou `'\n'`. En Rust, un `'` qui n'est
        // pas suivi d'un caractère refermé est une durée de vie : on l'ignore.
        if language.char_literals && c == '\'' {
            if let Some(end) = char_literal_end(&chars, i) {
                mask_range(&mut out, i, end);
                i = end;
                handled = true;
            }
        }
        if handled {
            continue;
        }
        // Chaîne brute Rust : r"…", r#"…"#, r##"…"## (nombre de `#` libre).
        if language.id == "rust" && c == 'r' {
            if let Some((open_len, close)) = rust_raw_string(&chars, i) {
                let end = find_seq(&chars, i + open_len, &close)
                    .map(|p| p + close.chars().count())
                    .unwrap_or(n);
                mask_range(&mut out, i + open_len, end);
                i = end;
                handled = true;
            }
        }
        if handled {
            continue;
        }
        for open in language.raw_strings {
            if starts_with(&chars, i, open) {
                let start = i + open.chars().count();
                let close = raw_close(open);
                let end = find_seq(&chars, start, &close).map(|p| p + close.chars().count());
                let end = end.unwrap_or(n);
                mask_range(&mut out, i + open.chars().count(), end);
                i = end;
                handled = true;
                break;
            }
        }
        if handled {
            continue;
        }
        for (delim, escape) in language.strings {
            if starts_with(&chars, i, delim) {
                let dlen = delim.chars().count();
                let start = i + dlen;
                let mut j = start;
                let mut end = n;
                while j < n {
                    if !escape.is_empty() && chars[j] == escape.chars().next().unwrap_or('\\') {
                        j += 2;
                        continue;
                    }
                    if starts_with(&chars, j, delim) {
                        end = j + dlen;
                        break;
                    }
                    j += 1;
                }
                mask_range(&mut out, start, end.saturating_sub(dlen));
                i = end.min(n);
                handled = true;
                break;
            }
        }
        if handled {
            continue;
        }
        i += 1;
    }

    out.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Détection des fonctions
// ---------------------------------------------------------------------------

type RegexCache = Mutex<std::collections::HashMap<String, Option<Regex>>>;

fn regex_cache() -> &'static RegexCache {
    static CACHE: OnceLock<RegexCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn compile_cached(pattern: &str) -> Option<Regex> {
    let mut cache = regex_cache().lock().ok()?;
    cache
        .entry(pattern.to_string())
        .or_insert_with(|| Regex::new(pattern).ok())
        .clone()
}

/// Position (offset) d'une définition de fonction.
#[derive(Debug, Clone)]
struct FnDef {
    name: String,
    offset: usize,
}

/// Repère les définitions de fonctions dans un source déjà masqué.
fn find_functions(masked: &str, language: &LangProfile) -> Vec<FnDef> {
    let mut out: Vec<FnDef> = Vec::new();
    let mut seen: HashSet<(String, usize)> = HashSet::new();

    for pattern in language.fn_patterns {
        let Some(re) = compile_cached(pattern) else {
            continue;
        };
        for caps in re.captures_iter(masked) {
            let Some(name_match) = caps.get(1) else {
                continue;
            };
            let name = name_match.as_str().to_string();
            if lang::RESERVED_NAMES.contains(&name.as_str()) {
                continue;
            }
            let line = masked[..name_match.start()].lines().count();
            if seen.insert((name.clone(), line)) {
                out.push(FnDef {
                    name,
                    offset: caps.get(0).unwrap().start(),
                });
            }
        }
    }
    out.sort_by_key(|f| f.offset);
    out.dedup_by(|a, b| a.offset == b.offset);
    out
}

/// Détermine la fin du corps d'une fonction selon le style du langage.
fn body_end(masked: &str, masked_chars: &[char], def_offset: usize, style: BodyStyle) -> usize {
    match style {
        BodyStyle::Braces => {
            let Some(open) = masked_chars[def_offset..]
                .iter()
                .position(|c| *c == '{')
                .map(|p| p + def_offset)
            else {
                return masked.len();
            };
            let mut depth = 0i32;
            let mut i = open;
            while i < masked_chars.len() {
                match masked_chars[i] {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return i;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            masked.len()
        }
        BodyStyle::Indent => {
            let def_line_start = masked[..def_offset].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let def_indent = masked[def_line_start..def_offset]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .count();
            let mut i = line_end(masked_chars, def_offset) + 1;
            while i < masked.len() {
                let line_end = line_end(masked_chars, i);
                let line = &masked[i..line_end];
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    let indent = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
                    if indent <= def_indent {
                        return i;
                    }
                }
                i = line_end + 1;
            }
            masked.len()
        }
        BodyStyle::End => {
            let def_line_start = masked[..def_offset].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let def_indent = masked[def_line_start..def_offset]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .count();
            let mut i = line_end(masked_chars, def_offset) + 1;
            while i < masked.len() {
                let line_end = line_end(masked_chars, i);
                let line = masked[i..line_end].trim();
                if line == "end" || line.starts_with("end ") || line.starts_with("end.") {
                    let indent = masked[i..line_end]
                        .chars()
                        .take_while(|c| *c == ' ' || *c == '\t')
                        .count();
                    if indent <= def_indent {
                        return i;
                    }
                }
                i = line_end + 1;
            }
            masked.len()
        }
        BodyStyle::Expression => masked.len(),
    }
}

/// Compte les occurrences d'un mot-clé (frontière de mot respectée).
fn count_keyword(haystack: &str, keyword: &str) -> usize {
    let trimmed = keyword.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let bytes = haystack.as_bytes();
    let needle = trimmed.as_bytes();
    let mut count = 0usize;
    let mut i = 0usize;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let before_ok = i == 0 || !is_word_byte(bytes[i - 1]);
            let after = i + needle.len();
            let after_ok = after >= bytes.len() || !is_word_byte(bytes[after]);
            if before_ok && after_ok {
                count += 1;
                i = after;
                continue;
            }
        }
        i += 1;
    }
    count
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Signaux de performance détectés dans un corps de fonction.
///
/// Les compteurs ne valent que pour ce qui suit le **début de la première
/// boucle** : une allocation faite après la boucle n'est pas une allocation
/// « dans une boucle ».
fn detect_signals(body: &str, language: &LangProfile) -> (Vec<String>, f64, SignalMetrics) {
    let mut signals = Vec::new();
    let mut score = 0.0;
    let mut metrics = SignalMetrics::default();

    let loop_positions: Vec<usize> = language
        .loops
        .iter()
        .flat_map(|k| keyword_positions(body, k).into_iter().map(|(p, _)| p))
        .collect();
    metrics.loops = loop_positions.len();
    let loops = metrics.loops;
    let region_start = loop_positions.iter().copied().min().unwrap_or(usize::MAX);

    let count_in_loop = |needle: &str| -> usize {
        keyword_positions(body, needle)
            .into_iter()
            .filter(|(p, _)| *p > region_start)
            .count()
    };

    if loops > 0 {
        signals.push(format!("boucle x{loops}"));
        score += 1.0;
        if loops > 1 {
            signals.push("boucles multiples (imbrication probable)".into());
            score += 2.0;
        }
    }

    let allocs: usize = language.alloc_calls.iter().map(|k| count_in_loop(k)).sum();
    metrics.allocs = allocs;
    if loops > 0 && allocs > 0 {
        signals.push(format!("allocation dans une boucle (x{allocs}, probable)"));
        score += 2.0;
    }

    let hot: usize = language.hot_calls.iter().map(|k| count_in_loop(k)).sum();
    metrics.hot = hot;
    if loops > 0 && hot > 0 {
        signals.push(format!("appel sensible dans une boucle (x{hot})"));
        score += 3.0;
    }

    let indexed = count_indexing(body, region_start);
    metrics.indexed = indexed;
    if loops > 0 && indexed > 0 {
        signals.push(format!(
            "indexation de collection dans une boucle (x{indexed})"
        ));
        score += 2.0;
    }

    // Accumulation : la boucle empile des elements dans un tampon.
    let accum = ACCUM_CALLS.iter().map(|k| count_in_loop(k)).sum::<usize>();
    let numeric = NUMERIC_ACCUM
        .iter()
        .map(|k| count_in_loop(k))
        .sum::<usize>();
    let build = COLLECTION_BUILD
        .iter()
        .map(|k| count_in_loop(k))
        .sum::<usize>();
    metrics.numeric_accum = numeric;
    metrics.collection_build = build;
    metrics.arith_ops = count_arith(body, region_start);
    if loops > 0 && accum > 0 {
        signals.push(format!("accumulation dans une boucle (x{accum})"));
        score += 2.0;
    }
    if loops > 0 && numeric > 0 {
        signals.push(format!(
            "accumulation numérique dans une boucle (x{numeric})"
        ));
        score += 2.0;
    }
    if loops > 0 && build > 0 {
        signals.push(format!(
            "construction de collection dans une boucle (x{build})"
        ));
        score += 2.0;
    }
    if loops > 0 && metrics.arith_ops >= 2 {
        signals.push(format!(
            "calcul élémentaire dans une boucle (x{})",
            metrics.arith_ops
        ));
        score += 1.0;
    }

    let bitwise = body.matches("<<").count()
        + body.matches(">>").count()
        + body.matches(" & ").count()
        + body.matches(" | ").count()
        + body.matches(" ^ ").count();
    if loops > 0 && bitwise > 0 {
        signals.push(format!("opérations bit à bit dans une boucle (x{bitwise})"));
        score += 1.0;
    }

    let branches: usize = language
        .branches
        .iter()
        .map(|k| count_keyword(body, k))
        .sum();
    let body_lines = body.lines().count().max(1);
    if branches as f64 / body_lines as f64 > 0.12 {
        signals.push(format!("branchements denses (x{branches})"));
        score += 1.0;
    }

    (signals, score, metrics)
}

/// Appels d'accumulation : la boucle empile dans un tampon.
const ACCUM_CALLS: &[&str] = &[
    "push_str",
    ".push(",
    ".append(",
    ".add(",
    "append(",
    "concat(",
    "strcat",
    "memcpy",
    "sprintf",
    "+=",
    "format!",
    "push_back",
    "emplace_back",
    ".insert(",
    ".extend(",
];

/// Accumulations numériques : la seule famille qui ouvre la voie au SIMD.
const NUMERIC_ACCUM: &[&str] = &["+=", "-=", "*=", "/="];

/// Construction d'une collection dans la boucle : candidate au hoisting.
const COLLECTION_BUILD: &[&str] = &[
    ".push(",
    ".append(",
    ".extend(",
    "push_str",
    "push_back",
    "emplace_back",
    ".insert(",
];

/// Seuil minimal de score pour qu'une fonction soit un candidat.
pub const MIN_CANDIDATE_SCORE: f64 = 3.0;

/// Positions (décalage, longueur) d'un mot-clé, hors Boundaries d'identifiant.
fn keyword_positions(body: &str, needle: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let bytes = body.as_bytes();
    let n = needle.len();
    let first_is_ident = is_ident_byte(needle.as_bytes()[0]);
    let last_is_ident = is_ident_byte(needle.as_bytes()[n - 1]);
    let mut start = 0usize;
    while start <= bytes.len() {
        let Some(p) = body[start..].find(needle) else {
            break;
        };
        let at = start + p;
        let before_ok = !first_is_ident || at == 0 || !is_ident_byte(bytes[at - 1]);
        let after = at + n;
        let after_ok = !last_is_ident || after >= bytes.len() || !is_ident_byte(bytes[after]);
        if before_ok && after_ok {
            out.push((at, n));
        }
        start = at + n;
    }
    out
}

/// Un octet peut-il faire partie d'un identifiant ?
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Compte les opérations arithmétiques binaires (` + `, ` * `, ...) après `from`.
fn count_arith(body: &str, from: usize) -> usize {
    let mut count = 0usize;
    let bytes = body.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if i <= from {
            continue;
        }
        if !matches!(b, b'+' | b'-' | b'*' | b'/') {
            continue;
        }
        let prev = if i == 0 { None } else { bytes.get(i - 1) };
        let next = bytes.get(i + 1);
        let spaced = prev.is_some_and(|p| p.is_ascii_whitespace())
            && next.is_some_and(|n| n.is_ascii_whitespace());
        let unary = prev.is_some_and(|p| !p.is_ascii_alphanumeric() && *p != b'_');
        if spaced && !unary {
            count += 1;
        }
    }
    count
}

/// Compte les indexations de collection (`a[i]`) dans un corps, après `from`.
fn count_indexing(body: &str, from: usize) -> usize {
    let chars: Vec<char> = body.chars().collect();
    let mut count = 0usize;
    for (i, c) in chars.iter().enumerate() {
        if *c != '[' || i <= from {
            continue;
        }
        let mut j = i + 1;
        let mut depth = 1i32;
        while j < chars.len() && depth > 0 {
            match chars[j] {
                '[' => depth += 1,
                ']' => depth -= 1,
                _ => {}
            }
            j += 1;
        }
        if depth == 0 && j - i > 2 {
            let inner: String = chars[i + 1..j - 1].iter().collect();
            if inner
                .chars()
                .any(|c| c.is_alphanumeric() || c == '_' || c == '.')
            {
                count += 1;
            }
        }
    }
    count
}

/// Analyse un fichier et retourne ses candidats.
pub fn scan_file(path: &Path, rel_path: &str, language: &LangProfile) -> Vec<ScanHit> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Vec::new();
    };
    if meta.len() > MAX_FILE_BYTES {
        return Vec::new();
    }
    let Ok(src) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    scan_source(&src, rel_path, language)
}

/// Analyse un contenu source et retourne ses candidats.
pub fn scan_source(src: &str, rel_path: &str, language: &LangProfile) -> Vec<ScanHit> {
    let masked = mask_source(src, language);
    let chars: Vec<char> = masked.chars().collect();
    let mut hits = Vec::new();

    for def in find_functions(&masked, language) {
        let end = body_end(&masked, &chars, def.offset, language.body);
        let body: String = chars[def.offset..end.min(chars.len())].iter().collect();
        let (signals, score, metrics) = detect_signals(&body, language);
        if score < MIN_CANDIDATE_SCORE {
            continue;
        }
        let start_line = masked[..def.offset].lines().count() + 1;
        let end_line = masked[..end.min(masked.len())].lines().count() + 1;
        let confidence = match signals.len() {
            0..=1 => Confidence::Low,
            2 => Confidence::Medium,
            _ => Confidence::High,
        };
        hits.push(ScanHit {
            rel_path: rel_path.to_string(),
            lang: language.id.to_string(),
            fn_name: def.name.clone(),
            start_line,
            end_line: end_line.max(start_line),
            lines: end_line.saturating_sub(start_line) + 1,
            score,
            confidence,
            signals,
            metrics,
        });
    }

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.rel_path.cmp(&b.rel_path))
    });
    hits
}

/// Parcours un projet et retourne les candidats, triés par score décroissant.
pub fn scan_project(root: &Path, options: &ScanOptions) -> Vec<ScanHit> {
    let excludes: HashSet<&str> = options.excludes.iter().map(|s| s.as_str()).collect();
    let mut hits = Vec::new();

    for scan_root in &options.roots {
        let target = if scan_root.is_absolute() {
            scan_root.clone()
        } else {
            root.join(scan_root)
        };
        if target.is_file() {
            collect_file(&target, root, &mut hits);
            continue;
        }
        let mut stack = vec![target];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if path.is_dir() {
                    if !excludes.contains(name.as_str()) {
                        stack.push(path);
                    }
                } else {
                    collect_file(&path, root, &mut hits);
                }
            }
        }
    }

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.rel_path.cmp(&b.rel_path))
            .then_with(|| a.start_line.cmp(&b.start_line))
    });
    hits.truncate(options.max_hits);
    hits
}

fn collect_file(path: &Path, root: &Path, hits: &mut Vec<ScanHit>) {
    let Some(ext) = path.extension().map(|e| e.to_string_lossy().to_string()) else {
        return;
    };
    let Some(language) = lang::by_extension(&ext) else {
        return;
    };
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    hits.extend(scan_file(path, &rel, language));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::by_extension;

    fn hits(src: &str, ext: &str) -> Vec<ScanHit> {
        let lang = by_extension(ext).unwrap();
        scan_source(src, "test.rs", lang)
    }

    #[test]
    fn rust_loop_with_allocation_is_flagged() {
        let src = r#"
fn parse(lines: &[String]) -> usize {
    let mut total = 0;
    for line in lines {
        let mut parts = Vec::new();
        for p in line.split(',') {
            parts.push(p.to_string());
        }
        total += parts.len();
    }
    total
}
"#;
        let found = hits(src, "rs");
        assert!(!found.is_empty(), "aucun candidat");
        assert_eq!(found[0].fn_name, "parse");
        assert!(found[0].signals.iter().any(|s| s.contains("allocation")));
    }

    #[test]
    fn comments_and_strings_do_not_create_candidates() {
        let src = r#"
// fn fake() { for (;;) { let x = Vec::new(); } }
fn real() -> u32 {
    let s = "fn also_fake() { Vec::new(); }";
    let v: Vec<u32> = Vec::new();
    for i in 0..10 {
        let w = Vec::new();
        v.push(i);
        w.push(i);
    }
    v.len() as u32
}
"#;
        let found = hits(src, "rs");
        assert_eq!(found.len(), 1, "candidats inattendus : {found:?}");
        assert_eq!(found[0].fn_name, "real");
    }

    #[test]
    fn python_indentation_body() {
        let src = "def work(items):\n    out = []\n    for it in items:\n        out.append(it.strip())\n    return out\n";
        let found = hits(src, "py");
        assert!(!found.is_empty());
        assert_eq!(found[0].fn_name, "work");
        assert!(found[0].end_line >= 5);
    }

    #[test]
    fn c_function_detected() {
        let src = "int sum(const int *a, int n) {\n  int t = 0;\n  for (int i = 0; i < n; i++) { t += a[i]; }\n  return t;\n}\n";
        let found = hits(src, "c");
        assert!(!found.is_empty());
        assert_eq!(found[0].fn_name, "sum");
    }

    #[test]
    fn go_function_detected() {
        let src = "func Work(items []string) int {\n\ttotal := 0\n\tfor _, it := range items {\n\t\ttotal += len(it)\n\t}\n\treturn total\n}\n";
        let found = hits(src, "go");
        assert!(!found.is_empty());
        assert_eq!(found[0].fn_name, "Work");
    }

    #[test]
    fn masking_preserves_offsets_and_newlines() {
        let src = "a /* x\ny */ b\n";
        let masked = mask_source(src, by_extension("c").unwrap());
        assert_eq!(masked.lines().count(), src.lines().count());
        assert_eq!(masked.len(), src.len());
    }

    #[test]
    fn reserved_keywords_are_not_functions() {
        let src = "void f(void) {\n  if (x) { for (;;) {} }\n  while (y) { }\n}\n";
        let found = hits(src, "c");
        assert!(found.iter().all(|h| h.fn_name == "f"), "{found:?}");
    }
}
