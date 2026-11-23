//! `0ass` — moteur : exécution, mesure, campagnes, intégration Git.
//!
//! Le moteur ne modifie jamais le projet de l'utilisateur sans validation
//! humaine explicite, et ne pousse jamais rien.

pub mod asmview;
pub mod campaign;
pub mod git;
pub mod review;
pub mod runner;
pub mod stats;
pub mod verify;

pub use campaign::{Campaign, CampaignOptions};

/// Version du moteur.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
