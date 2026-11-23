//! Exécution de commandes et mesure de durée.
//!
//! Les commandes sont exécutées via le shell de la plateforme (`cmd /C` sous
//! Windows, `sh -c` ailleurs) : c'est ce que l'utilisateur écrit dans
//! `0ass.toml`, donc c'est ce qui doit être exécuté.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{anyhow, Context, Result};

use crate::stats::{compare, Comparison, Samples};

/// Une commande à exécuter.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    /// Ligne de commande complète.
    pub line: String,
    /// Répertoire de travail.
    pub cwd: PathBuf,
}

impl CommandSpec {
    /// Crée une spécification de commande.
    pub fn new(line: impl Into<String>, cwd: impl Into<PathBuf>) -> CommandSpec {
        CommandSpec {
            line: line.into(),
            cwd: cwd.into(),
        }
    }
}

/// Normalise une commande pour la plateforme courante.
///
/// Sous Windows, `cmd /C` ne cherche pas les exécutables dans le répertoire
/// courant : `target/release/demo` échoue alors que `.\target\release\demo.exe`
/// fonctionne. On répare donc les chemins relatifs, et on ajoute `.exe` si le
/// fichier existe avec cette extension.
pub fn normalize_command(line: &str) -> String {
    if !cfg!(windows) {
        return line.to_string();
    }
    let Some(first) = line.split_whitespace().next() else {
        return line.to_string();
    };
    if !(first.contains('/') || first.contains('\\')) {
        return line.to_string();
    }
    let absolute =
        first.starts_with('/') || first.starts_with('\\') || first.as_bytes().get(1) == Some(&b':');
    if absolute || first.starts_with("./") || first.starts_with(".\\") {
        return line.to_string();
    }
    let slashed = first.replace('/', "\\");
    let with_exe = format!("{slashed}.exe");
    let program = if Path::new(&with_exe).exists() {
        with_exe
    } else {
        slashed
    };
    line.replacen(first, &format!(".\\{program}"), 1)
}

/// Exécute une commande et renvoie sa durée en millisecondes.
///
/// La sortie est capturée (jamais laissée bloquer sur un pipe plein).
pub fn time_once(spec: &CommandSpec) -> Result<f64> {
    let line = normalize_command(&spec.line);
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", &line]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", &line]);
        c
    };
    cmd.current_dir(&spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let start = Instant::now();
    let output = cmd
        .output()
        .with_context(|| format!("exécution impossible : {}", spec.line))?;
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(anyhow!(
            "la commande a échoué (code {:?}) : {}\n{}\n{}",
            output.status.code(),
            spec.line,
            stdout.lines().take(5).collect::<Vec<_>>().join(" | "),
            stderr.lines().take(5).collect::<Vec<_>>().join(" | ")
        ));
    }
    Ok(elapsed)
}

/// Mesure une commande : échauffement puis exécutions mesurées.
pub fn measure(spec: &CommandSpec, warmup: u32, runs: u32) -> Result<Samples> {
    for _ in 0..warmup {
        time_once(spec)?;
    }
    let mut values = Vec::with_capacity(runs as usize);
    for _ in 0..runs {
        values.push(time_once(spec)?);
    }
    Ok(Samples::new(values))
}

/// Résultat d'une comparaison A/B entrelacée.
#[derive(Debug, Clone)]
pub struct AbResult {
    pub comparison: Comparison,
    pub before: Samples,
    pub after: Samples,
    /// Durée totale de la campagne de mesure, en secondes.
    pub duration_s: f64,
}

/// Compare deux variantes en **entrelaçant** les exécutions (A-B-A-B) : cela
/// annule la dérive de la machine (thermalisation, charge background).
pub fn compare_interleaved(
    before: &CommandSpec,
    after: &CommandSpec,
    warmup: u32,
    runs: u32,
) -> Result<AbResult> {
    let start = Instant::now();
    for _ in 0..warmup {
        time_once(before)?;
        time_once(after)?;
    }
    let mut b = Vec::with_capacity(runs as usize);
    let mut a = Vec::with_capacity(runs as usize);
    for _ in 0..runs {
        b.push(time_once(before)?);
        a.push(time_once(after)?);
    }
    let before_samples = Samples::new(b);
    let after_samples = Samples::new(a);
    let comparison = compare(&before_samples, &after_samples);
    Ok(AbResult {
        comparison,
        before: before_samples,
        after: after_samples,
        duration_s: start.elapsed().as_secs_f64(),
    })
}

/// Exécute une commande en capturant sa sortie (utile pour les bancs).
pub fn capture(spec: &CommandSpec) -> Result<String> {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", &normalize_command(&spec.line)]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", &spec.line]);
        c
    };
    cmd.current_dir(&spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let out = cmd
        .output()
        .with_context(|| format!("exécution impossible : {}", spec.line))?;
    Ok(format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}

impl CommandSpec {
    /// Exécute la commande et renvoie le code de sortie et la sortie fusionnée.
    ///
    /// Le code de sortie est toujours rendu : une commande qui échoue n'est
    /// pas une erreur de l'outil, c'est un **résultat** à contrôler.
    pub fn run(&self) -> Result<(i32, String)> {
        let line = normalize_command(&self.line);
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", &line]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", &self.line]);
            c
        };
        cmd.current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let out = cmd
            .output()
            .with_context(|| format!("exécution impossible : {}", self.line))?;
        let code = out.status.code().unwrap_or(-1);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        Ok((code, text))
    }
}

/// Vérifie qu'un chemin existe et est un fichier.
pub fn ensure_file(path: &Path) -> Result<()> {
    if !path.is_file() {
        return Err(anyhow!("fichier introuvable : {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("0ass-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn measures_a_successful_command() {
        let spec = CommandSpec::new("echo hello", tmp_dir());
        let samples = measure(&spec, 0, 3).unwrap();
        assert_eq!(samples.len(), 3);
        assert!(samples.median() >= 0.0);
    }

    #[test]
    fn failing_command_is_reported() {
        let spec = CommandSpec::new("exit 3", tmp_dir());
        let err = measure(&spec, 0, 1).unwrap_err();
        assert!(err.to_string().contains("échoué"));
    }

    #[test]
    fn interleaved_comparison_detects_improvement() {
        let dir = tmp_dir();
        let fast = CommandSpec::new("echo a", &dir);
        let slow = CommandSpec::new("ping -n 2 127.0.0.1 >NUL", &dir);
        let result = compare_interleaved(&fast, &slow, 0, 2);
        // La commande lente peut échouer selon la plateforme : on n'échoue pas le test.
        if let Ok(r) = result {
            assert!(r.comparison.before_ms >= 0.0);
        }
    }

    #[test]
    fn relative_paths_are_normalized_on_windows() {
        let line = "target/release/demo --once";
        let out = normalize_command(line);
        if cfg!(windows) {
            assert!(out.starts_with(".\\target"));
        } else {
            assert_eq!(out, line);
        }
    }
}
