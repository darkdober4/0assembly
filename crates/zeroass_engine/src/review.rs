//! Review humaine : approuver, rejeter, fusionner.
//!
//! Ce module regroupe la logique d'approbation pour que l'interface en ligne
//! de commande et l'interface graphique fasse **exactement** la même chose.
//!
//! Règles tenues ici :
//!
//! * le projet analysé n'est jamais modifié directement : tout passe par un
//!   *worktree* isolé, hors du dépôt ;
//! * un patch n'est commité que s'il est **compatible** (interface inchangée,
//!   repli portable, vérifications du projet vertes) ;
//! * un commit par patch, sur une branche `0ass/<date>` ;
//! * la fusion est un **fast-forward** vers la branche courante ;
//! * `0ass` ne pousse jamais : le `git push` reste à la main.

use std::path::{Path, PathBuf};

use anyhow::Result;

use zeroass_core::manifest::{Candidate, Manifest, Measurement, Status};
use zeroass_core::profile::ProjectProfile;

use crate::campaign;
use crate::git;
use crate::runner::{self, CommandSpec};
use crate::stats::Samples;
use crate::verify;

/// Résultat d'une approbation.
#[derive(Debug, Clone)]
pub struct ApproveOutcome {
    /// Identifiant du candidat.
    pub id: String,
    /// Branche de travail utilisée.
    pub branch: String,
    /// SHA du commit créé.
    pub commit: Option<String>,
    /// Mesure A/B, si elle a pu être faite.
    pub measurement: Option<Measurement>,
    /// Vérification de compatibilité.
    pub compatibility: zeroass_core::manifest::Compatibility,
    /// Message destiné à l'utilisateur.
    pub message: String,
}

/// Prépare le worktree isolé de la campagne.
pub fn prepare_worktree(
    project: &Path,
    report_dir: &Path,
    branch: Option<&str>,
) -> Result<(PathBuf, String)> {
    git::ensure_repo(project)?;
    let date = zeroass_core::today_compact();
    let branch = branch
        .map(|b| b.to_string())
        .unwrap_or_else(|| git::branch_name("0ass", &date));
    let wt = campaign::worktree_dir(report_dir);
    if let Some(parent) = wt.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let base = git::current_branch(project)?;
    git::worktree_add(project, &wt, &branch, &base)?;
    Ok((wt, branch))
}

/// Mesure A/B entrelacée entre le projet d'origine et le worktree patché.
pub fn measure_ab(
    profile: &ProjectProfile,
    project: &Path,
    worktree: &Path,
    runs: Option<u32>,
) -> Result<Option<Measurement>> {
    if profile.run.command.trim().is_empty() {
        return Ok(None);
    }
    let runs = runs.unwrap_or(profile.run.runs);
    let before = CommandSpec::new(profile.run.command.clone(), project);
    let after = CommandSpec::new(profile.run.command.clone(), worktree);
    let ab = runner::compare_interleaved(&before, &after, profile.run.warmup, runs)?;
    let c = &ab.comparison;
    Ok(Some(Measurement {
        local_gain_pct: c.local_gain_pct,
        global_gain_pct: c.local_gain_pct * 0.0,
        before_ms: c.before_ms,
        after_ms: c.after_ms,
        n: c.n as u32,
        mad_pct: c.mad_pct,
        mde_pct: c.mde_pct,
        regressions: Vec::new(),
    }))
}

