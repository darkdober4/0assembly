//! Manifeste d'une campagne : le format d'échange entre le moteur, l'interface
//! graphique et l'agent (qui ne fait que le lire).

use serde::{Deserialize, Serialize};

use crate::scan::Confidence;

/// Identifiant de l'outil et version du format.
pub const TOOL: &str = "0ass";
pub const FORMAT_VERSION: u32 = 1;

/// Statut d'un candidat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// En attente de review humaine.
    Review,
    /// Approuvé, pas encore commité.
    Approved,
    /// Commité.
    Applied,
    /// Rejeté (par l'humain ou par la mesure).
    Rejected,
    /// Mesure obsolète (le code a changé depuis).
    Stale,
}

/// Niveau de risque d'un candidat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Risk {
    /// Risque d'undefined behavior : `low`, `medium`, `high`.
    pub ub: String,
    /// Portabilité : `portable`, `x86_64`, `aarch64`.
    pub portability: String,
    /// Toujours vrai : la review humaine est obligatoire.
    pub review_required: bool,
}

/// Résultat d'une commande de vérification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    /// Commande exécutée.
    pub command: String,
    /// Code de sortie (`0` = succès).
    pub exit_code: i32,
    /// Durée en millisecondes.
    pub duration_ms: u64,
    /// Sortie (dernières lignes).
    pub tail: String,
    /// La commande a-t-elle réussi ?
    pub ok: bool,
}

/// Vérification de compatibilité d'un patch.
///
/// Règle du projet : **le code réécrit doit rester compatible avec le reste du
/// programme et le programme doit continuer de tourner**. Concrètement :
///
/// 1. la signature de la fonction est inchangée (le reste du code compile) ;
/// 2. un repli portable existe (le binaire tourne même si le chemin optimisé
///    n'est pas pris : CPU sans AVX, architecture différente, build portable) ;
/// 3. la suite de tests passe **après** le patch comme **avant**.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Compatibility {
    /// Signature d'origine conservée à l'identique.
    pub signature_preserved: bool,
    /// Repli portable présent dans le patch.
    pub fallback_present: bool,
    /// Vérifications exécutées avant le patch.
    pub before: Vec<CommandResult>,
    /// Vérifications exécutées après le patch.
    pub after: Vec<CommandResult>,
    /// Le patch est-il compatible (tout est vert) ?
    pub compatible: bool,
    /// Motif du refus, si refus.
    pub reason: Option<String>,
}

impl Default for Compatibility {
    fn default() -> Self {
        Self {
            signature_preserved: true,
            fallback_present: false,
            before: Vec::new(),
            after: Vec::new(),
            compatible: false,
            reason: Some("compatibilité non vérifiée".into()),
        }
    }
}

/// Mesure A/B d'un candidat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measurement {
    /// Gain local mesuré, en pourcentage.
    pub local_gain_pct: f64,
    /// Impact global estimé sur le runtime, en pourcentage.
    pub global_gain_pct: f64,
    /// Temps médian avant, en millisecondes.
    pub before_ms: f64,
    /// Temps médian après, en millisecondes.
    pub after_ms: f64,
    /// Nombre d'exécutions.
    pub n: u32,
    /// Dispersion relative (MAD / médiane), en pourcentage.
    pub mad_pct: f64,
    /// Effet minimum détectable, en pourcentage.
    pub mde_pct: f64,
    /// Régressions détectées ailleurs (vide = aucune).
    pub regressions: Vec<String>,
}

/// Candidat à optimiser.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    /// Identifiant stable (`A1`, `A2`…).
    pub id: String,
    /// Chemin relatif du fichier.
    pub file: String,
    /// Nom de la fonction.
    pub fn_name: String,
    /// Langage.
    pub lang: String,
    /// Ligne de début.
    pub start_line: usize,
    /// Ligne de fin.
    pub end_line: usize,
    /// Part du runtime mesurée (profilage) : `None` si non mesurée.
    pub runtime_share_pct: Option<f64>,
    /// Estimation statique de « chaleur » (0–100).
    pub hotness_estimate: f64,
    /// Signaux détectés par le scan.
    pub detected: Vec<String>,
    /// Stratégie proposée.
    pub strategy: String,
    /// Justification de la stratégie.
    pub strategy_reason: String,
    /// Couverture par architecture.
    pub targets: Vec<String>,
    /// Mesure A/B, si elle a été réalisée.
    pub measurement: Option<Measurement>,
    /// Risque associé.
    pub risk: Risk,
    /// Vérification de compatibilité (signature, repli, tests).
    pub compatibility: Compatibility,
    /// Statut courant.
    pub status: Status,
    /// Fichier de patch associé (chemin relatif au rapport).
    pub patch: Option<String>,
    /// SHA du commit une fois appliqué.
    pub commit: Option<String>,
    /// Confiance du diagnostic statique.
    pub confidence: Confidence,
}

