//! Materializing snapshots from real temporary repositories (§4.1, §6.2).
//! `git` is used only to build fixtures.

#![allow(clippy::disallowed_methods)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use crate::common;
use tempfile::TempDir;
use vet_app::acquisition::Acquisition;
use vet_app::ports::{Checkout, CheckoutError, History, Scanner, Store, StoreError};
use vet_core::content::{BlobId, EntryKind, FileEntry, RepoPath, Snapshot, SnapshotId, SnapshotStatus};
use vet_core::merkle;
use vet_infra::git::GixRepo;
use vet_infra::materialize::Materialize;
use vet_infra::materialize::kept::{KeptTrees, Limits, Referenced};
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

#[derive(Default)]
struct MemStore {
    blobs: Mutex<HashMap<BlobId, Vec<u8>>>,
    snapshots: Mutex<HashMap<SnapshotId, (Snapshot, SnapshotStatus)>>,
}

impl Store for MemStore {
    async fn put_blobs(&self, blobs: Vec<Vec<u8>>) -> Result<Vec<BlobId>, StoreError> {
        let mut held = self.blobs.lock().unwrap();
        Ok(blobs
            .into_iter()
            .map(|bytes| {
                let id = BlobId::of(&bytes);
                held.insert(id, bytes);
                id
            })
            .collect())
    }
    async fn missing_blobs(&self, ids: Vec<BlobId>) -> Result<Vec<BlobId>, StoreError> {
        let held = self.blobs.lock().unwrap();
        Ok(ids.into_iter().filter(|id| !held.contains_key(id)).collect())
    }
    async fn blob(&self, id: BlobId) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.blobs.lock().unwrap().get(&id).cloned())
    }
    async fn blobs(&self, ids: Vec<BlobId>) -> Result<BTreeMap<BlobId, Vec<u8>>, StoreError> {
        let held = self.blobs.lock().unwrap();
        Ok(ids
            .into_iter()
            .filter_map(|id| Some((id, held.get(&id)?.clone())))
            .collect())
    }
    async fn snapshot(&self, id: SnapshotId) -> Result<Option<(Snapshot, SnapshotStatus)>, StoreError> {
        Ok(self.snapshots.lock().unwrap().get(&id).cloned())
    }
}

impl MemStore {
    /// What a review does to the store when it keeps a snapshot.
    fn put_snapshot(&self, snapshot: Snapshot, status: SnapshotStatus) {
        self.snapshots.lock().unwrap().insert(snapshot.id, (snapshot, status));
    }
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

fn trees(data: &Path) -> Arc<KeptTrees> {
    let none: Arc<dyn Referenced> = Arc::new(HashSet::<SnapshotId>::new);
    Arc::new(KeptTrees::for_test(data.join("materialized"), Limits::default(), none))
}

#[tokio::test]
async fn base_and_dirty_snapshots_materialize_from_git_plus_store() {
    let dir = fixture();
    let data = tempfile::tempdir().unwrap();
    let acq = Arc::new(Acquisition::new(GixRepo::for_test(dir.path()), MemStore::default()));
    let opened = common::start_at_head(&acq).await;
    let materializer = Materialize::new(acq.clone(), trees(data.path()));
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
async fn a_tree_outlives_its_guard_and_a_new_launch_finds_it_without_the_snapshot_being_recorded() {
    let dir = fixture();
    let data = tempfile::tempdir().unwrap();
    let acq = Arc::new(Acquisition::new(GixRepo::for_test(dir.path()), MemStore::default()));
    let opened = common::start_at_head(&acq).await;

    let first = Materialize::new(acq, trees(data.path()))
        .materialize(opened.base.id, POLICY)
        .await
        .unwrap();
    let root = first.root.clone();
    drop(first.guard);
    assert_eq!(read(&root, "a.txt"), b"committed\n");

    // A new launch: its acquisition knows nothing of the snapshot, and git is not asked.
    let restarted = Arc::new(Acquisition::new(GixRepo::for_test(dir.path()), MemStore::default()));
    let again = Materialize::new(restarted, trees(data.path()))
        .materialize(opened.base.id, POLICY)
        .await
        .unwrap();
    assert_eq!(again.root, root);

    // The policy is part of the key.
    let executing = Materialize::new(
        Arc::new(Acquisition::new(GixRepo::for_test(dir.path()), MemStore::default())),
        trees(data.path()),
    )
    .materialize(opened.base.id, AnalysisPolicy::EXECUTING)
    .await;
    assert!(matches!(executing, Err(NotMaterializable::Other(_))));
}

#[tokio::test]
async fn unknown_snapshot_is_refused() {
    let dir = fixture();
    let data = tempfile::tempdir().unwrap();
    let acq = Acquisition::new(GixRepo::for_test(dir.path()), MemStore::default());
    let result = Materialize::new(Arc::new(acq), trees(data.path()))
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
    match Materialize::new(acq, trees(data.path()))
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
