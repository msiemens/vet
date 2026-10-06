//! What `roslyn-language-server` is told to load. It loads nothing by itself
//! unless sent `solution/open` or `project/open` after `initialized`.
//!
//! The candidates are the files of the tree as Vet sees it: in a worktree the
//! scan, which drops what git ignores; in a materialized snapshot every file,
//! since it holds nothing but the snapshot's own.

use std::path::{Path, PathBuf};

use vet_app::ports::{Dirty, GitRepo};

use crate::git::GixRepo;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Load {
    /// The only solution in the root.
    Solution(PathBuf),
    /// Every project, when there is no single solution in the root to load.
    Projects(Vec<PathBuf>),
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| extensions.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// The files of the tree at `root`, relative to it.
pub(crate) async fn files(root: &Path) -> Result<Vec<PathBuf>, String> {
    // `.git` is how git marks a worktree's root, as a directory or a file.
    if root.join(".git").exists() {
        let scan = GixRepo::new(root).scan(Dirty::All).await.map_err(|e| e.0)?;
        return Ok(scan
            .snapshot
            .entries
            .keys()
            .map(|p| PathBuf::from(String::from_utf8_lossy(&p.0).into_owned()))
            .collect());
    }
    let mut found = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .flatten()
        {
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                pending.push(path);
            } else if let Ok(relative) = path.strip_prefix(root) {
                found.push(relative.to_owned());
            }
        }
    }
    Ok(found)
}

/// The solution (`.sln` or `.slnx`) directly in `root` if there is exactly one,
/// else every `.csproj` of the tree. `None` if there is nothing to load.
pub async fn find(root: &Path) -> Result<Option<Load>, String> {
    let files = files(root).await?;
    let mut solutions: Vec<PathBuf> = (files.iter())
        .filter(|f| f.components().count() == 1 && has_extension(f, &["sln", "slnx"]))
        .map(|f| root.join(f))
        .collect();
    if solutions.len() == 1 {
        return Ok(solutions.pop().map(Load::Solution));
    }
    let mut projects: Vec<PathBuf> = (files.iter())
        .filter(|f| has_extension(f, &["csproj"]))
        .map(|f| root.join(f))
        .collect();
    projects.sort();
    Ok((!projects.is_empty()).then_some(Load::Projects(projects)))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    fn touch(root: &Path, name: &str) {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}");
    }

    #[tokio::test]
    async fn a_single_solution_is_loaded_whole() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "App.sln");
        touch(dir.path(), "src/App.csproj");
        assert_eq!(
            find(dir.path()).await.unwrap(),
            Some(Load::Solution(dir.path().join("App.sln")))
        );
    }

    #[tokio::test]
    async fn without_a_single_solution_every_project_is_loaded() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "A.sln");
        touch(dir.path(), "B.slnx");
        touch(dir.path(), "src/App/App.csproj");
        touch(dir.path(), "a/b/c/d/e/Deep.csproj");
        assert_eq!(
            find(dir.path()).await.unwrap(),
            Some(Load::Projects(vec![
                dir.path().join("a/b/c/d/e/Deep.csproj"),
                dir.path().join("src/App/App.csproj"),
            ]))
        );
    }

    #[tokio::test]
    async fn a_worktree_contributes_what_git_does_not_ignore() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        touch(dir.path(), ".gitignore");
        std::fs::write(dir.path().join(".gitignore"), "obj/\nbuild-output/\n").unwrap();
        touch(dir.path(), "src/App/App.csproj");
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", "base"]);
        touch(dir.path(), "src/New/New.csproj");
        touch(dir.path(), "src/App/obj/Generated.csproj");
        touch(dir.path(), "build-output/Other.csproj");
        assert_eq!(
            find(dir.path()).await.unwrap(),
            Some(Load::Projects(vec![
                dir.path().join("src/App/App.csproj"),
                dir.path().join("src/New/New.csproj"),
            ]))
        );
    }

    #[tokio::test]
    async fn nothing_to_load_is_none() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "readme.md");
        assert_eq!(find(dir.path()).await.unwrap(), None);
    }
}
