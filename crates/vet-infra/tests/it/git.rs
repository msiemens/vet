//! Acquisition and line diff against real temporary repositories. `git` is used
//! only to build fixtures.

#![allow(clippy::disallowed_methods)]

use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use crate::common;
use tempfile::TempDir;
use vet_app::acquisition::Acquisition;
use vet_app::changeset::ChangesetId;
use vet_app::ports::{
    BlobRead, BlobReader, ChangesetRefs, Differ, Dirty, History, IgnoreRules, Scanner, Store, StoreError, TextDiff,
};
use vet_core::content::{BlobId, EntryKind, NonTextKind, RepoPath, Snapshot, SnapshotId, SnapshotStatus};
use vet_infra::budget::Budget;
use vet_infra::git::GixRepo;

const EMPTY_BLOB: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";

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

fn repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    dir
}

fn write(dir: &Path, name: &str, content: &[u8]) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// Sets the modification time of `name` back, past the window in which a stat cannot vouch for content.
fn age(dir: &Path, name: &str, secs: u64) {
    let file = std::fs::File::options().write(true).open(dir.join(name)).unwrap();
    file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(secs))
        .unwrap();
}

fn commit(dir: &Path) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "c"]);
}

fn path(s: &str) -> RepoPath {
    RepoPath(s.as_bytes().to_vec())
}

fn blob(bytes: &[u8]) -> BlobId {
    BlobId::of(bytes)
}

async fn head_and_scan(dir: &Path, repo: &GixRepo) -> (Snapshot, vet_app::ports::Scan) {
    let head = repo.head().await.unwrap();
    let base = repo.commit_snapshot(&head).await.unwrap();
    let _ = dir;
    (base, repo.scan(Dirty::All).await.unwrap())
}

#[derive(Default)]
struct MemStore(Mutex<HashMap<BlobId, Vec<u8>>>);

impl Store for MemStore {
    async fn put_blobs(&self, blobs: Vec<Vec<u8>>) -> Result<Vec<BlobId>, StoreError> {
        let mut held = self.0.lock().unwrap();
        Ok(blobs
            .into_iter()
            .map(|bytes| {
                let id = blob(&bytes);
                held.insert(id, bytes);
                id
            })
            .collect())
    }
    async fn missing_blobs(&self, ids: Vec<BlobId>) -> Result<Vec<BlobId>, StoreError> {
        let held = self.0.lock().unwrap();
        Ok(ids.into_iter().filter(|id| !held.contains_key(id)).collect())
    }
    async fn blob(&self, id: BlobId) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.0.lock().unwrap().get(&id).cloned())
    }
    async fn blobs(&self, ids: Vec<BlobId>) -> Result<BTreeMap<BlobId, Vec<u8>>, StoreError> {
        let held = self.0.lock().unwrap();
        Ok(ids
            .into_iter()
            .filter_map(|id| Some((id, held.get(&id)?.clone())))
            .collect())
    }
    async fn snapshot(&self, _: SnapshotId) -> Result<Option<(Snapshot, SnapshotStatus)>, StoreError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn crlf_worktree_matches_head_under_autocrlf() {
    let dir = repo();
    write(dir.path(), "a.txt", b"one\ntwo\n");
    commit(dir.path());
    git(dir.path(), &["config", "core.autocrlf", "true"]);
    write(dir.path(), "a.txt", b"one\r\ntwo\r\n");
    let repo = GixRepo::new(dir.path());
    let (base, scan) = head_and_scan(dir.path(), &repo).await;
    assert_eq!(
        scan.snapshot.entries[&path("a.txt")].blob,
        base.entries[&path("a.txt")].blob
    );
    assert_eq!(scan.snapshot.id, base.id);
}

#[tokio::test]
async fn crlf_without_conversion_is_a_change() {
    let dir = repo();
    // gix honours the user's global config like git does, so pin the setting
    // this test is about instead of inheriting e.g. `core.autocrlf=input`.
    git(dir.path(), &["config", "core.autocrlf", "false"]);
    write(dir.path(), "a.txt", b"one\ntwo\n");
    commit(dir.path());
    write(dir.path(), "a.txt", b"one\r\ntwo\r\n");
    let repo = GixRepo::new(dir.path());
    let (base, scan) = head_and_scan(dir.path(), &repo).await;
    assert_ne!(scan.snapshot.id, base.id);
}

