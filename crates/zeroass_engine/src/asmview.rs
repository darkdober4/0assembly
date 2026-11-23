//! Émission de l'assembleur d'une fonction.
//!
//! Le but n'est pas d'écrire de l'assembleur à la place de l'humain, mais de
//! lui **montrer** ce que le compilateur produit pour la fonction visée :
//! c'est la référence contre laquelle on juge une réécriture en `asm!`.
//!
//! Méthode : on demande au compilateur d'émettre l'assembleur
//! (`cargo rustc -- --emit asm`, ou la commande `[asm]` du profil), puis on
//! extrait le bloc de la fonction dans le fichier `.s` produit.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};

use zeroass_core::profile::ProjectProfile;

/// Résultat de l'émission d'assembleur.
#[derive(Debug, Clone)]
pub struct AsmResult {
    /// Nom de la fonction demandée.
    pub symbol: String,
    /// Bloc d'assembleur extrait.
    pub assembly: String,
    /// Fichier `.s` complet (pour aller plus loin).
    pub file: Option<PathBuf>,
    /// Commande utilisée.
    pub command: String,
    /// Vrai si la fonction n'a pas été trouvée dans l'assembleur.
    pub missing: bool,
    /// Vrai si l'assembleur vient d'une compilation non optimisée (la
    /// version optimisée avait inliné la fonction).
    pub unoptimized: bool,
}

/// Commande d'émission pour le projet.
pub fn asm_command(profile: &ProjectProfile) -> String {
    if let Some(asm) = &profile.asm {
        if !asm.command.trim().is_empty() {
            return asm.command.clone();
        }
    }
    match profile.project.kind.as_str() {
        "rust" => "cargo rustc --release -- --emit asm".to_string(),
        "c" | "cpp" => "cargo build --release".to_string(),
        _ => String::new(),
    }
}

/// Émet l'assembleur et extrait le bloc de la fonction.
pub fn emit_assembly(profile: &ProjectProfile, project: &Path, fn_name: &str) -> Result<AsmResult> {
    let result = emit_once(profile, project, fn_name, "")?;
    if !result.missing {
        return Ok(result);
    }
    if profile.project.kind == "rust" {
        let mut anti_inline = emit_once(profile, project, fn_name, " -C opt-level=0")?;
        if !anti_inline.missing {
            anti_inline.unoptimized = true;
            return Ok(anti_inline);
        }
    }
    Ok(result)
}

/// Une passe d'émission.
fn emit_once(
    profile: &ProjectProfile,
    project: &Path,
    fn_name: &str,
    extra_args: &str,
) -> Result<AsmResult> {
    let base = asm_command(profile);
    if base.trim().is_empty() {
        anyhow::bail!(
            "aucune commande d'émission d'assembleur : renseigne [asm].command dans 0ass.toml"
        );
    }
    let command = format!("{base}{extra_args}");
    let spec = crate::runner::CommandSpec::new(command.clone(), project);
    let (code, output) = spec.run()?;
    if code != 0 {
        anyhow::bail!("l'émission d'assembleur a échoué : {output}");
    }

    // Le fichier .s le plus récent produit par la compilation.
    let asm_file = newest_asm_file(project);
    let (assembly, missing) = match &asm_file {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("lecture de {}", path.display()))?;
            let block = extract_function(&text, fn_name);
            let missing = block.is_none();
            (block.unwrap_or_default(), missing)
        }
        None => (String::new(), true),
    };

    Ok(AsmResult {
        symbol: fn_name.to_string(),
        assembly,
        file: asm_file,
        command,
        missing,
        unoptimized: false,
    })
}

/// Fichier `.s` le plus récent sous `target/`.
fn newest_asm_file(project: &Path) -> Option<PathBuf> {
    let deps = project.join("target").join("release").join("deps");
    let mut best: Option<(SystemTime, PathBuf)> = None;
    let Ok(rd) = std::fs::read_dir(&deps) else {
        return None;
    };
    for entry in rd.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("s") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if best.as_ref().is_none_or(|(t, _)| modified > *t) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, p)| p)
}

/// Extrait le bloc d'assembleur d'une fonction.
///
/// On cherche une étiquette contenant le nom de la fonction, puis on collecte
/// jusqu'à la directive `.size` qui en marque la fin.
pub fn extract_function(assembly: &str, fn_name: &str) -> Option<String> {
    let lines: Vec<&str> = assembly.lines().collect();
    let start = lines.iter().position(|l| {
        let t = l.trim();
        t.ends_with(':') && !t.starts_with('.') && t.contains(fn_name)
    })?;
    let mut block = Vec::new();
    for line in lines.iter().skip(start) {
        block.push(*line);
        let t = line.trim();
        if t.starts_with(".size") {
            break;
        }
        if block.len() > 400 {
            break;
        }
    }
    Some(block.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_function_block() {
        let asm = r#"
.text
_ZN4demo12somme_carres17h9e3a2b1c4d5e6f7g8hE:
    pushq	%rbp
    movq	%rsp, %rbp
    popq	%rbp
    retq
    .size	_ZN4demo12somme_carres17h9e3a2b1c4d5e6f7g8hE, 42
_ZN4demo3autre17h0000000000000000E:
    retq
"#;
        let block = extract_function(asm, "somme_carres").unwrap();
        assert!(block.contains("pushq"));
        assert!(block.contains(".size"));
        assert!(!block.contains("autre"));
    }

    #[test]
    fn missing_function_returns_none() {
        assert!(extract_function(".text\nfoo:\n retq\n", "barre").is_none());
    }
}
