//! Intégration Git : worktree isolé, commits, fusion fast-forward.
//!
//! `0ass` n'écrit **jamais** dans l'arbre de travail de l'utilisateur : tout passe
//! par un worktree dédié. Et `0ass` ne pousse jamais : le push reste humain.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

/// Exécute une commande git et renvoie sa sortie.
pub fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .with_context(|| format!("git {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "git {} a échoué : {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Branche courante.
pub fn current_branch(root: &Path) -> Result<String> {
    git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
}

/// L'arbre de travail est-il propre ?
pub fn is_clean(root: &Path) -> Result<bool> {
    // Les fichiers non suivis (build, `0ass.toml`, rapports) ne bloquent pas
    // une fusion : seuls les fichiers **suivis** modifiés comptent.
    Ok(git(root, &["status", "--porcelain", "--untracked-files=no"])?.is_empty())
}

/// SHA du commit courant.
pub fn head_sha(root: &Path) -> Result<String> {
    git(root, &["rev-parse", "HEAD"])
}

/// Crée un worktree isolé sur une branche donnée.
///
/// La branche est créée depuis `base` si elle n'existe pas encore.
pub fn worktree_add(root: &Path, path: &Path, branch: &str, base: &str) -> Result<()> {
    if path.exists() {
        // Worktree déjà présent : on le réutilise.
        let _ = git(root, &["worktree", "prune"]);
        return Ok(());
    }
    let exists = git(root, &["rev-parse", "--verify", "--quiet", branch]).is_ok();
    if exists {
        git(root, &["worktree", "add", &path.to_string_lossy(), branch])?;
    } else {
        git(
            root,
            &[
                "worktree",
                "add",
                "-b",
                branch,
                &path.to_string_lossy(),
                base,
            ],
        )?;
    }
    Ok(())
}

/// Supprime un worktree et sa branche.
pub fn worktree_remove(root: &Path, path: &Path, branch: Option<&str>) {
    let _ = git(
        root,
        &["worktree", "remove", "--force", &path.to_string_lossy()],
    );
    let _ = git(root, &["worktree", "prune"]);
    if let Some(b) = branch {
        let _ = git(root, &["branch", "-D", b]);
    }
}

/// Applique un fichier de patch dans un worktree.
/// Applique un patch et renvoie la liste des fichiers touches.
///
/// Règle du projet : jamais de `git add -A`. Seuls les fichiers réellement
/// modifiés par le patch sont listés, et donc seuls eux seront commités.
pub fn apply_patch(worktree: &Path, patch: &Path) -> Result<Vec<String>> {
    if !patch.is_file() {
        bail!("patch introuvable : {}", patch.display());
    }
    git(worktree, &["apply", "--3way", &patch.to_string_lossy()])?;
    Ok(changed_files(worktree))
}

/// Fichiers modifiés dans le worktree (par rapport à HEAD).
pub fn changed_files(worktree: &Path) -> Vec<String> {
    match git(worktree, &["status", "--porcelain"]) {
        Ok(out) => out
            .lines()
            .filter_map(|line| line.get(3..))
            .map(|path| path.trim().to_string())
            .map(|path| match path.split_once(" -> ") {
                Some((_, new)) => new.to_string(),
                None => path,
            })
            .filter(|p| !p.is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Copie les fichiers cibles d'un patch vers un **dossier de backup centralisé**.
///
/// Les fichiers ne sont copiés que s'ils n'existent pas déjà dans le backup :
/// on conserve ainsi la version la plus ancienne (avant toute modification).
/// Les fichiers nouvellement créés n'ont pas de version antérieure à sauvegarder.
pub fn backup_patch_targets(
    worktree: &Path,
    patch: &Path,
    backup_dir: &Path,
) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(patch)
        .with_context(|| format!("lecture du patch {}", patch.display()))?;
    let mut backed_up = Vec::new();

    for line in content.lines() {
        let Some(rest) = line.strip_prefix("diff --git ") else {
            continue;
        };
        let parts: Vec<&str> = rest.split_whitespace().collect();
        let Some(b_path) = parts.iter().find(|p| p.starts_with("b/")) else {
            continue;
        };
        let rel = &b_path[2..];
        let src = worktree.join(rel);
        if !src.exists() {
            continue;
        }
        let dst = backup_dir.join(rel);
        // Déduplication : on garde la toute première version capturée.
        if dst.exists() {
            continue;
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("création du dossier de backup {}", parent.display()))?;
        }
        std::fs::copy(&src, &dst)
            .with_context(|| format!("backup de {} vers {}", rel, dst.display()))?;
        backed_up.push(rel.to_string());
    }

    Ok(backed_up)
}

/// Le patch est-il déjà appliqué dans le worktree ?
///
/// On le détecte en cherchant à l'appliquer **à l'envers** : si l'annulation
/// est possible, c'est que le patch est déjà en place.
pub fn patch_applied(worktree: &Path, patch: &Path) -> bool {
    if !patch.is_file() {
        return false;
    }
    git(
        worktree,
        &["apply", "--check", "--reverse", &patch.to_string_lossy()],
    )
    .is_ok()
}

/// Nombre de commits de la branche par rapport à `base`.
pub fn commits_since(worktree: &Path, base: &str) -> usize {
    git(worktree, &["rev-list", "--count", &format!("{base}..HEAD")])
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0)
}

/// Remet le worktree dans l'état de HEAD (branche vierge).
pub fn reset_worktree(worktree: &Path) -> Result<()> {
    git(worktree, &["reset", "--hard"])?;
    git(worktree, &["clean", "-fd"])?;
    Ok(())
}

/// Crée un commit et renvoie son SHA.
///
/// Seuls les fichiers indiqués sont ajoutés à l'index : le commit ne peut pas
/// embarquer un fichier étranger.
pub fn commit(worktree: &Path, files: &[String], message: &str) -> Result<String> {
    ensure_identity(worktree)?;
    for f in files {
        git(worktree, &["add", "--", f])?;
    }
    git(worktree, &["commit", "-m", message])?;
    head_sha(worktree)
}

/// Vérifie que Git knows qui est l'auteur du commit.
pub fn ensure_identity(root: &Path) -> Result<()> {
    match git(root, &["var", "GIT_AUTHOR_IDENT"]) {
        Ok(_) => Ok(()),
        Err(_) => bail!(
            "identité Git inconnue : configure `git config user.name \"...\"` et \
             `git config user.email \"...\"` dans le dépôt analysé"
        ),
    }
}

/// Fusionne une branche dans la branche courante en **fast-forward** uniquement.
pub fn merge_ff_only(root: &Path, branch: &str) -> Result<()> {
    if !is_clean(root)? {
        bail!("l'arbre de travail n'est pas propre : commit ou stash avant la fusion");
    }
    git(root, &["merge", "--ff-only", branch])?;
    Ok(())
}

/// Vérifie qu'une branche peut être fusionnée en fast-forward.
pub fn can_fast_forward(root: &Path, branch: &str) -> bool {
    git(root, &["merge-base", "--is-ancestor", "HEAD", branch]).is_ok()
}

/// Liste les worktrees actifs.
pub fn worktrees(root: &Path) -> Result<Vec<String>> {
    let out = git(root, &["worktree", "list"])?;
    Ok(out.lines().map(|l| l.to_string()).collect())
}

/// Crée un nom de branche daté.
pub fn branch_name(prefix: &str, date: &str) -> String {
    format!("{prefix}/{date}")
}

/// Chemin du worktree par défaut d'un projet.
pub fn default_worktree(root: &Path) -> PathBuf {
    root.join(".0ass-wt")
}

/// Vérifie que le dépôt est bien un dépôt git.
pub fn ensure_repo(root: &Path) -> Result<()> {
    if !root.join(".git").exists() && git(root, &["rev-parse", "--git-dir"]).is_err() {
        return Err(anyhow!("{} n'est pas un dépôt git", root.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("0ass-git-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let init = |args: &[&str]| {
            Command::new("git")
                .current_dir(&dir)
                .args(args)
                .output()
                .unwrap();
        };
        init(&["init", "-q"]);
        init(&["config", "user.email", "0ass@example.invalid"]);
        init(&["config", "user.name", "0ass test"]);
        std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
        init(&["add", "-A"]);
        init(&["commit", "-q", "-m", "init"]);
        dir
    }

    #[test]
    fn reads_branch_and_cleanliness() {
        let dir = temp_repo("status");
        assert!(!current_branch(&dir).unwrap().is_empty());
        assert!(is_clean(&dir).unwrap());
        // Un fichier non suivi ne bloque pas une fusion fast-forward.
        std::fs::write(dir.join("b.txt"), "x").unwrap();
        assert!(is_clean(&dir).unwrap());
        // Un fichier suivi et modifié, si.
        std::fs::write(dir.join("a.txt"), "change\n").unwrap();
        assert!(!is_clean(&dir).unwrap());
    }

    #[test]
    fn worktree_and_fast_forward_merge() {
        let dir = temp_repo("wt");
        // Le worktree vit HORS du dépôt : sinon l'arbre principal paraît sale.
        let outside = std::env::temp_dir().join(format!("0ass-wt-{}-{}", "wt", std::process::id()));
        let _ = std::fs::remove_dir_all(&outside);
        let wt = outside.join("wt");
        let branch = "0ass/test-branch";
        worktree_add(&dir, &wt, branch, "HEAD").unwrap();
        std::fs::write(wt.join("c.txt"), "from worktree\n").unwrap();
        let sha = commit(&wt, &["c.txt".to_string()], "feat: ajout").unwrap();
        assert_eq!(sha.len(), 40);
        assert!(can_fast_forward(&dir, branch));
        merge_ff_only(&dir, branch).unwrap();
        assert!(dir.join("c.txt").exists());
        worktree_remove(&dir, &wt, Some(branch));
        let _ = std::fs::remove_dir_all(&outside);
    }
}
