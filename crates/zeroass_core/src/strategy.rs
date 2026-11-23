//! Catalogue des stratégies d'optimisation.
//!
//! Chaque stratégie décrit **comment** réécrire une fonction, pour quelles cibles,
//! et avec quel niveau de risque. Le moteur ne.code pas : il s'appuie sur ce
//! catalogue pour proposer une piste et pour documenter le risque.

use serde::{Deserialize, Serialize};

use crate::scan::SignalMetrics;

/// Niveau de risque d'une stratégie (undefined behavior).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// Aucune différence observable, code sûr.
    Low,
    /// Différences possibles (overflow, aliasing).
    Medium,
    /// Comportement indéfini possible.
    High,
}

/// Cibles couvertes par une stratégie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetSupport {
    /// Même source pour toutes les architectures.
    Portable,
    /// Implémentation spécifique par architecture.
    PerArch,
}

/// Une stratégie du catalogue (données statiques).
#[derive(Debug, Clone)]
pub struct Strategy {
    /// Identifiant court.
    pub id: &'static str,
    /// Nom lisible.
    pub name: &'static str,
    /// Description de l'approche.
    pub description: &'static str,
    /// Couverture des architectures.
    pub support: TargetSupport,
    /// Risque d'undefined behavior.
    pub risk: RiskLevel,
    /// Gains typiques observés (ordre de grandeur, pas une garantie).
    pub typical_gain: &'static str,
    /// Langages concernés (vide = tous).
    pub languages: &'static [&'static str],
}

/// Vue serialisable d'une stratégie (manifeste, interface, rapport).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub support: String,
    pub risk: String,
    pub typical_gain: String,
    pub languages: Vec<String>,
}

impl From<&Strategy> for StrategyView {
    fn from(s: &Strategy) -> Self {
        StrategyView {
            id: s.id.to_string(),
            name: s.name.to_string(),
            description: s.description.to_string(),
            support: match s.support {
                TargetSupport::Portable => "portable".into(),
                TargetSupport::PerArch => "per_arch".into(),
            },
            risk: match s.risk {
                RiskLevel::Low => "low".into(),
                RiskLevel::Medium => "medium".into(),
                RiskLevel::High => "high".into(),
            },
            typical_gain: s.typical_gain.to_string(),
            languages: s.languages.iter().map(|l| l.to_string()).collect(),
        }
    }
}

/// Catalogue complet.
pub static STRATEGIES: &[Strategy] = &[
    Strategy {
        id: "bounds_elimination",
        name: "Élimination des bounds-checks",
        description: "Remplacer l'indexation directe par des itérateurs ou une forme dont \
                      le compilateur peut prouver l'absence de vérification.",
        support: TargetSupport::Portable,
        risk: RiskLevel::Low,
        typical_gain: "5 % – 35 %",
        languages: &[],
    },
    Strategy {
        id: "branchless",
        name: "Code sans branchement",
        description: "Remplacer un if/else par une opération arithmétique (min/max, \
                      sélection inconditionnelle) quand les deux branches sont triviales.",
        support: TargetSupport::Portable,
        risk: RiskLevel::Low,
        typical_gain: "2 % – 20 %",
        languages: &[],
    },
    Strategy {
        id: "alloc_hoist",
        name: "Hoisting des allocations",
        description: "Réutiliser le tampon alloué hors de la boucle au lieu de le recréer \
                      à chaque itération.",
        support: TargetSupport::Portable,
        risk: RiskLevel::Low,
        typical_gain: "5 % – 40 %",
        languages: &[],
    },
    Strategy {
        id: "simd_portable",
        name: "SIMD portable",
        description: "Vectoriser la boucle via une abstraction SIMD portable (`std::simd` en \
                      Rust) : même source pour x86-64 et aarch64, repli scalaire garanti. \
                      La signature de la fonction est conservée à l'identique.",
        support: TargetSupport::Portable,
        risk: RiskLevel::Low,
        typical_gain: "20 % – 300 %",
        languages: &["rust"],
    },
    Strategy {
        id: "intrinsics_simd",
        name: "Intrinsiques SIMD",
        description: "Vectorisation explicite par architecture (`core::arch`), avec détection \
                      des capacités du processeur à l'exécution et **repli scalaire obligatoire** \
                      : sans le jeu d'instructions, le code portable prend le relais.",
        support: TargetSupport::PerArch,
        risk: RiskLevel::Medium,
        typical_gain: "30 % – 400 %",
        languages: &["rust", "c", "cpp"],
    },
    Strategy {
        id: "asm_inline",
        name: "Assembly inline",
        description: "Bloc `asm!` inline pour une instruction que le compilateur n'exprime pas. \
                      Réservé aux cas mesurés. **Règle de compatibilité** : la fonction doit \
                      rester callable et correcte sans ce bloc (repli portable obligatoire), et \
                      sa signature ne doit pas changer.",
        support: TargetSupport::PerArch,
        risk: RiskLevel::High,
        typical_gain: "variable",
        languages: &["rust", "c", "cpp"],
    },
    Strategy {
        id: "measure_first",
        name: "Mesurer avant de réécrire",
        description: "Le signal statique ne suffit pas : un profilage ciblé dira si la                       fonction est réellement chaude, et quelle partie domine.",
        support: TargetSupport::Portable,
        risk: RiskLevel::Low,
        typical_gain: "variable",
        languages: &[],
    },
    Strategy {
        id: "no_change",
        name: "Ne rien changer",
        description: "Le compilateur produit déjà un code optimal ou le gain potentiel ne \
                      justifie pas la maintenance.",
        support: TargetSupport::Portable,
        risk: RiskLevel::Low,
        typical_gain: "0 %",
        languages: &[],
    },
];

