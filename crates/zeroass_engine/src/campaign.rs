//! Campagne : scan → mesure → manifeste → rapport.
//!
//! Une campagne ne modifie **jamais** le projet analysé. Elle produit un dossier
//! de rapport hors dépôt, contenant le manifeste JSON et le résumé Markdown.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use zeroass_core::manifest::{BenchReport, Candidate, Manifest, ManifestProject, Risk, TargetInfo};
use zeroass_core::profile::ProjectProfile;
use zeroass_core::scan::{self, ScanHit, ScanOptions};
use zeroass_core::strategy;
use zeroass_core::summary;

use crate::runner::{self, CommandSpec};

/// Options de campagne.
#[derive(Debug, Clone)]
pub struct CampaignOptions {
    /// Racine du projet analysé.
    pub project_root: PathBuf,
    /// Dossier de rapport (défaut : `%LOCALAPPDATA%/0ass/reports/<projet>`).
    pub report_root: Option<PathBuf>,
    /// Ne pas exécuter le banc (scan statique seul).
    pub skip_bench: bool,
}

impl CampaignOptions {
    pub fn new(project_root: impl Into<PathBuf>) -> CampaignOptions {
        CampaignOptions {
            project_root: project_root.into(),
            report_root: None,
            skip_bench: false,
        }
    }
}

/// Résultat d'une campagne.
#[derive(Debug, Clone)]
pub struct Campaign {
    pub manifest: Manifest,
    pub report_dir: PathBuf,
    pub summary_md: String,
}

/// Dossier de rapport par défaut.
pub fn default_report_root(project_root: &Path) -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir);
    let slug = project_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "projet".into());
    base.join("0ass").join("reports").join(slug)
}

/// Exécute une campagne complète.
pub fn run(options: &CampaignOptions) -> Result<Campaign> {
    let root = &options.project_root;
    if !root.is_dir() {
        anyhow::bail!("projet introuvable : {}", root.display());
    }
    let profile = ProjectProfile::load_or_default(root);

    // 1. Scan statique multi-langages.
    let scan_options = ScanOptions {
        roots: profile.roots(root),
        excludes: scan::DEFAULT_EXCLUDES
            .iter()
            .map(|s| s.to_string())
            .collect(),
        max_hits: 500,
    };
    let hits = scan::scan_project(root, &scan_options);
    let total_score: f64 = hits.iter().map(|h| h.score).sum();
    let candidates = build_candidates(&hits, total_score);

    // 2. Mesure du banc global (si le projet est mesurable).
    let bench = if options.skip_bench || !profile.is_measurable() {
        None
    } else {
        measure_bench(&profile, root).ok()
    };

    // 3. Manifeste.
    let manifest = Manifest::new(
        ManifestProject {
            root: root.to_string_lossy().to_string(),
            name: profile.project.name.clone(),
            kind: profile.project.kind.clone(),
            profile: if root.join("0ass.toml").exists() {
                Some(root.join("0ass.toml").to_string_lossy().to_string())
            } else {
                None
            },
        },
        TargetInfo {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            rustc: rustc_version(),
            profile: "release".into(),
        },
    );
    let mut manifest = manifest;
    manifest.bench = bench;
    manifest.candidates = candidates;

    // 4. Écriture du rapport.
    let report_root = options
        .report_root
        .clone()
        .unwrap_or_else(|| default_report_root(root));
    let report_dir = next_campaign_dir(&report_root)?;
    std::fs::create_dir_all(&report_dir)
        .with_context(|| format!("création du dossier de rapport {}", report_dir.display()))?;
    std::fs::create_dir_all(report_dir.join("patches"))?;
    std::fs::create_dir_all(report_dir.join("logs"))?;

    let summary_md = summary::summary(&manifest);
    std::fs::write(report_dir.join("manifest.json"), manifest.to_json())?;
    std::fs::write(report_dir.join("summary.md"), &summary_md)?;

    Ok(Campaign {
        manifest,
        report_dir,
        summary_md,
    })
}

/// Construit les candidats à partir des résultats du scan.
fn build_candidates(hits: &[ScanHit], total_score: f64) -> Vec<Candidate> {
    hits.iter()
        .enumerate()
        .map(|(idx, h)| {
            let (strat, reason) = strategy::propose(&h.metrics, &h.lang);
            let share = if total_score > 0.0 {
                h.score / total_score * 100.0
            } else {
                0.0
            };
            let targets = match strat.support {
                strategy::TargetSupport::Portable => vec!["portable (x86_64 + aarch64)".into()],
                strategy::TargetSupport::PerArch => {
                    vec!["x86_64".into(), "aarch64".into(), "repli scalaire".into()]
                }
            };
            Candidate {
                id: format!("A{}", idx + 1),
                file: h.rel_path.clone(),
                fn_name: h.fn_name.clone(),
                lang: h.lang.clone(),
                start_line: h.start_line,
                end_line: h.end_line,
                runtime_share_pct: None,
                hotness_estimate: (share * 10.0).min(100.0),
                detected: h.signals.clone(),
                strategy: strat.id.to_string(),
                strategy_reason: reason,
                targets,
                measurement: None,
                risk: Risk {
                    ub: match strat.risk {
                        strategy::RiskLevel::Low => "low".into(),
                        strategy::RiskLevel::Medium => "medium".into(),
                        strategy::RiskLevel::High => "high".into(),
                    },
                    portability: match strat.support {
                        strategy::TargetSupport::Portable => "portable".into(),
                        strategy::TargetSupport::PerArch => "per_arch".into(),
                    },
                    review_required: true,
                },
                compatibility: zeroass_core::manifest::Compatibility::default(),
                status: zeroass_core::manifest::Status::Review,
                patch: None,
                commit: None,
                confidence: h.confidence,
            }
        })
        .collect()
}

