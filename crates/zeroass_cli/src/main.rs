//! `0ass` — interface en ligne de commande.
//!
//! Commandes :
//! - `init`      : génère un `0ass.toml`
//! - `scan`      : scan multi-langages + mesure + rapport
//! - `report`    : affiche le dernier rapport (Markdown ou JSON)
//! - `bench`     : compare un patch appliqué (A/B entrelacé)
//! - `approve`   : approuve un candidat, mesure, commit dans le worktree
//! - `merge`     : fusion fast-forward de la branche 0ass dans la branche courante
//! - `gui`       : lance l'interface graphique

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use zeroass_core::manifest::{Candidate, Manifest};
use zeroass_core::profile::ProjectProfile;
use zeroass_engine::campaign::{self, CampaignOptions};
use zeroass_engine::git;
use zeroass_engine::review;
use zeroass_engine::runner::{self, CommandSpec};

const USAGE: &str = "\
0ass — recherche et vérification de fonctions convertibles en assembleur

USAGE
  0ass <commande> [options]

COMMANDES
  init                  Génère un 0ass.toml dans le projet
  scan                  Scanne le projet, mesure, écrit le rapport
  report                Affiche le dernier rapport (Markdown par défaut, --json)
  bench                 Mesure un patch : 0ass bench --id A1 --patch p.patch
  approve               Approuve un candidat, mesure et commit (worktree isolé)
  merge                 Fusion fast-forward de la branche 0ass dans la branche courante
  gui                   Lance l'interface graphique

OPTIONS COMMUNS
  --project <chemin>    Racine du projet (défaut : répertoire courant)
  --report <chemin>     Dossier de rapport (défaut : %LOCALAPPDATA%/0ass/reports/<projet>)
  --json                Sortie JSON (report)
  --no-bench            Ne pas exécuter le banc (scan statique seul)
  --runs <n>            Nombre d'exécutions mesurées (défaut : 30)
  --help                Cette aide
";

fn main() {
    if let Err(e) = run() {
        eprintln!("erreur : {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" || args[0] == "help" {
        print!("{USAGE}");
        return Ok(());
    }
    let command = args[0].clone();
    let opts = Options::parse(&args[1..])?;

    match command.as_str() {
        "init" => cmd_init(&opts),
        "scan" => cmd_scan(&opts),
        "report" => cmd_report(&opts),
        "bench" => cmd_bench(&opts),
        "approve" => cmd_approve(&opts),
        "merge" => cmd_merge(&opts),
        "gui" => cmd_gui(),
        other => {
            eprint!("{USAGE}");
            bail!("commande inconnue : {other}");
        }
    }
}

/// Options analysées.
struct Options {
    project: PathBuf,
    report_root: Option<PathBuf>,
    json: bool,
    no_bench: bool,
    runs: Option<u32>,
    id: Option<String>,
    patch: Option<PathBuf>,
    branch: Option<String>,
}

impl Options {
    fn parse(args: &[String]) -> Result<Options> {
        let mut o = Options {
            project: std::env::current_dir()?,
            report_root: None,
            json: false,
            no_bench: false,
            runs: None,
            id: None,
            patch: None,
            branch: None,
        };
        let mut i = 0usize;
        while i < args.len() {
            let a = args[i].as_str();
            let value = |i: &mut usize| -> Result<String> {
                *i += 1;
                args.get(*i)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("option {a} attend une valeur"))
            };
            match a {
                "--project" => o.project = PathBuf::from(value(&mut i)?),
                "--report" => o.report_root = Some(PathBuf::from(value(&mut i)?)),
                "--json" => o.json = true,
                "--no-bench" => o.no_bench = true,
                "--runs" => {
                    let v = value(&mut i)?;
                    o.runs = Some(v.parse().context("--runs attend un entier")?);
                }
                "--id" => o.id = Some(value(&mut i)?),
                "--patch" => o.patch = Some(PathBuf::from(value(&mut i)?)),
                "--branch" => o.branch = Some(value(&mut i)?),
                other => bail!("option inconnue : {other}"),
            }
            i += 1;
        }
        Ok(o)
    }
}

/// Dossier de rapport d'un projet.
fn report_root(opts: &Options) -> PathBuf {
    opts.report_root
        .clone()
        .unwrap_or_else(|| campaign::default_report_root(&opts.project))
}

/// `0ass init`
fn cmd_init(opts: &Options) -> Result<()> {
    let path = opts.project.join("0ass.toml");
    if path.exists() {
        println!("déjà présent : {}", path.display());
        return Ok(());
    }
    let name = opts
        .project
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "projet".into());
    let template = format!(
        r#"# Profil 0ass — {name}

[project]
name = "{name}"
kind = "generic"
source_roots = ["."]

[build]
command = ""
binary = ""

[run]
# Commande de la charge de travail chronométrée (la plus petite qui
# exerce le code que vous voulez optimiser).
command = ""
warmup = 3
runs = 30

# [bench]
# command = "cargo bench"

# [profile]
# linux = "perf record -g -- cargo bench"
# macos = "xcrun xctrace record --template 'Time Profile' --launch -- cargo bench"

# Vérifications exécutées avant et après chaque patch.
# Elles garantissent que le code réécrit reste compatible avec le reste du
# programme et que le programme continue de tourner :
#   - `check` vérifie l'interface (le reste du code compile encore) ;
#   - `test`  vérifie le comportement (les tests passent encore) ;
#   - un patch sans repli portable est refusé.
[verify]
check = []
test = []
"#
    );
    std::fs::write(&path, template)?;
    println!("écrit : {}", path.display());
    println!("complète [run].command pour activer la mesure.");
    println!("complète [verify] pour que les patches soient vérifiés (signature, repli, tests).");
    Ok(())
}

