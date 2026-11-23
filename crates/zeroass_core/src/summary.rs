//! Génération du résumé Markdown d'une campagne.
//!
//! Le `.md` est le document destiné à l'humain : il liste les changements
//! proposés, les mesures, les risques et les écarts.

use std::fmt::Write as _;

use crate::manifest::{Manifest, Status};

/// Génère le résumé Markdown d'un manifeste.
pub fn summary(manifest: &Manifest) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "# 0ass — rapport {}", manifest.generated_at);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "**Projet** : {} ({}) · **cible** : {} {} · rustc {} · {}",
        manifest.project.name,
        manifest.project.kind,
        manifest.target.os,
        manifest.target.arch,
        manifest.target.rustc.as_deref().unwrap_or("—"),
        manifest.target.profile
    );

    match &manifest.bench {
        Some(b) => {
            let _ = writeln!(
                out,
                "**Mesure** : {} ({}) · {} runs · médiane {:.1} ms · bruit {:.1} % · {}",
                b.level,
                b.tool,
                b.runs,
                b.median_ms,
                b.mad_pct,
                if b.reliable {
                    "fiable"
                } else {
                    "NON FIABLE (bruit trop élevé)"
                }
            );
        }
        None => {
            let _ = writeln!(
                out,
                "**Mesure** : aucune (projet non mesurable ou banc absent)"
            );
        }
    }
    let _ = writeln!(out);

    let review: Vec<_> = manifest
        .candidates
        .iter()
        .filter(|c| c.status != Status::Rejected)
        .collect();

    if review.is_empty() {
        let _ = writeln!(out, "## Candidats\n\nAucun candidat retenu.");
    } else {
        let _ = writeln!(out, "## Candidats retenus ({})", review.len());
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "| ID | Fonction | Langage | Part runtime | Gain local | Impact global | Stratégie | Risque | Statut |"
        );
        let _ = writeln!(out, "|----|----------|---------|--------------|-----------|---------------|-----------|--------|--------|");
        for c in &review {
            let share = c
                .runtime_share_pct
                .map(|v| format!("{v:.1} %"))
                .unwrap_or_else(|| "—".into());
            let gain = c
                .measurement
                .as_ref()
                .map(|m| format!("**{:+.1} %**", m.local_gain_pct))
                .unwrap_or_else(|| "—".into());
            let global = c
                .measurement
                .as_ref()
                .map(|m| format!("{:+.2} %", m.global_gain_pct))
                .unwrap_or_else(|| "—".into());
            let _ = writeln!(
                out,
                "| {} | `{}` | {} | {} | {} | {} | {} | {} | {} |",
                c.id,
                c.fn_name,
                c.lang,
                share,
                gain,
                global,
                c.strategy,
                c.risk.ub,
                status_label(c.status),
            );
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "## Changements proposés");
    let _ = writeln!(out);
    if review.is_empty() {
        let _ = writeln!(out, "—");
    }
    for c in &review {
        let _ = writeln!(out, "### {} — `{}::{}`", c.id, c.file, c.fn_name);
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "- **Localisation** : `{}` lignes {}–{}",
            c.file, c.start_line, c.end_line
        );
        let _ = writeln!(out, "- **Détecté** : {}", c.detected.join(" · "));
        let _ = writeln!(
            out,
            "- **Stratégie** : `{}` — {}",
            c.strategy, c.strategy_reason
        );
        if !c.targets.is_empty() {
            let _ = writeln!(out, "- **Cibles** : {}", c.targets.join(", "));
        }
        match &c.measurement {
            Some(m) => {
                let _ = writeln!(
                    out,
                    "- **Mesuré** : {:.2} ms → {:.2} ms (**{:+.1} %**), n={}, bruit {:.1} %, MDE {:.1} %",
                    m.before_ms, m.after_ms, m.local_gain_pct, m.n, m.mad_pct, m.mde_pct
                );
                let _ = writeln!(
                    out,
                    "- **Impact global estimé** : {:+.2} % du runtime",
                    m.global_gain_pct
                );
                if !m.regressions.is_empty() {
                    let _ = writeln!(out, "- **Régressions** : {}", m.regressions.join(" · "));
                }
            }
            None => {
                let _ = writeln!(out, "- **Mesuré** : pas encore (candidat à vérifier)");
            }
        }
        let _ = writeln!(
            out,
            "- **Risque** : UB {} · portabilité {} · review humaine requise",
            c.risk.ub, c.risk.portability
        );
        let _ = writeln!(
            out,
            "- **Compatibilité** : signature {} · repli portable {} · vérifications {}",
            if c.compatibility.signature_preserved {
                "inchangée"
            } else {
                "**MODIFIÉE**"
            },
            if c.compatibility.fallback_present {
                "présent"
            } else {
                "**ABSENT**"
            },
            if c.compatibility.compatible {
                "vertes"
            } else if c.compatibility.before.is_empty() && c.compatibility.after.is_empty() {
                "non vérifiées"
            } else {
                "**en échec**"
            }
        );
        if let Some(reason) = &c.compatibility.reason {
            let _ = writeln!(out, "  - {reason}");
        }
        for r in &c.compatibility.after {
            if !r.ok {
                let _ = writeln!(
                    out,
                    "  - échec : `{}` (code {}) — {}",
                    r.command,
                    r.exit_code,
                    r.tail.lines().last().unwrap_or("")
                );
            }
        }
        if let Some(patch) = &c.patch {
            let _ = writeln!(out, "- **Patch** : `{patch}`");
        }
        if let Some(commit) = &c.commit {
            let _ = writeln!(out, "- **Commit** : `{commit}`");
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "## Règles de compatibilité");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Un patch n'est jamais commité s'il ne respecte pas ces trois règles :"
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "1. **Interface** : la signature de la fonction est inchangée, le reste du programme compile."
    );
    let _ = writeln!(
        out,
        "2. **Repli** : un chemin portable subsiste, le programme tourne même si le chemin optimisé n'est pas pris."
    );
    let _ = writeln!(
        out,
        "3. **Comportement** : les vérifications du projet (`[verify]` de `0ass.toml`) passent avant et après."
    );
    let _ = writeln!(out);

    if !manifest.rejected.is_empty() {
        let _ = writeln!(out, "## Écartés ({})", manifest.rejected.len());
        let _ = writeln!(out);
        let _ = writeln!(out, "| ID | Fonction | Raison |");
        let _ = writeln!(out, "|----|----------|--------|");
        for r in &manifest.rejected {
            let _ = writeln!(out, "| {} | `{}` | {} |", r.id, r.fn_name, r.reason);
        }
        let _ = writeln!(out);
    }

    let pending = review.iter().filter(|c| c.status == Status::Review).count();
    let _ = writeln!(out, "## Suite");
    let _ = writeln!(out);
    if pending > 0 {
        let _ = writeln!(
            out,
            "- {pending} patch(s) en attente de review → ouvrir l'interface : `0ass gui`"
        );
        let _ = writeln!(
            out,
            "- Après approbation : un commit par patch, puis **Merge fast-forward dans main**."
        );
        let _ = writeln!(
            out,
            "- Le `git push` reste à ta main : `0ass` ne pousse jamais."
        );
    } else {
        let _ = writeln!(out, "- Aucun patch en attente de review.");
    }

    out
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Review => "à revoir",
        Status::Approved => "approuvé",
        Status::Applied => "appliqué",
        Status::Rejected => "rejeté",
        Status::Stale => "périmé",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ManifestProject, TargetInfo};

    #[test]
    fn summary_mentions_candidates_and_rejections() {
        let mut m = Manifest::new(
            ManifestProject {
                root: "D:/demo".into(),
                name: "demo".into(),
                kind: "rust".into(),
                profile: None,
            },
            TargetInfo {
                os: "linux".into(),
                arch: "x86_64".into(),
                rustc: None,
                profile: "release".into(),
            },
        );
        m.rejected.push(crate::manifest::RejectedCandidate {
            id: "A2".into(),
            file: "src/b.rs".into(),
            fn_name: "g".into(),
            reason: "gain mesuré négatif".into(),
        });
        let md = summary(&m);
        assert!(md.contains("# 0ass — rapport"));
        assert!(md.contains("Écartés (1)"));
        assert!(md.contains("gain mesuré négatif"));
    }
}