/// Applique un patch, vérifie sa compatibilité, le mesure et le commite.
///
/// Si le patch n'est pas compatible, **rien n'est commité** : le candidat est
/// marqué comme rejeté et le manifeste est réécrit.
pub fn approve(
    project: &Path,
    report_dir: &Path,
    id: &str,
    patch: &Path,
    runs: Option<u32>,
    branch: Option<&str>,
) -> Result<ApproveOutcome> {
    let mut manifest = campaign::load_manifest(report_dir)?;
    let candidate = manifest
        .candidates
        .iter()
        .find(|c| c.id == id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("candidat inconnu : {id}"))?;
    let profile = ProjectProfile::load_or_default(project);
    let (worktree, branch_name) = prepare_worktree(project, report_dir, branch)?;
    let base_branch = git::current_branch(project)?;

    // 1. Application du patch dans le worktree isolé.
    //    Si la branche de travail est vierge (aucun patch approuvé), on la
    //    remet à l'état de la base : un essai précédent laisse parfois un
    //    patch à moitié appliqué.
    if git::commits_since(&worktree, &base_branch) == 0 {
        git::reset_worktree(&worktree)?;
    }
    let already_applied = git::patch_applied(&worktree, patch);
    if !already_applied {
        let backup_dir = report_dir.join("backups");
        let _ = git::backup_patch_targets(&worktree, patch, &backup_dir);
    }
    let touched = if already_applied {
        git::changed_files(&worktree)
    } else {
        git::apply_patch(&worktree, patch)?
    };
    if touched.is_empty() && !already_applied {
        anyhow::bail!("le patch ne modifie aucun fichier");
    }
    if touched.is_empty() && already_applied {
        // Le patch est déjà commité : le candidat a été approuvé auparavant.
        let sha = git::head_sha(&worktree)?;
        return Ok(ApproveOutcome {
            id: id.to_string(),
            branch: branch_name,
            commit: Some(sha.clone()),
            measurement: None,
            compatibility: candidate.compatibility.clone(),
            message: format!("déjà approuvé (commit {sha})"),
        });
    }

    // 2. Construction des deux variantes : la base (pour la mesure « avant »)
    //    et le worktree patché.
    if !profile.build.command.trim().is_empty() {
        let base_spec = CommandSpec::new(profile.build.command.clone(), project);
        let base_out = runner::capture(&base_spec)?;
        if !base_out.trim().is_empty() {
            // La construction de la base doit reussir, sinon la mesure « avant »
            // n'a aucun sens.
            let spec = CommandSpec::new(profile.build.command.clone(), &worktree);
            runner::capture(&spec)?;
        }
    }

    // 3. Compatibilité (signature, repli portable, tests du projet).
    let compatibility = verify::verify_patch(&profile, project, &worktree, &candidate);
    if !compatibility.compatible {
        let reason = compatibility
            .reason
            .clone()
            .unwrap_or_else(|| "compatibilité non vérifiée".into());
        if let Some(c) = manifest.candidates.iter_mut().find(|c| c.id == id) {
            c.compatibility = compatibility.clone();
            c.status = Status::Rejected;
        }
        save_manifest(report_dir, &manifest)?;
        return Ok(ApproveOutcome {
            id: id.to_string(),
            branch: branch_name,
            commit: None,
            measurement: None,
            compatibility,
            message: format!("REFUS : {reason}"),
        });
    }

    // 4. Mesure A/B entrelacée.
    let measurement = measure_ab(&profile, project, &worktree, runs)?;

    // 5. Un commit par patch.
    let message = format!(
        "perf({}): {} — {:+}\n\n{}\n\n0ass {} · rapport {}",
        candidate.file,
        candidate.fn_name,
        measurement
            .as_ref()
            .map(|m| format!("{:.1} % local", m.local_gain_pct))
            .unwrap_or_else(|| "non mesuré".into()),
        candidate.strategy_reason,
        candidate.id,
        manifest.generated_at,
    );
    let sha = git::commit(&worktree, &touched, &message)?;

    // 6. Mise à jour du manifeste.
    if let Some(c) = manifest.candidates.iter_mut().find(|c| c.id == id) {
        c.measurement = measurement.clone();
        c.compatibility = compatibility.clone();
        c.status = Status::Approved;
        c.patch = Some(patch.to_string_lossy().to_string());
        c.commit = Some(sha.clone());
    }
    save_manifest(report_dir, &manifest)?;

    Ok(ApproveOutcome {
        id: id.to_string(),
        branch: branch_name,
        commit: Some(sha),
        measurement,
        compatibility,
        message: "approuvé — reste à fusionner (le push reste à ta main)".into(),
    })
}