/// Rapport de mesure du banc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchReport {
    /// Niveau de mesure : `micro` ou `blackbox`.
    pub level: String,
    /// Outil de mesure (`criterion`, `wallclock`).
    pub tool: String,
    /// Nombre d'exécutions mesurées.
    pub runs: u32,
    /// Nombre d'exécutions d'échauffement.
    pub warmup: u32,
    /// Médiane, en millisecondes.
    pub median_ms: f64,
    /// Dispersion relative, en pourcentage.
    pub mad_pct: f64,
    /// Effet minimum détectable, en pourcentage.
    pub mde_pct: f64,
    /// La mesure est-elle fiable (bruit sous le seuil) ?
    pub reliable: bool,
    /// Durée totale du banc, en secondes.
    pub duration_s: f64,
}

/// Informations de compilation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetInfo {
    pub os: String,
    pub arch: String,
    pub rustc: Option<String>,
    pub profile: String,
}

/// Informations de projet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestProject {
    pub root: String,
    pub name: String,
    pub kind: String,
    pub profile: Option<String>,
}

/// Manifeste complet d'une campagne.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub tool: String,
    pub version: u32,
    pub project: ManifestProject,
    pub generated_at: String,
    pub target: TargetInfo,
    /// Rapport de mesure du banc global (optionnel).
    pub bench: Option<BenchReport>,
    /// Candidats retenus.
    pub candidates: Vec<Candidate>,
    /// Candidats écartés (avec la raison).
    pub rejected: Vec<RejectedCandidate>,
}

/// Candidat écarté.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectedCandidate {
    pub id: String,
    pub file: String,
    pub fn_name: String,
    /// Motif de l'écart.
    pub reason: String,
}

impl Manifest {
    /// Crée un manifeste vide pour un projet.
    pub fn new(project: ManifestProject, target: TargetInfo) -> Manifest {
        Manifest {
            tool: TOOL.to_string(),
            version: FORMAT_VERSION,
            project,
            generated_at: crate::now_iso8601(),
            target,
            bench: None,
            candidates: Vec::new(),
            rejected: Vec::new(),
        }
    }

    /// Sérialise le manifeste en JSON indenté.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }

    /// Prochain identifiant libre (`A1`, `A2`…).
    pub fn next_id(&self) -> String {
        let mut max = 0usize;
        for c in &self.candidates {
            if let Some(n) = c.id.strip_prefix('A').and_then(|d| d.parse::<usize>().ok()) {
                max = max.max(n);
            }
        }
        for c in &self.rejected {
            if let Some(n) = c.id.strip_prefix('A').and_then(|d| d.parse::<usize>().ok()) {
                max = max.max(n);
            }
        }
        format!("A{}", max + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        let mut m = Manifest::new(
            ManifestProject {
                root: "D:/demo".into(),
                name: "demo".into(),
                kind: "rust".into(),
                profile: None,
            },
            TargetInfo {
                os: "windows".into(),
                arch: "x86_64".into(),
                rustc: Some("1.99.0".into()),
                profile: "release".into(),
            },
        );
        m.candidates.push(Candidate {
            id: "A1".into(),
            file: "src/a.rs".into(),
            fn_name: "f".into(),
            lang: "rust".into(),
            start_line: 1,
            end_line: 9,
            runtime_share_pct: Some(3.2),
            hotness_estimate: 42.0,
            detected: vec!["boucle x2".into()],
            strategy: "bounds_elimination".into(),
            strategy_reason: "indexation vérifiée".into(),
            targets: vec!["portable".into()],
            measurement: None,
            risk: Risk {
                ub: "low".into(),
                portability: "portable".into(),
                review_required: true,
            },
            compatibility: Compatibility::default(),
            status: Status::Review,
            patch: None,
            commit: None,
            confidence: Confidence::Medium,
        });
        m
    }

    #[test]
    fn roundtrip_json() {
        let m = sample();
        let json = m.to_json();
        let back: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.candidates.len(), 1);
        assert_eq!(back.candidates[0].id, "A1");
        assert_eq!(back.tool, "0ass");
    }

    #[test]
    fn next_id_increments() {
        let mut m = sample();
        assert_eq!(m.next_id(), "A2");
        m.rejected.push(RejectedCandidate {
            id: "A2".into(),
            file: "b".into(),
            fn_name: "g".into(),
            reason: "gain nul".into(),
        });
        assert_eq!(m.next_id(), "A3");
    }
}
