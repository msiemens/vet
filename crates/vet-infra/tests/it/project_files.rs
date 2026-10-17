//! What a project keeps outside the store: its directory in the data directory, and the refs Vet
//! keeps in its repository. `git` is used only to build the fixtures and to look at the result.

#![allow(clippy::disallowed_methods)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use vet_app::changeset::ChangesetId;
use vet_app::ports::{ProjectFiles, TreeUsage, VetRefs};
use vet_core::content::{Hash, SnapshotId};
use vet_infra::budget::{Budget, REVIEW_IO};
use vet_infra::git::RepoRefs;
use vet_infra::materialize::projects::ProjectTrees;
use vet_infra::storage::DataFiles;
use vet_infra::storage::layout::{PROJECTS, SOURCES, project_dir};
use vet_semantic::policy::RestoreKind;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args([
            "-c",
            "user.name=Tess",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn snapshot(n: u8) -> SnapshotId {
    SnapshotId(Hash([n; 32]))
}

/// The trees below `data`, of which an open review needs those of the snapshots `needed`.
fn trees(data: &Path, needed: &[u8]) -> Arc<ProjectTrees> {
    let needed: HashSet<SnapshotId> = needed.iter().map(|n| snapshot(*n)).collect();
    Arc::new(ProjectTrees::new(
        data.to_owned(),
        Arc::new(move || needed.clone()),
        Budget::new(REVIEW_IO),
    ))
}

fn files(data: &Path, trees: &Arc<ProjectTrees>) -> DataFiles {
    DataFiles::new(data.to_owned(), trees.clone(), Budget::new(REVIEW_IO))
}

/// Keeps a tree of `bytes` bytes of the snapshot `n` under the project at `root`.
async fn keep(trees: &ProjectTrees, root: &str, n: u8, bytes: usize) -> vet_infra::materialize::kept::Lease {
    trees
        .of(root)
        .await
        .acquire(snapshot(n), RestoreKind::Fetch, |dest: PathBuf| async move {
            std::fs::write(dest.join("file"), vec![b'x'; bytes])
        })
        .await
        .unwrap()
        .lease
}

#[tokio::test]
async fn a_projects_trees_are_counted_from_its_own_directory_and_those_in_use_apart() {
    let data = tempfile::tempdir().unwrap();
    let none = trees(data.path(), &[]);
    let needing = trees(data.path(), &[2]);
    let (files, needing) = (files(data.path(), &none), files(data.path(), &needing));
    let leased = keep(&none, "/work/a", 1, 1000).await;
    drop(keep(&none, "/work/a", 2, 500).await);
    drop(keep(&none, "/work/b", 3, 4000).await);

    let a = files.trees("/work/a").await;
    assert_eq!(
        a,
        TreeUsage {
            total: 1500,
            in_use: 1000
        },
        "the leased tree is in use"
    );
    let b = files.trees("/work/b").await;
    assert_eq!(b, TreeUsage { total: 4000, in_use: 0 });
    assert_eq!(files.trees("/work/never").await, TreeUsage::default());

    assert_eq!(
        needing.trees("/work/a").await.in_use,
        1500,
        "a tree an open review needs is in use too"
    );
    drop(leased);
    assert_eq!(needing.trees("/work/a").await.in_use, 500);
    assert_eq!(files.trees("/work/a").await.in_use, 0);

    let last_used = project_dir(data.path(), "/work/a")
        .join(SOURCES)
        .read_dir()
        .unwrap()
        .flatten()
        .map(|entry| entry.path().join("meta").join("last-used"))
        .filter(|file| file.is_file());
    for file in last_used {
        let old = SystemTime::now() - Duration::from_secs(15 * 24 * 3600);
        std::fs::OpenOptions::new()
            .write(true)
            .open(file)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    assert_eq!(
        needing.trees("/work/a").await.in_use,
        0,
        "a tree needed by an open review nobody has used for two weeks is not"
    );
}

#[tokio::test]
async fn removing_a_project_deletes_its_directory_only_and_a_sweep_takes_what_a_removal_left() {
    let data = tempfile::tempdir().unwrap();
    let trees = trees(data.path(), &[]);
    let files = files(data.path(), &trees);
    drop(keep(&trees, "/work/a", 1, 10).await);
    drop(keep(&trees, "/work/b", 2, 10).await);
    let mut roots = files.roots().await;
    roots.sort();
    assert_eq!(roots, ["/work/a", "/work/b"], "each project directory names its root");

    files.remove("/work/a").await;

    assert!(!project_dir(data.path(), "/work/a").exists());
    assert_eq!(files.roots().await, ["/work/b"]);
    assert_eq!(files.trees("/work/b").await.total, 10);
    files.remove("/work/a").await;
    files.remove("/work/never").await;

    let left = data.path().join(PROJECTS).join(".trash").join("1-a");
    std::fs::create_dir_all(left.join("sources")).unwrap();
    std::fs::write(left.join("root"), "/work/c").unwrap();
    assert_eq!(
        files.roots().await,
        ["/work/b"],
        "what waits to be deleted is no project"
    );
    files.sweep().await;
    assert!(!left.exists());
    assert_eq!(files.roots().await, ["/work/b"]);
}

/// A repository with a commit and a linked worktree.
struct Worktrees {
    _dir: tempfile::TempDir,
    main: PathBuf,
    linked: PathBuf,
    stranger: PathBuf,
}

impl Worktrees {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        let (main, linked, stranger) = (
            canonical.join("main"),
            canonical.join("linked"),
            canonical.join("other"),
        );
        for repo in [&main, &stranger] {
            std::fs::create_dir_all(repo).unwrap();
            git(repo, &["init", "-q", "-b", "main"]);
            std::fs::write(repo.join("a.txt"), "a").unwrap();
            git(repo, &["add", "-A"]);
            git(repo, &["commit", "-q", "-m", "c"]);
        }
        git(
            &main,
            &["worktree", "add", "-q", "-b", "side", linked.to_str().unwrap()],
        );
        Self {
            _dir: dir,
            main,
            linked,
            stranger,
        }
    }

    /// The refs of Vet and of the user.
    fn refs(&self) -> String {
        git(&self.main, &["for-each-ref", "--format=%(refname)"])
    }
}

fn root(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[tokio::test]
async fn the_refs_of_a_removed_project_go_and_those_of_another_worktrees_project_stay() {
    let repo = Worktrees::new();
    for name in [
        "refs/vet/changesets/1/1/tip",
        "refs/vet/changesets/2/1/tip",
        "refs/vet/pr/7/head",
    ] {
        git(&repo.main, &["update-ref", name, "HEAD"]);
    }
    let refs = RepoRefs::new(Budget::new(REVIEW_IO));

    let common = refs.common_dir(root(&repo.main)).await.unwrap();
    assert_eq!(
        refs.common_dir(root(&repo.linked)).await,
        Some(common.clone()),
        "worktrees share it"
    );
    assert_ne!(refs.common_dir(root(&repo.stranger)).await, Some(common));
    assert_eq!(refs.common_dir(root(&repo.main.join("gone"))).await, None);

    refs.delete_refs(root(&repo.linked), vec![ChangesetId(1)], false)
        .await
        .unwrap();
    assert_eq!(
        repo.refs()
            .lines()
            .filter(|l| l.starts_with("refs/vet/"))
            .collect::<Vec<_>>(),
        ["refs/vet/changesets/2/1/tip", "refs/vet/pr/7/head"],
        "the project's changeset refs went; another's and the pull requests' stay"
    );

    refs.delete_refs(root(&repo.main), vec![ChangesetId(2)], true)
        .await
        .unwrap();
    let left = repo.refs();
    assert!(!left.contains("refs/vet/"), "{left}");
    assert!(
        left.contains("refs/heads/main") && left.contains("refs/heads/side"),
        "the user's refs: {left}"
    );
}

#[tokio::test]
async fn refs_cannot_be_deleted_in_a_folder_that_is_no_repository() {
    let dir = tempfile::tempdir().unwrap();
    let refs = RepoRefs::new(Budget::new(REVIEW_IO));
    assert!(
        refs.delete_refs(root(dir.path()), vec![ChangesetId(1)], true)
            .await
            .is_err()
    );
}