/// Mesure un patch **sans** l'approuver.
///
/// Le patch est appliqué dans le worktree isolé, les deux variantes sont
/// compilées, puis la comparaison A/B entrelacée est faite. La mesure est
/// consignée dans le manifeste ; rien n'est commité.
pub fn measure_patch(
    project: &Path,
    report_dir: &Path,
    id: &str,
    patch: &Path,
    runs: Option<u32>,
) -> Result<ApproveOutcome> {
    let mut manifest = campaign::load_manifest(report_dir)?;
    let candidate = manifest
        .candidates
        .iter()
        .find(|c| c.id == id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("candidat inconnu : {id}"))?;
    let profile = ProjectProfile::load_or_default(project);
    let (worktree, branch_name) = prepare_worktree(project, report_dir, None)?;
    let base_branch = git::current_branch(project)?;
    if git::commits_since(&worktree, &base_branch) == 0 {
        git::reset_worktree(&worktree)?;
    }
    let already = git::patch_applied(&worktree, patch);
    if !already {
        let backup_dir = report_dir.join("backups");
        let _ = git::backup_patch_targets(&worktree, patch, &backup_dir);
        git::apply_patch(&worktree, patch)?;
    }

    if !profile.build.command.trim().is_empty() {
        let base_spec = CommandSpec::new(profile.build.command.clone(), project);
        runner::capture(&base_spec)?;
        let spec = CommandSpec::new(profile.build.command.clone(), &worktree);
        runner::capture(&spec)?;
    }

    let measurement = measure_ab(&profile, project, &worktree, runs)?;
    let message = match &measurement {
        Some(m) => format!(
            "{:.2} ms -> {:.2} ms ({:+.1} %) · bruit {:.1} % · MDE {:.1} %",
            m.before_ms, m.after_ms, m.local_gain_pct, m.mad_pct, m.mde_pct
        ),
        None => "aucune commande de mesure : renseigne [run].command dans 0ass.toml".to_string(),
    };

    if let Some(c) = manifest.candidates.iter_mut().find(|c| c.id == id) {
        c.measurement = measurement.clone();
    }
    save_manifest(report_dir, &manifest)?;

    Ok(ApproveOutcome {
        id: id.to_string(),
        branch: branch_name,
        commit: None,
        measurement,
        compatibility: candidate.compatibility,
        message,
    })
}

/// Rejette un candidat (décision humaine).
pub fn reject(report_dir: &Path, id: &str, reason: &str) -> Result<()> {
    let mut manifest = campaign::load_manifest(report_dir)?;
    if let Some(c) = manifest.candidates.iter_mut().find(|c| c.id == id) {
        c.status = Status::Rejected;
    }
    manifest
        .rejected
        .push(zeroass_core::manifest::RejectedCandidate {
            id: id.to_string(),
            file: String::new(),
            fn_name: String::new(),
            reason: reason.to_string(),
        });
    save_manifest(report_dir, &manifest)?;
    Ok(())
}

/// Fusion fast-forward de la branche `0ass/<date>` dans la branche courante.
pub fn merge_ff(project: &Path, report_dir: &Path) -> Result<String> {
    let manifest = campaign::load_manifest(report_dir)?;
    let has_commit = manifest.candidates.iter().any(|c| c.commit.is_some());
    if !has_commit {
        anyhow::bail!("aucun patch approuvé dans ce rapport");
    }
    let wt = campaign::worktree_dir(report_dir);
    let branch = git::current_branch(&wt)?;
    let current = git::current_branch(project)?;
    git::merge_ff_only(project, &branch)?;
    Ok(format!(
        "fusion fast-forward : {branch} -> {current}. Le `git push` reste à ta main."
    ))
}

/// Écrit le manifeste et le résumé Markdown.
pub fn save_manifest(report_dir: &Path, manifest: &Manifest) -> Result<()> {
    std::fs::write(report_dir.join("manifest.json"), manifest.to_json())?;
    let md = zeroass_core::summary::summary(manifest);
    std::fs::write(report_dir.join("summary.md"), md)?;
    Ok(())
}

/// Mesures brutes d'un banc (utilisées par l'interface pour l'affichage).
pub fn bench_samples(spec: &CommandSpec, warmup: u32, runs: u32) -> Result<Samples> {
    runner::measure(spec, warmup, runs)
}

/// Candidat du manifeste correspondant à un identifiant.
pub fn candidate_of<'a>(manifest: &'a Manifest, id: &str) -> Option<&'a Candidate> {
    manifest.candidates.iter().find(|c| c.id == id)
}
