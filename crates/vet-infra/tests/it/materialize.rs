//! Materializing snapshots from real temporary repositories (§4.1, §6.2).
//! `git` is used only to build fixtures.

#![allow(clippy::disallowed_methods)]

use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use crate::common::{self, MemStore};
use tempfile::TempDir;
use vet_app::acquisition::Acquisition;
use vet_app::ports::{Checkout, CheckoutError, History, Scanner, Store, StoredBlob};
use vet_core::content::{BlobId, EntryKind, FileEntry, RepoPath, Snapshot, SnapshotId};
use vet_core::merkle;
use vet_infra::git::GixRepo;
use vet_infra::materialize::Materialize;
use vet_infra::materialize::kept::{DEFAULT_MAX_IDLE, KeptTrees, Referenced};
use vet_infra::materialize::projects::ProjectTrees;
use vet_infra::storage::layout::{ROOT as ROOT_FILE, SOURCES, project_dir};
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{Materializer, NotMaterializable};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
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
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn write(dir: &Path, name: &str, content: &[u8]) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn path(s: &str) -> RepoPath {
    RepoPath(s.as_bytes().to_vec())
}

/// HEAD has a.txt, run.sh (executable), link and dir/b.txt. The worktree edits
/// a.txt and adds an untracked new.txt.
fn fixture() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(dir.path(), "a.txt", b"committed\n");
    write(dir.path(), "dir/b.txt", b"b\n");
    write(dir.path(), "run.sh", b"#!/bin/sh\n");
    std::fs::set_permissions(dir.path().join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("a.txt", dir.path().join("link")).unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "c"]);
    write(dir.path(), "a.txt", b"dirty\n");
    write(dir.path(), "new.txt", b"new\n");
    dir
}

fn read(root: &Path, name: &str) -> Vec<u8> {
    std::fs::read(root.join(name)).unwrap()
}

const POLICY: AnalysisPolicy = AnalysisPolicy::LIMITED;

const ROOT: &str = "/work/repo";

fn project_trees(data: &Path, referenced: &[SnapshotId]) -> ProjectTrees {
    let referenced: HashSet<_> = referenced.iter().copied().collect();
    let referenced: Arc<dyn Referenced> = Arc::new(move || referenced.clone());
    ProjectTrees::for_test(data.to_owned(), DEFAULT_MAX_IDLE, referenced)
}

async fn trees(data: &Path) -> Arc<KeptTrees> {
    project_trees(data, &[]).of(ROOT).await
}