#[tokio::test]
async fn mode_comes_from_the_index_when_filemode_is_off() {
    let dir = repo();
    write(dir.path(), "run.sh", b"#!/bin/sh\n");
    write(dir.path(), "plain.txt", b"x\n");
    std::fs::set_permissions(dir.path().join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    commit(dir.path());
    git(dir.path(), &["config", "core.filemode", "false"]);
    std::fs::set_permissions(dir.path().join("run.sh"), std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(dir.path().join("plain.txt"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let repo = GixRepo::new(dir.path());
    let (base, scan) = head_and_scan(dir.path(), &repo).await;
    assert_eq!(scan.snapshot.entries[&path("run.sh")].mode, 0o100755);
    assert_eq!(scan.snapshot.entries[&path("plain.txt")].mode, 0o100644);
    assert_eq!(scan.snapshot.id, base.id);
}

#[tokio::test]
async fn symlink_stays_a_symlink_when_symlinks_are_off() {
    let dir = repo();
    write(dir.path(), "target.txt", b"x\n");
    std::os::unix::fs::symlink("target.txt", dir.path().join("link")).unwrap();
    commit(dir.path());
    git(dir.path(), &["config", "core.symlinks", "false"]);
    std::fs::remove_file(dir.path().join("link")).unwrap();
    write(dir.path(), "link", b"target.txt");
    let repo = GixRepo::new(dir.path());
    let (base, scan) = head_and_scan(dir.path(), &repo).await;
    let entry = &scan.snapshot.entries[&path("link")];
    assert_eq!((entry.kind, entry.mode), (EntryKind::Symlink, 0o120000));
    assert_eq!(scan.snapshot.id, base.id);
}

#[tokio::test]
async fn case_only_differences_are_flagged() {
    let dir = repo();
    // A case-insensitive filesystem keeps one of the two files; the index keeps
    // both, as after checking out a commit made elsewhere.
    write(dir.path(), "Foo.txt", b"1\n");
    write(dir.path(), "foo.txt", b"2\n");
    write(dir.path(), "other.txt", b"3\n");
    for name in ["Foo.txt", "foo.txt"] {
        let entry = format!("100644,{EMPTY_BLOB},{name}");
        git(
            dir.path(),
            &["update-index", "--add", "--info-only", "--cacheinfo", &entry],
        );
    }
    let repo = GixRepo::new(dir.path());
    let scan = repo.scan(Dirty::All).await.unwrap();
    assert_eq!(scan.status.path_collisions, vec![path("Foo.txt"), path("foo.txt")]);
}

#[tokio::test]
async fn untracked_files_are_included_and_ignored_ones_are_not() {
    let dir = repo();
    write(dir.path(), ".gitignore", b"ignored.txt\ntarget/\n");
    write(dir.path(), "tracked.txt", b"t\n");
    commit(dir.path());
    write(dir.path(), "untracked.txt", b"u\n");
    write(dir.path(), "ignored.txt", b"i\n");
    write(dir.path(), "target/out.bin", b"o\n");
    let repo = GixRepo::new(dir.path());
    let scan = repo.scan(Dirty::All).await.unwrap();
    let paths: Vec<_> = scan.snapshot.entries.keys().cloned().collect();
    assert_eq!(
        paths,
        vec![path(".gitignore"), path("tracked.txt"), path("untracked.txt")]
    );
    assert_eq!(scan.snapshot.entries[&path("untracked.txt")].mode, 0o100644);
}

#[tokio::test]
async fn custom_filter_paths_are_raw_and_never_run() {
    let dir = repo();
    let marker = dir.path().join("marker");
    write(dir.path(), ".gitattributes", b"*.dat filter=probe\n");
    write(dir.path(), "a.dat", b"clean\n");
    commit(dir.path());
    git(
        dir.path(),
        &[
            "config",
            "filter.probe.clean",
            &format!("touch {}; cat", marker.display()),
        ],
    );
    write(dir.path(), "a.dat", b"edited\n");
    let repo = GixRepo::new(dir.path());
    let scan = repo.scan(Dirty::All).await.unwrap();
    let entry = &scan.snapshot.entries[&path("a.dat")];
    assert!(entry.unfiltered);
    assert_eq!(entry.blob, blob(b"edited\n"));
    assert!(!marker.exists());
}

#[tokio::test]
async fn files_over_the_cap_are_hashed_but_not_retained() {
    let dir = repo();
    write(dir.path(), "small.txt", b"s\n");
    commit(dir.path());
    let big = vec![b'x'; 100];
    write(dir.path(), "big.txt", &big);
    let repo = GixRepo::new(dir.path()).with_cap(10);
    let acq = Acquisition::new(repo, MemStore::default());
    let opened = common::start_at_head(&acq).await;
    assert_eq!(opened.status.content_unavailable, vec![path("big.txt")]);
    assert_eq!(opened.target.entries[&path("big.txt")].blob, blob(&big));
    assert!(acq.store.blob(blob(&big)).await.unwrap().is_none());

    let diff = acq
        .git
        .diff(&acq.store, &opened.base, &opened.target)
        .await
        .unwrap()
        .diff;
    assert_eq!(diff.non_text.len(), 1);
    assert_eq!(diff.non_text[0].kind, NonTextKind::ContentUnavailable);
}

#[tokio::test]
async fn capped_reads_refuse_large_tracked_blobs_and_large_worktree_files() {
    let dir = repo();
    let big = vec![b'x'; 100];
    write(dir.path(), "small.txt", b"s\n");
    write(dir.path(), "big.txt", &big);
    commit(dir.path());
    write(dir.path(), "untracked.txt", &[b'y'; 100]);
    let repo = GixRepo::new(dir.path()).with_cap(10);
    // The commit's tree maps its blobs to git objects.
    let tree = repo.commit_snapshot(&repo.head().await.unwrap()).await.unwrap();
    let scan = repo.scan(Dirty::All).await.unwrap();

    let read = async |name: &str| {
        let p = path(name);
        let entries = if name == "untracked.txt" { &scan.snapshot } else { &tree }
            .entries
            .get(&p)
            .unwrap();
        repo.read_blob_capped(&p, entries.blob).await.unwrap()
    };
    assert_eq!(read("small.txt").await, BlobRead::Found(b"s\n".to_vec()));
    assert_eq!(read("big.txt").await, BlobRead::TooLarge);
    assert_eq!(read("untracked.txt").await, BlobRead::Missing);
}

#[tokio::test]
async fn pin_copies_dirty_content_and_diff_survives_later_edits() {
    let dir = repo();
    write(dir.path(), "a.txt", b"1\n2\n3\n");
    write(dir.path(), "same.txt", b"same\n");
    commit(dir.path());
    write(dir.path(), "a.txt", b"1\nTWO\n3\n");
    write(dir.path(), "new.txt", b"n\n");
    let acq = Acquisition::new(GixRepo::new(dir.path()), MemStore::default());
    let opened = common::start_at_head(&acq).await;
    assert_eq!(opened.head, acq.git.head().await.unwrap());
    assert!(acq.store.blob(blob(b"1\nTWO\n3\n")).await.unwrap().is_some());
    assert!(acq.store.blob(blob(b"n\n")).await.unwrap().is_some());
    assert!(acq.store.blob(blob(b"same\n")).await.unwrap().is_none());

    // The worktree moves on; the pinned content still diffs.
    write(dir.path(), "a.txt", b"unrelated\n");
    let TextDiff { diff, texts } = acq.git.diff(&acq.store, &opened.base, &opened.target).await.unwrap();
    assert_eq!(diff.files.len(), 2);
    assert_eq!(
        texts.get(&blob(b"1\nTWO\n3\n")).map(Vec::as_slice),
        Some(&b"1\nTWO\n3\n"[..]),
        "the diff comes with the content it compared, as it was pinned"
    );
    let a = &diff.files[0];
    assert_eq!(a.new_path, Some(path("a.txt")));
    assert_eq!((a.hunks[0].old.start, a.hunks[0].old.end), (1, 2));
    assert_eq!((a.hunks[0].new.start, a.hunks[0].new.end), (1, 2));
    let added = &diff.files[1];
    assert_eq!((added.old_path.clone(), added.hunks[0].new.end), (None, 1));
}

#[tokio::test]
async fn renames_are_detected_with_their_edits() {
    let dir = repo();
    let body: String = (0..20).map(|i| format!("line {i}\n")).collect();
    write(dir.path(), "old.txt", body.as_bytes());
    write(dir.path(), "pure.txt", b"pure rename content\nmore\n");
    commit(dir.path());
    std::fs::remove_file(dir.path().join("old.txt")).unwrap();
    std::fs::remove_file(dir.path().join("pure.txt")).unwrap();
    write(
        dir.path(),
        "dir/new.txt",
        body.replace("line 5\n", "line five\n").as_bytes(),
    );
    write(dir.path(), "dir/pure2.txt", b"pure rename content\nmore\n");
    let repo = GixRepo::new(dir.path());
    let (base, scan) = head_and_scan(dir.path(), &repo).await;
    let diff = repo
        .diff(&MemStore::default(), &base, &scan.snapshot)
        .await
        .unwrap()
        .diff;
    assert_eq!(diff.renames.0.get(&path("old.txt")), Some(&path("dir/new.txt")));
    assert_eq!(diff.renames.0.get(&path("pure.txt")), Some(&path("dir/pure2.txt")));
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].old_path, Some(path("old.txt")));
    assert_eq!(diff.files[0].hunks.len(), 1);
    assert_eq!(diff.non_text.len(), 1);
    assert_eq!(diff.non_text[0].kind, NonTextKind::Rename);
    assert_eq!(diff.non_text[0].from, Some(path("pure.txt")));
}

#[tokio::test]
async fn binary_mode_and_symlink_changes_are_non_text_items() {
    let dir = repo();
    write(dir.path(), "img.bin", b"\0\x01\x02");
    write(dir.path(), "run.sh", b"#!/bin/sh\n");
    std::os::unix::fs::symlink("a", dir.path().join("link")).unwrap();
    commit(dir.path());
    write(dir.path(), "img.bin", b"\0\x01\x03");
    std::fs::set_permissions(dir.path().join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::remove_file(dir.path().join("link")).unwrap();
    std::os::unix::fs::symlink("b", dir.path().join("link")).unwrap();
    let repo = GixRepo::new(dir.path());
    let (base, scan) = head_and_scan(dir.path(), &repo).await;
    let diff = repo
        .diff(&MemStore::default(), &base, &scan.snapshot)
        .await
        .unwrap()
        .diff;
    assert!(diff.files.is_empty());
    let kinds: Vec<_> = diff.non_text.iter().map(|i| (i.path.clone(), i.kind)).collect();
    assert_eq!(
        kinds,
        vec![
            (path("img.bin"), NonTextKind::Binary),
            (path("link"), NonTextKind::Symlink),
            (path("run.sh"), NonTextKind::Mode)
        ]
    );
}

#[tokio::test]
async fn added_and_deleted_empty_files_are_items_and_text_to_empty_stays_a_text_diff() {
    let dir = repo();
    write(dir.path(), "gone.txt", b"");
    write(dir.path(), "emptied.txt", b"a\nb\n");
    write(dir.path(), "filled.txt", b"");
    commit(dir.path());
    std::fs::remove_file(dir.path().join("gone.txt")).unwrap();
    write(dir.path(), "new.txt", b"");
    write(dir.path(), "emptied.txt", b"");
    write(dir.path(), "filled.txt", b"x\n");
    let repo = GixRepo::new(dir.path());
    let (base, scan) = head_and_scan(dir.path(), &repo).await;
    let diff = repo
        .diff(&MemStore::default(), &base, &scan.snapshot)
        .await
        .unwrap()
        .diff;

    let items: Vec<_> = diff
        .non_text
        .iter()
        .map(|i| (i.path.clone(), i.kind, i.before.is_some(), i.after.is_some()))
        .collect();
    assert_eq!(
        items,
        vec![
            (path("gone.txt"), NonTextKind::Empty, true, false),
            (path("new.txt"), NonTextKind::Empty, false, true)
        ]
    );
    let files: Vec<_> = diff.files.iter().map(|f| (f.new_path.clone(), f.hunks.len())).collect();
    assert_eq!(
        files,
        vec![(Some(path("emptied.txt")), 1), (Some(path("filled.txt")), 1)]
    );
}

#[tokio::test]
async fn unchanged_tree_has_the_head_snapshot_id() {
    let dir = repo();
    write(dir.path(), "a/b.txt", b"x\n");
    write(dir.path(), "c.txt", b"y\n");
    commit(dir.path());
    let repo = GixRepo::new(dir.path());
    let (base, scan) = head_and_scan(dir.path(), &repo).await;
    assert_eq!(scan.snapshot, base);
}

fn head_of(dir: &Path) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[tokio::test]
async fn submodule_commits_come_from_the_index_and_the_checked_out_head() {
    let sub = repo();
    write(sub.path(), "s.txt", b"s\n");
    commit(sub.path());
    let dir = repo();
    write(dir.path(), "a.txt", b"a\n");
    commit(dir.path());
    let url = sub.path().to_str().unwrap();
    git(
        dir.path(),
        &["-c", "protocol.file.allow=always", "submodule", "add", "-q", url, "lib"],
    );
    commit(dir.path());
    let pinned = head_of(&dir.path().join("lib"));
    let repo = GixRepo::new(dir.path());

    let (base, scan) = head_and_scan(dir.path(), &repo).await;
    let entry = |s: &Snapshot| s.entries[&path("lib")].clone();
    assert_eq!(entry(&base).kind, EntryKind::Submodule);
    assert_eq!(
        repo.submodule_commit(entry(&base).blob).await.unwrap(),
        Some(pinned.clone())
    );
    assert_eq!(scan.snapshot.id, base.id);

    // The submodule moves on inside its worktree.
    write(&dir.path().join("lib"), "s.txt", b"t\n");
    commit(&dir.path().join("lib"));
    let moved = head_of(&dir.path().join("lib"));
    let scan = repo.scan(Dirty::All).await.unwrap();
    assert_ne!(scan.snapshot.id, base.id);
    assert_eq!(
        repo.submodule_commit(entry(&scan.snapshot).blob).await.unwrap(),
        Some(moved)
    );
}

#[tokio::test]
async fn changeset_refs_are_set_per_version_and_deleted_with_the_changeset() {
    let dir = repo();
    write(dir.path(), "a.txt", b"a\n");
    commit(dir.path());
    let head = head_of(dir.path());
    let repo = GixRepo::new(dir.path());
    let commit_id = repo.head().await.unwrap();
    let refs = |dir: &Path| {
        let out = Command::new("git")
            .current_dir(dir)
            .args(["for-each-ref", "--format=%(refname) %(objectname)", "refs/vet"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };

    let set = |changeset, version, name| repo.set_changeset_ref(ChangesetId(changeset), version, name, &commit_id);
    set(7, 1, "base").await.unwrap();
    set(7, 1, "tip").await.unwrap();
    set(7, 2, "base").await.unwrap();
    set(8, 1, "base").await.unwrap();
    assert_eq!(
        refs(dir.path()),
        format!(
            "refs/vet/changesets/7/1/base {head}\nrefs/vet/changesets/7/1/tip {head}\n\
             refs/vet/changesets/7/2/base {head}\nrefs/vet/changesets/8/1/base {head}\n"
        )
    );

    repo.delete_changeset_refs(ChangesetId(7)).await.unwrap();
    assert_eq!(refs(dir.path()), format!("refs/vet/changesets/8/1/base {head}\n"));
}

#[tokio::test]
async fn tool_stores_are_neither_scanned_nor_watched() {
    let dir = repo();
    write(dir.path(), "tracked.txt", b"t\n");
    commit(dir.path());
    write(dir.path(), ".pnpm-store/v11/files/ab/cd", b"x\n");
    write(dir.path(), "pkg/.pnpm-store/v11/x", b"x\n");
    write(dir.path(), "untracked.txt", b"u\n");
    let repo = GixRepo::new(dir.path());

    let scan = repo.scan(Dirty::All).await.unwrap();
    let paths: Vec<_> = scan.snapshot.entries.keys().cloned().collect();
    assert_eq!(paths, vec![path("tracked.txt"), path("untracked.txt")]);

    let watched = [".pnpm-store", ".pnpm-store/v11/x", "pkg/.pnpm-store/y", "untracked.txt"].map(path);
    let kept = repo.unignored(watched.into()).await.unwrap();
    assert_eq!(kept, vec![path("untracked.txt")]);
}

#[tokio::test]
async fn ignore_rules_drop_ignored_paths_but_never_tracked_ones() {
    let dir = repo();
    write(dir.path(), ".gitignore", b"target/\n*.log\n");
    write(dir.path(), "src/a.rs", b"a\n");
    write(dir.path(), "target/keep.txt", b"forced\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["add", "-f", "target/keep.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "c"]);
    write(dir.path(), "target/debug/x", b"x\n");
    let repo = GixRepo::new(dir.path());

    let paths: Vec<_> = [
        "src/a.rs",
        "target",
        "target/debug/x",
        "target/keep.txt",
        "build.log",
        "src/new.rs",
        ".gitignore",
    ]
    .map(path)
    .into();
    let kept = repo.unignored(paths).await.unwrap();

    assert_eq!(
        kept,
        ["src/a.rs", "target", "target/keep.txt", "src/new.rs", ".gitignore"].map(path)
    );
}

#[tokio::test]
async fn repeated_scans_follow_edits_through_the_stat_cache() {
    let dir = repo();
    write(dir.path(), "tracked.txt", b"t\n");
    commit(dir.path());
    write(dir.path(), "new.txt", b"aaaa\n");
    write(dir.path(), "tracked.txt", b"u\n");
    // Older than the second the scan reads them in, so the next scans may reuse the records.
    age(dir.path(), "new.txt", 20);
    age(dir.path(), "tracked.txt", 20);
    let repo = GixRepo::new(dir.path());
    let first = repo.scan(Dirty::All).await.unwrap().snapshot;
    assert_eq!(first, repo.scan(Dirty::All).await.unwrap().snapshot);
    assert_eq!(first, repo.scan(Dirty::All).await.unwrap().snapshot);

    write(dir.path(), "new.txt", b"bbbb\n");
    write(dir.path(), "tracked.txt", b"v\n");
    // Edits of the same size, as old as the files were: only the stat tells them apart.
    age(dir.path(), "new.txt", 10);
    age(dir.path(), "tracked.txt", 10);
    let last = repo.scan(Dirty::All).await.unwrap().snapshot;
    assert_eq!(last.entries[&path("new.txt")].blob, blob(b"bbbb\n"));
    assert_eq!(last.entries[&path("tracked.txt")].blob, blob(b"v\n"));
}

#[tokio::test]
async fn a_read_waits_only_for_the_budget_of_the_handle_it_is_made_through() {
    let dir = repo();
    write(dir.path(), "a.txt", b"one\n");
    commit(dir.path());
    let (review, semantic) = (Budget::new(2), Budget::new(2));
    let on_review = GixRepo::new(dir.path()).budgeted(review.clone());
    let on_semantic = on_review.budgeted(semantic.clone());
    let read = |repo: &GixRepo| {
        let repo = repo.clone();
        async move { repo.read_blob(&path("a.txt"), blob(b"one\n")).await }
    };

    let held = semantic.acquire_all().await;
    let answered = tokio::time::timeout(Duration::from_secs(30), read(&on_review)).await;
    assert_eq!(
        answered.expect("the review's read waited").unwrap().as_deref(),
        Some(&b"one\n"[..])
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), read(&on_semantic))
            .await
            .is_err(),
        "the semantic read took no unit of its own budget"
    );
    drop(held);
    assert_eq!(read(&on_semantic).await.unwrap().as_deref(), Some(&b"one\n"[..]));
}
