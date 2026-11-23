//! Vérification de compatibilité d'un patch.
//!
//! Règle fondatrice du projet : **le code réécrit doit rester compatible avec
//! le reste du programme, et le programme doit continuer de tourner**.
//!
//! Cette règle est vérifiée mechaniquement avant toute approbation :
//!
//! 1. **Interface** : la signature de la fonction est identique avant et après
//!    le patch. Le reste du programme compile donc sans modification.
//! 2. **Repli** : un chemin portable subsiste. Le binaire tourne meme si le
//!    chemin optimise n'est pas pris (CPU sans AVX, autre architecture, build
//!    portable). Pour une strategie portable, le repli est le code lui-meme.
//! 3. **Comportement** : les commandes de verification du projet (`[verify]`
//!    dans `0ass.toml`) passent avant **et** apres le patch.

use std::path::Path;
use std::time::Instant;

use zeroass_core::manifest::{Candidate, CommandResult, Compatibility};
use zeroass_core::profile::ProjectProfile;

use crate::runner::CommandSpec;

/// Résultat brut d'une commande de vérification.
fn run_check(line: &str, cwd: &Path) -> CommandResult {
    let spec = CommandSpec::new(line.to_string(), cwd);
    let start = Instant::now();
    let (exit_code, output) = match spec.run() {
        Ok(r) => r,
        Err(e) => (-1, format!("{e}")),
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let tail = output
        .lines()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    CommandResult {
        command: line.to_string(),
        exit_code,
        duration_ms,
        tail,
        ok: exit_code == 0,
    }
}

/// Texte de la signature d'une fonction (de son nom jusqu'à l'accolade).
///
/// Le texte est normalisé (espaces réduits) : seule la **forme** de la
/// signature compte, pas sa mise en forme.
pub fn signature_of(source: &str, fn_name: &str) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let mut search = 0usize;
        while let Some(p) = trimmed[search..].find(fn_name) {
            let pos = search + p;
            let before = trimmed[..pos].chars().next_back();
            let after = trimmed[pos + fn_name.len()..].chars().next();
            let ok_before = !before.is_some_and(|c| c.is_alphanumeric() || c == '_');
            let ok_after = !after.is_some_and(|c| c.is_alphanumeric() || c == '_');
            if ok_before && ok_after {
                let mut text = trimmed.to_string();
                let mut j = idx + 1;
                while !text.contains('{') && !text.contains(';') && j < lines.len() {
                    text.push(' ');
                    text.push_str(lines[j].trim());
                    j += 1;
                }
                let cut = text
                    .find('{')
                    .or_else(|| text.find(';'))
                    .unwrap_or(text.len());
                let normalized = text[..cut].split_whitespace().collect::<Vec<_>>().join(" ");
                return Some(normalized);
            }
            search = pos + fn_name.len();
        }
    }
    None
}

/// Detecte la presence d'un repli portable dans le code patche.
pub fn has_fallback(source: &str) -> bool {
    let markers = [
        "#[cfg(",
        "cfg!(",
        "cfg!(target",
        "target_feature",
        "is_x86_feature_detected",
        "#[target_feature",
        "fallback",
        "repli",
        "scalar",
        "portable",
        "else {",
    ];
    markers.iter().any(|m| source.contains(m))
}

/// Verifie la compatibilite d'un patch entre la base et le worktree patche.
pub fn verify_patch(
    profile: &ProjectProfile,
    base: &Path,
    patched: &Path,
    candidate: &Candidate,
) -> Compatibility {
    let mut result = Compatibility {
        signature_preserved: true,
        fallback_present: false,
        before: Vec::new(),
        after: Vec::new(),
        compatible: false,
        reason: None,
    };

    // 1. Interface : la signature doit etre inchangee.
    let base_src = std::fs::read_to_string(base.join(&candidate.file)).unwrap_or_default();
    let patched_src = std::fs::read_to_string(patched.join(&candidate.file)).unwrap_or_default();
    match (
        signature_of(&base_src, &candidate.fn_name),
        signature_of(&patched_src, &candidate.fn_name),
    ) {
        (Some(a), Some(b)) if a == b => result.signature_preserved = true,
        (None, None) => {
            result.signature_preserved = true;
        }
        _ => {
            result.signature_preserved = false;
            result.reason = Some(
                "la signature de la fonction a change : le reste du programme ne compilera \
                 plus tel quel"
                    .into(),
            );
        }
    }

    // 2. Repli portable : obligatoire pour toute reecriture par architecture.
    let portable_strategy = !candidate
        .targets
        .iter()
        .any(|t| t.contains("per_arch") || t == "x86_64");
    result.fallback_present = portable_strategy || has_fallback(&patched_src);
    if !result.fallback_present {
        let reason = "aucun repli portable dans le patch : le programme ne tournerait pas \
                      sur un processeur ou une architecture non couverte"
            .to_string();
        result.reason = Some(match result.reason.take() {
            Some(prev) => format!("{prev} ; {reason}"),
            None => reason,
        });
    }

    // 3. Comportement : les verifications du projet passent avant et apres.
    let commands: Vec<String> = profile
        .verify
        .check
        .iter()
        .chain(profile.verify.test.iter())
        .cloned()
        .collect();
    for line in &commands {
        result.before.push(run_check(line, base));
        result.after.push(run_check(line, patched));
    }
    let base_ok = result.before.iter().all(|c| c.ok);
    let patched_ok = result.after.iter().all(|c| c.ok);
    if !base_ok {
        let reason = "les verifications du projet echouent deja avant le patch : la base \
                      doit etre reparee d'abord"
            .to_string();
        result.reason = Some(match result.reason.take() {
            Some(prev) => format!("{prev} ; {reason}"),
            None => reason,
        });
    } else if !patched_ok {
        let failed: Vec<String> = result
            .after
            .iter()
            .filter(|c| !c.ok)
            .map(|c| c.command.clone())
            .collect();
        let reason = format!("le patch casse la compatibilite : {}", failed.join(", "));
        result.reason = Some(match result.reason.take() {
            Some(prev) => format!("{prev} ; {reason}"),
            None => reason,
        });
    }

    result.compatible = result.signature_preserved
        && result.fallback_present
        && base_ok
        && patched_ok
        && result.reason.is_none();
    if result.compatible {
        result.reason = None;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_extracted_and_normalized() {
        let src = "fn  calculer ( a : i32 ) -> i32 {\n    a + 1\n}\n";
        let sig = signature_of(src, "calculer").unwrap();
        assert!(sig.contains("calculer"));
        assert!(sig.contains("-> i32"));
    }

    #[test]
    fn changed_signature_is_detected() {
        let a = "fn f(a: i32) -> i32 { a }\n";
        let b = "fn f(a: i32, b: i32) -> i32 { a + b }\n";
        assert_ne!(signature_of(a, "f"), signature_of(b, "f"));
    }

    #[test]
    fn fallback_markers_are_detected() {
        assert!(has_fallback(
            "#[cfg(target_arch = \"x86_64\")] fn fast() {}"
        ));
        assert!(!has_fallback(
            "fn fast() { unsafe { core::arch::x86_64::_mm_add } }"
        ));
    }
}