#[tokio::test]
async fn base_and_dirty_snapshots_materialize_from_git_plus_store() {
    let dir = fixture();
    let data = tempfile::tempdir().unwrap();
    let acq = Arc::new(Acquisition::new(GixRepo::for_test(dir.path()), MemStore::default()));
    let opened = common::start_at_head(&acq).await;
    let materializer = Materialize::new(acq.clone(), trees(data.path()).await);
    acq.store.put_snapshot(opened.target.clone(), opened.status.clone());

    // The worktree moves on; the materialized dirty snapshot does not.
    write(dir.path(), "a.txt", b"later\n");

    let target = materializer.materialize(opened.target.id, POLICY).await.unwrap();
    assert_eq!(read(&target.root, "a.txt"), b"dirty\n");
    assert_eq!(read(&target.root, "new.txt"), b"new\n");
    assert_eq!(read(&target.root, "dir/b.txt"), b"b\n");
    let mode = std::fs::metadata(target.root.join("run.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111);
    assert_eq!(
        std::fs::read_link(target.root.join("link")).unwrap(),
        Path::new("a.txt")
    );
    assert!(!target.root.join(".git").exists());

    // The base is the HEAD tree: git alone, not in the store.
    let base = materializer.materialize(opened.base.id, POLICY).await.unwrap();
    assert_eq!(read(&base.root, "a.txt"), b"committed\n");
    assert!(!base.root.join("new.txt").exists());
}

#[tokio::test]
async fn a_binary_held_by_hash_is_materialized_while_the_worktree_still_has_it() {
    let dir = fixture();
    write(dir.path(), "data.bin", b"\0archive");
    let acq = Arc::new(Acquisition::new(GixRepo::for_test(dir.path()), MemStore::default()));
    let opened = common::start_at_head(&acq).await;
    acq.store.put_snapshot(opened.target.clone(), opened.status.clone());
    let blob = BlobId::of(b"\0archive");
    assert_eq!(
        acq.store.blob(blob).await.unwrap(),
        Some(StoredBlob::Binary { size: 8 }),
        "only its hash and size were kept"
    );

    let data = tempfile::tempdir().unwrap();
    let kept = Materialize::new(acq.clone(), trees(data.path()).await)
        .materialize(opened.target.id, POLICY)
        .await
        .unwrap();
    assert_eq!(read(&kept.root, "data.bin"), b"\0archive");

    // The file changed since: git never had it and the worktree no longer does.
    write(dir.path(), "data.bin", b"\0changed!");
    let data = tempfile::tempdir().unwrap();
    let refused = Materialize::new(acq, trees(data.path()).await)
        .materialize(opened.target.id, POLICY)
        .await;
    match refused {
        Err(NotMaterializable::MissingBlobs(blobs)) => assert_eq!(blobs, [blob]),
        other => panic!("{:?}", other.map(|m| m.root)),
    }
}

#[tokio::test]
async fn a_tree_outlives_its_guard_and_a_new_launch_finds_it_without_the_snapshot_being_recorded() {
    let dir = fixture();
    let data = tempfile::tempdir().unwrap();
    let acq = Arc::new(Acquisition::new(GixRepo::for_test(dir.path()), MemStore::default()));
    let opened = common::start_at_head(&acq).await;

    let first = Materialize::new(acq, trees(data.path()).await)
        .materialize(opened.base.id, POLICY)
        .await
        .unwrap();
    let root = first.root.clone();
    drop(first.guard);
    assert_eq!(read(&root, "a.txt"), b"committed\n");

    // A new launch: its acquisition knows nothing of the snapshot, and git is not asked.
    let restarted = Arc::new(Acquisition::new(GixRepo::for_test(dir.path()), MemStore::default()));
    let again = Materialize::new(restarted, trees(data.path()).await)
        .materialize(opened.base.id, POLICY)
        .await
        .unwrap();
    assert_eq!(again.root, root);

    // The policy is part of the key.
    let executing = Materialize::new(
        Arc::new(Acquisition::new(GixRepo::for_test(dir.path()), MemStore::default())),
        trees(data.path()).await,
    )
    .materialize(opened.base.id, AnalysisPolicy::EXECUTING)
    .await;
    assert!(matches!(executing, Err(NotMaterializable::Other(_))));
}

#[tokio::test]
async fn policies_that_restore_alike_share_a_tree_and_an_executing_restore_has_its_own() {
    let dir = fixture();
    let data = tempfile::tempdir().unwrap();
    let acq = Arc::new(Acquisition::new(GixRepo::for_test(dir.path()), MemStore::default()));
    let opened = common::start_at_head(&acq).await;
    let materializer = Materialize::new(acq, trees(data.path()).await);
    let root = async |policy| materializer.materialize(opened.base.id, policy).await.unwrap().root;

    let limited = root(AnalysisPolicy::LIMITED).await;
    assert_eq!(root(AnalysisPolicy::SERVERS).await, limited);
    assert_ne!(root(AnalysisPolicy::EXECUTING).await, limited);
}

#[tokio::test]
async fn trees_live_in_their_projects_directory_and_each_project_has_its_own() {
    let dir = fixture();
    let data = tempfile::tempdir().unwrap();
    let acq = Arc::new(Acquisition::new(GixRepo::for_test(dir.path()), MemStore::default()));
    let opened = common::start_at_head(&acq).await;
    let projects = project_trees(data.path(), &[]);
    let materialize = async |root| {
        Materialize::new(acq.clone(), projects.of(root).await)
            .materialize(opened.base.id, POLICY)
            .await
            .unwrap()
            .root
    };

    let first = materialize(ROOT).await;
    let second = materialize("/work/other").await;

    let sources = project_dir(data.path(), ROOT).join(SOURCES);
    assert!(first.starts_with(&sources), "{first:?}");
    assert!(second.starts_with(project_dir(data.path(), "/work/other").join(SOURCES)));
    assert_ne!(first, second, "the same snapshot is a tree in each project");
    assert_eq!(
        std::fs::read_to_string(project_dir(data.path(), ROOT).join(ROOT_FILE)).unwrap(),
        ROOT
    );
}

#[tokio::test]
async fn unknown_snapshot_is_refused() {
    let dir = fixture();
    let data = tempfile::tempdir().unwrap();
    let acq = Acquisition::new(GixRepo::for_test(dir.path()), MemStore::default());
    let result = Materialize::new(Arc::new(acq), trees(data.path()).await)
        .materialize(SnapshotId(vet_core::content::Hash([9; 32])), POLICY)
        .await;
    assert!(matches!(result, Err(NotMaterializable::Other(_))));
}

#[tokio::test]
async fn snapshot_with_unavailable_blobs_is_refused_naming_them() {
    let dir = fixture();
    // Not pinned: the dirty blobs were never copied into the store.
    let acq = Arc::new(Acquisition::new(GixRepo::for_test(dir.path()), MemStore::default()));
    let scan = acq.git.scan(vet_app::ports::Dirty::All).await.unwrap();
    acq.store.put_snapshot(scan.snapshot.clone(), scan.status);
    acq.record_commit(scan.snapshot.id, acq.git.head().await.unwrap());
    // Dirty content is gone from the worktree too, so nothing can reconstruct it.
    let gone = [BlobId::of(b"dirty\n"), BlobId::of(b"new\n")];
    let data = tempfile::tempdir().unwrap();
    match Materialize::new(acq, trees(data.path()).await)
        .materialize(scan.snapshot.id, POLICY)
        .await
    {
        Err(NotMaterializable::MissingBlobs(mut blobs)) => {
            blobs.sort();
            let mut want = gone.to_vec();
            want.sort();
            assert_eq!(blobs, want);
        }
        other => panic!("{:?}", other.map(|m| m.root)),
    }
}

#[tokio::test]
async fn checkout_refuses_paths_that_escape_the_root() {
    let dir = fixture();
    let repo = GixRepo::for_test(dir.path());
    let head = repo.head().await.unwrap();
    for bad in ["../escape", "a/../../escape", ".git/hooks/pre-commit", "x//y"] {
        let blob = BlobId::of(b"x");
        let store = MemStore::default();
        store.put_blobs(vec![b"x".to_vec()]).await.unwrap();
        let entries = [(
            path(bad),
            FileEntry {
                blob,
                mode: 0o100644,
                kind: EntryKind::File,
                unfiltered: false,
            },
        )]
        .into_iter()
        .collect();
        let snapshot = Snapshot {
            id: merkle::snapshot_id(&entries),
            entries,
        };
        let out = TempDir::new().unwrap();
        let result = repo.checkout(&store, &head, &snapshot, out.path()).await;
        assert!(matches!(result, Err(CheckoutError::Git(_))), "{bad}");
        assert!(std::fs::read_dir(out.path()).unwrap().next().is_none(), "{bad}");
    }
}

#[tokio::test]
async fn checkout_never_runs_filter_drivers() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    let marker = dir.path().join("marker");
    write(dir.path(), ".gitattributes", b"*.dat filter=probe\n");
    write(dir.path(), "a.dat", b"clean\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "c"]);
    git(
        dir.path(),
        &[
            "config",
            "filter.probe.smudge",
            &format!("touch {}; cat", marker.display()),
        ],
    );
    let repo = GixRepo::for_test(dir.path());
    let head = repo.head().await.unwrap();
    let snapshot = repo.commit_snapshot(&head).await.unwrap();
    let out = TempDir::new().unwrap();
    repo.checkout(&MemStore::default(), &head, &snapshot, out.path())
        .await
        .unwrap();
    assert_eq!(read(out.path(), "a.dat"), b"clean\n");
    assert!(!marker.exists());
}