/// `0ass scan`
fn cmd_scan(opts: &Options) -> Result<()> {
    let mut options = CampaignOptions::new(opts.project.clone());
    options.report_root = opts.report_root.clone();
    options.skip_bench = opts.no_bench;

    let campaign = campaign::run(&options)?;
    println!("{}", campaign.summary_md);
    println!("Rapport : {}", campaign.report_dir.display());
    println!(
        "Manifeste : {}",
        campaign.report_dir.join("manifest.json").display()
    );
    Ok(())
}

/// `0ass report`
fn cmd_report(opts: &Options) -> Result<()> {
    let root = report_root(opts);
    let dir = campaign::latest_report(&root)?;
    if opts.json {
        let manifest = campaign::load_manifest(&dir)?;
        println!("{}", manifest.to_json());
    } else {
        let md = std::fs::read_to_string(dir.join("summary.md"))
            .with_context(|| format!("lecture de {}", dir.join("summary.md").display()))?;
        print!("{md}");
    }
    Ok(())
}

/// Charge le manifeste et retrouve un candidat par identifiant.
fn find_candidate<'a>(manifest: &'a Manifest, id: &str) -> Result<&'a Candidate> {
    manifest
        .candidates
        .iter()
        .find(|c| c.id.eq_ignore_ascii_case(id))
        .ok_or_else(|| anyhow::anyhow!("candidat inconnu : {id}"))
}

/// Prépare le worktree d'une campagne et renvoie (chemin, branche).
fn prepare_worktree(
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

/// `0ass approve` : applique le patch, verifie la compatibilite, mesure, commit.
fn cmd_approve(opts: &Options) -> Result<()> {
    let id = opts
        .id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--id <A1> est requis"))?;
    let patch = opts
        .patch
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--patch <fichier.patch> est requis"))?;

    let root = report_root(opts);
    let report_dir = campaign::latest_report(&root)?;
    let outcome = review::approve(
        &opts.project,
        &report_dir,
        &id,
        &patch,
        opts.runs,
        opts.branch.as_deref(),
    )?;
    println!("{}", outcome.message);
    if let Some(c) = outcome.compatibility.reason.as_ref() {
        if outcome.commit.is_none() {
            println!("motif : {c}");
        }
    }
    if let Some(m) = &outcome.measurement {
        println!(
            "mesure : {:.2} ms -> {:.2} ms ({:+.1} %) · bruit {:.1} % · MDE {:.1} %",
            m.before_ms, m.after_ms, m.local_gain_pct, m.mad_pct, m.mde_pct
        );
    }
    if let Some(sha) = &outcome.commit {
        println!("commit : {sha} (branche {})", outcome.branch);
        println!(
            "manifeste mis à jour : {}",
            report_dir.join("manifest.json").display()
        );
        println!("reste à fusionner : `0ass merge` (le push reste à ta main)");
    }
    Ok(())
}

/// `0ass merge` : fusion fast-forward dans la branche courante.
fn cmd_merge(opts: &Options) -> Result<()> {
    let root = report_root(opts);
    let report_dir = campaign::latest_report(&root)?;
    let message = review::merge_ff(&opts.project, &report_dir)?;
    println!("{message}");
    Ok(())
}

/// `0ass bench` : mesure un patch sans approuver.
fn cmd_bench(opts: &Options) -> Result<()> {
    let id = opts
        .id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--id <A1> est requis"))?;
    let patch = opts
        .patch
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--patch <fichier.patch> est requis"))?;

    let root = report_root(opts);
    let report_dir = campaign::latest_report(&root)?;
    let manifest = campaign::load_manifest(&report_dir)?;
    let _candidate = find_candidate(&manifest, &id)?;

    let profile = ProjectProfile::load_or_default(&opts.project);
    let (worktree, branch) = prepare_worktree(&opts.project, &report_dir, opts.branch.as_deref())?;
    let _touched = git::apply_patch(&worktree, &patch)?;
    if !profile.build.command.trim().is_empty() {
        let spec = CommandSpec::new(profile.build.command.clone(), &worktree);
        runner::capture(&spec)?;
    }
    if profile.run.command.trim().is_empty() {
        bail!("aucune commande de mesure : renseigne [run].command dans 0ass.toml");
    }
    let before = CommandSpec::new(profile.run.command.clone(), &opts.project);
    let after = CommandSpec::new(profile.run.command.clone(), &worktree);
    let runs = opts.runs.unwrap_or(profile.run.runs);
    let ab = runner::compare_interleaved(&before, &after, profile.run.warmup, runs)?;
    let c = &ab.comparison;
    println!("branche de travail : {branch}");
    println!(
        "{:.2} ms -> {:.2} ms ({:+.1} %) · bruit {:.1} % · MDE {:.1} % · n={} · {}",
        c.before_ms,
        c.after_ms,
        c.local_gain_pct,
        c.mad_pct,
        c.mde_pct,
        c.n,
        if c.significant {
            "significatif"
        } else {
            "NON significatif"
        }
    );
    Ok(())
}

/// `0ass gui`
fn cmd_gui() -> Result<()> {
    let exe = if cfg!(windows) {
        "0ass-gui.exe"
    } else {
        "0ass-gui"
    };
    let status = Command::new(exe).status().with_context(|| {
        format!("lanceur {exe} introuvable : construis l'interface (cargo build -p zeroass_gui)")
    })?;
    if !status.success() {
        bail!("l'interface s'est terminée avec le code {status}");
    }
    Ok(())
}
