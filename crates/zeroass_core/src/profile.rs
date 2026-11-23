//! Profil de projet : le fichier `0ass.toml`.
//!
//! C'est ce qui permet à `0ass` de travailler avec **n'importe quel programme** :
//! `0ass` ne connaît pas tous les systèmes de build, il sait lire un profil.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Contenu de `0ass.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectProfile {
    #[serde(default)]
    pub project: ProjectInfo,
    #[serde(default)]
    pub build: BuildInfo,
    #[serde(default)]
    pub run: RunInfo,
    #[serde(default)]
    pub bench: Option<BenchInfo>,
    #[serde(default)]
    pub verify: VerifyInfo,
    #[serde(default)]
    pub asm: Option<AsmInfo>,
    #[serde(default)]
    pub profile: PlatformProfiles,
}

/// Commande d'émission de l'assembleur (pour voir ce que produit le
/// compilateur pour la fonction visée).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AsmInfo {
    /// Commande qui produit un fichier d'assembleur (`.s`).
    #[serde(default)]
    pub command: String,
}

/// Commandes de vérification exécutées avant et après chaque patch.
///
/// Ellesfallen garants de la règle « le code changé doit rester compatible avec
/// le reste du programme et le programme doit continuer de tourner » :
/// la compilation vérifie l'interface, les tests vérifient le comportement.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerifyInfo {
    /// Commandes de compilation / vérification d'interface.
    #[serde(default)]
    pub check: Vec<String>,
    /// Commandes de test du projet.
    #[serde(default)]
    pub test: Vec<String>,
}

/// Informations générales.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    /// Nom du projet (défaut : nom du dossier).
    #[serde(default)]
    pub name: String,
    /// Nature du projet : `rust`, `generic`, `c`, `go`, `python`…
    #[serde(default)]
    pub kind: String,
    /// Racines de code à analyser.
    #[serde(default = "default_roots")]
    pub source_roots: Vec<String>,
}

impl Default for ProjectInfo {
    fn default() -> Self {
        ProjectInfo {
            name: String::new(),
            kind: String::new(),
            source_roots: default_roots(),
        }
    }
}

/// Commande de construction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildInfo {
    /// Commande de construction (ex. `cargo build --release`).
    #[serde(default)]
    pub command: String,
    /// Binaire produit.
    #[serde(default)]
    pub binary: String,
}

/// Charge de travail chronométrée.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunInfo {
    /// Commande de la charge de travail.
    #[serde(default)]
    pub command: String,
    /// Nombre d'exécutions d'échauffement (jetées).
    #[serde(default = "default_warmup")]
    pub warmup: u32,
    /// Nombre d'exécutions mesurées.
    #[serde(default = "default_runs")]
    pub runs: u32,
}

/// Banc de mesure dédié (prioritaire sur la mesure black-box).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchInfo {
    /// Commande du banc.
    pub command: String,
}

/// Commandes de profilage par plateforme.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlatformProfiles {
    #[serde(default)]
    pub linux: Option<String>,
    #[serde(default)]
    pub macos: Option<String>,
    #[serde(default)]
    pub windows: Option<String>,
}

fn default_roots() -> Vec<String> {
    vec![".".to_string()]
}
fn default_warmup() -> u32 {
    3
}
fn default_runs() -> u32 {
    30
}

/// Erreur de lecture/parsing du profil.
#[derive(Debug)]
pub struct ProfileError(pub String);

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "profil invalide : {}", self.0)
    }
}
impl std::error::Error for ProfileError {}

impl ProjectProfile {
    /// Charge un profil depuis `0ass.toml` dans `root`.
    pub fn load(root: &Path) -> Result<ProjectProfile, ProfileError> {
        let path = root.join("0ass.toml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ProfileError(format!("{} : {e}", path.display())))?;
        let mut profile: ProjectProfile =
            toml::from_str(&text).map_err(|e| ProfileError(format!("{} : {e}", path.display())))?;
        if profile.project.name.is_empty() {
            profile.project.name = root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "projet".into());
        }
        if profile.project.kind.is_empty() {
            profile.project.kind = "generic".into();
        }
        Ok(profile)
    }

    /// Charge un profil ou renvoie un profil par défaut (projet sans mesure).
    pub fn load_or_default(root: &Path) -> ProjectProfile {
        ProjectProfile::load(root).unwrap_or_default()
    }

    /// Racines absolues à analyser.
    pub fn roots(&self, root: &Path) -> Vec<PathBuf> {
        if self.project.source_roots.is_empty() {
            return vec![root.to_path_buf()];
        }
        self.project
            .source_roots
            .iter()
            .map(|r| root.join(r))
            .collect()
    }

    /// Commande de profilage adaptée à la plateforme courante.
    pub fn profile_command(&self) -> Option<&str> {
        let cmd = if cfg!(target_os = "linux") {
            self.profile.linux.as_deref()
        } else if cfg!(target_os = "macos") {
            self.profile.macos.as_deref()
        } else if cfg!(target_os = "windows") {
            self.profile.windows.as_deref()
        } else {
            None
        };
        cmd.or(self.bench.as_ref().map(|b| b.command.as_str()))
    }

    /// Le projet peut-il être mesuré ?
    pub fn is_measurable(&self) -> bool {
        !self.run.command.trim().is_empty() || self.bench.is_some()
    }

    /// Le projet possède-t-il des vérifications de compatibilité ?
    pub fn has_verification(&self) -> bool {
        !self.verify.check.is_empty() || !self.verify.test.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_minimal_profile() {
        let toml = r#"
[project]
name = "demo"
kind = "rust"
source_roots = ["src"]

[build]
command = "cargo build --release"

[run]
command = "./target/release/demo --once"
runs = 12
"#;
        let p: ProjectProfile = toml::from_str(toml).unwrap();
        assert_eq!(p.project.name, "demo");
        assert_eq!(p.project.source_roots, vec!["src".to_string()]);
        assert_eq!(p.run.runs, 12);
        assert_eq!(p.run.warmup, 3);
        assert!(p.is_measurable());
    }

    #[test]
    fn defaults_are_safe() {
        let p: ProjectProfile = toml::from_str("").unwrap();
        assert_eq!(p.project.source_roots, vec![".".to_string()]);
        assert!(!p.is_measurable());
        assert!(!p.has_verification());
    }

    #[test]
    fn verification_commands_are_read() {
        let toml = r#"
[verify]
check = ["cargo check --all-targets"]
test = ["cargo test"]
"#;
        let p: ProjectProfile = toml::from_str(toml).unwrap();
        assert_eq!(p.verify.check.len(), 1);
        assert_eq!(p.verify.test, vec!["cargo test".to_string()]);
        assert!(p.has_verification());
    }
}