/// Recherche une stratégie par identifiant.
pub fn by_id(id: &str) -> Option<&'static Strategy> {
    STRATEGIES.iter().find(|s| s.id == id)
}

/// Stratégies applicables à un langage.
pub fn for_language(lang: &str) -> Vec<&'static Strategy> {
    STRATEGIES
        .iter()
        .filter(|s| s.languages.is_empty() || s.languages.contains(&lang))
        .collect()
}

/// Propose une stratégie à partir des signaux détectés par le scan.
///
/// Le choix est volontairement conservateur : on part de la stratégie la plus
/// sûre qui répond au signal principal, et on n'atteint jamais l'assembleur
/// sans mesure. Le SIMD n'est proposé que si la boucle **accumule des
/// nombres** ou fait un **calcul élémentaire** sur **au moins deux**
/// éléments indexés : c'est le minimum pour qu'une vectorisation ait un sens.
pub fn propose(metrics: &SignalMetrics, lang: &str) -> (&'static Strategy, String) {
    let m = metrics;

    // On note chaque stratégie d'après les signaux et on retient la meilleure.
    // À égalité, la plus sûre l'emporte (l'ordre du catalogue va du plus sûr
    // au plus risqué).
    let mut choices: Vec<(&'static Strategy, f64, String)> = Vec::new();

    let vectorisable = m.numeric_accum > 0 || (m.arith_ops >= 2 && m.indexed >= 2);

    if m.allocs > 0 {
        choices.push((
            by_id("alloc_hoist").unwrap(),
            3.0,
            "une allocation est créée dans la boucle : la déplacer hors de la boucle              supprime les allocations répétées sans changer la sémantique."
                .into(),
        ));
    }
    if m.indexed > 0 {
        choices.push((
            by_id("bounds_elimination").unwrap(),
            3.0,
            "la boucle indexe une collection : la forme itérative (ou une borne              prouvée) supprime les vérifications d'index à chaque tour."
                .into(),
        ));
    }
    if vectorisable && lang == "rust" {
        choices.push((
            by_id("simd_portable").unwrap(),
            4.5,
            "la boucle accumule ou calcule élément par élément sur des données indexées :              un accumulateur unique, éventuellement vectorisable (`std::simd`), remplace              les copies répétées."
                .into(),
        ));
    }
    if m.collection_build > 0 && lang != "rust" {
        choices.push((
            by_id("alloc_hoist").unwrap(),
            3.5,
            "la boucle construit une collection : un tampon réutilisé remplace les              allocations et copies répétées."
                .into(),
        ));
    }
    if m.hot > 0 {
        choices.push((
            by_id("measure_first").unwrap(),
            2.0,
            "la boucle appelle des routines sensibles (longueur, recherche, copie) :              ces appels sont de bons candidats à la vectorisation, mais il faut d'abord              mesurer la fonction pour savoir si elle pèse vraiment."
                .into(),
        ));
    }
    if m.loops > 1 {
        choices.push((
            by_id("measure_first").unwrap(),
            1.5,
            "boucles imbriquées : la profondeur d'imbrication est plus coûteuse que              l'indexation ; mesurer avant de réécrire."
                .into(),
        ));
    }

    choices.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    match choices.into_iter().next() {
        Some((s, _, reason)) => (s, reason),
        None => (
            by_id("no_change").unwrap(),
            "aucun signal de performance exploitable n'a été détecté.".into(),
        ),
    }
}

/// Vue serialisable d'une stratégie par identifiant.
pub fn view(id: &str) -> Option<StrategyView> {
    by_id(id).map(StrategyView::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_lookup() {
        assert!(by_id("simd_portable").is_some());
        assert!(by_id("inexistant").is_none());
    }

    #[test]
    fn per_arch_strategies_are_flagged() {
        let asm = by_id("asm_inline").unwrap();
        assert_eq!(asm.support, TargetSupport::PerArch);
        assert_eq!(asm.risk, RiskLevel::High);
    }

    #[test]
    fn language_filtering() {
        let rust = for_language("rust");
        assert!(rust.iter().any(|s| s.id == "simd_portable"));
        let python = for_language("python");
        assert!(!python.iter().any(|s| s.id == "simd_portable"));
    }
}