/// Mesure le banc global du projet.
fn measure_bench(profile: &ProjectProfile, root: &Path) -> Result<BenchReport> {
    let command = if !profile.run.command.trim().is_empty() {
        profile.run.command.clone()
    } else {
        profile
            .bench
            .as_ref()
            .map(|b| b.command.clone())
            .unwrap_or_default()
    };
    if command.trim().is_empty() {
        anyhow::bail!("aucune commande de mesure");
    }
    let spec = CommandSpec::new(command, root);
    let samples = runner::measure(&spec, profile.run.warmup, profile.run.runs)?;
    let mad_pct = samples.mad_pct();
    Ok(BenchReport {
        level: if profile.bench.is_some() {
            "micro".into()
        } else {
            "blackbox".into()
        },
        tool: if profile.bench.is_some() {
            "projet".into()
        } else {
            "wallclock".into()
        },
        runs: profile.run.runs,
        warmup: profile.run.warmup,
        median_ms: samples.median(),
        mad_pct,
        mde_pct: samples.mde_pct(),
        reliable: mad_pct <= 5.0,
        duration_s: 0.0,
    })
}

/// Version de rustc, si elle est accessible.
pub fn rustc_version() -> Option<String> {
    let out = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let version = text.split_whitespace().nth(1)?;
    Some(version.to_string())
}

/// Dossier de campagne incrémental (`2026-11-09-run1`, `run2`, …).
pub fn next_campaign_dir(report_root: &Path) -> Result<PathBuf> {
    let date = zeroass_core::today_compact();
    if !report_root.exists() {
        return Ok(report_root.join(format!("{date}-run1")));
    }
    let mut max = 0usize;
    for entry in std::fs::read_dir(report_root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(rest) = name.strip_prefix(&format!("{date}-run")) {
            if let Ok(n) = rest.parse::<usize>() {
                max = max.max(n);
            }
        }
    }
    Ok(report_root.join(format!("{date}-run{}", max + 1)))
}

/// Trouve le dernier rapport d'un projet.
pub fn latest_report(report_root: &Path) -> Result<PathBuf> {
    let mut best: Option<(String, PathBuf)> = None;
    if !report_root.exists() {
        anyhow::bail!("aucun rapport dans {}", report_root.display());
    }
    for entry in std::fs::read_dir(report_root)? {
        let entry = entry?;
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.contains("-run") {
            continue;
        }
        let manifest = entry.path().join("manifest.json");
        if !manifest.exists() {
            continue;
        }
        let stamp = manifest
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .map(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let key = format!("{stamp:020}");
        if best.as_ref().map(|(k, _)| key > *k).unwrap_or(true) {
            best = Some((key, entry.path()));
        }
    }
    match best {
        Some((_, p)) => Ok(p),
        None => anyhow::bail!("aucun rapport dans {}", report_root.display()),
    }
}

/// Dossier du worktree d'une campagne (hors dépôt, dans le rapport).
pub fn worktree_dir(report_dir: &Path) -> PathBuf {
    report_dir.join("worktree")
}

/// Charge le manifeste d'un dossier de rapport.
pub fn load_manifest(report_dir: &Path) -> Result<Manifest> {
    let path = report_dir.join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("lecture du manifeste {}", path.display()))?;
    let manifest: Manifest = serde_json::from_str(&text)
        .with_context(|| format!("analyse du manifeste {}", path.display()))?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn campaign_dirs_are_incremental() {
        let base = std::env::temp_dir().join(format!("0ass-reports-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let first = next_campaign_dir(&base).unwrap();
        std::fs::create_dir_all(&first).unwrap();
        let second = next_campaign_dir(&base).unwrap();
        assert_ne!(first, second);
        assert!(second.to_string_lossy().contains("-run2"));
    }

    #[test]
    fn candidates_are_built_with_ids_and_strategy() {
        let hits = vec![ScanHit {
            rel_path: "src/a.rs".into(),
            lang: "rust".into(),
            fn_name: "f".into(),
            start_line: 1,
            end_line: 9,
            lines: 9,
            score: 5.0,
            confidence: zeroass_core::scan::Confidence::Medium,
            signals: vec![
                "boucle x2".into(),
                "allocation dans une boucle (x1, probable)".into(),
            ],
            metrics: zeroass_core::scan::SignalMetrics {
                loops: 2,
                allocs: 1,
                ..Default::default()
            },
        }];
        let candidates = build_candidates(&hits, 5.0);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, "A1");
        assert_eq!(candidates[0].strategy, "alloc_hoist");
        assert!(candidates[0].risk.review_required);
    }
}
