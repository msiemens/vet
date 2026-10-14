//! Repositories at the edges: no commit yet, a detached HEAD, remote-tracking branches, commit
//! messages with CRLF, paths that are not UTF-8, and trees that are not repositories.
//! `git` is used only to build the fixtures.

#![allow(clippy::disallowed_methods)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use vet_app::changeset::CommitId;
use vet_app::ports::{BlobReader, Differ, Dirty, History, Scanner, Store, StoreError};
use vet_core::content::{BlobId, NonTextKind, RepoPath, Snapshot, SnapshotId, SnapshotStatus};
use vet_infra::budget::Budget;
use vet_infra::git::{GixRepo, find_root, list_files};

fn git_os(dir: &Path, args: &[&OsStr]) -> String {
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

fn git(dir: &Path, args: &[&str]) -> String {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    git_os(dir, &args)
}

fn repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    dir
}

fn commit(dir: &Path, file: &str, message: &str) -> CommitId {
    std::fs::write(dir.join(file), message).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    CommitId(git(dir, &["rev-parse", "HEAD"]))
}

fn path(s: &str) -> RepoPath {
    s.into()
}

/// A content store that holds nothing: the diff reads from git.
struct NoStore;

impl Store for NoStore {
    async fn put_blobs(&self, _: Vec<Vec<u8>>) -> Result<Vec<BlobId>, StoreError> {
        unimplemented!()
    }
    async fn missing_blobs(&self, ids: Vec<BlobId>) -> Result<Vec<BlobId>, StoreError> {
        Ok(ids)
    }
    async fn blob(&self, _: BlobId) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(None)
    }
    async fn blobs(&self, _: Vec<BlobId>) -> Result<BTreeMap<BlobId, Vec<u8>>, StoreError> {
        Ok(BTreeMap::new())
    }
    async fn snapshot(&self, _: SnapshotId) -> Result<Option<(Snapshot, SnapshotStatus)>, StoreError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn a_repository_without_a_commit_has_a_tree_to_scan_and_no_history() {
    let dir = repo();
    std::fs::write(dir.path().join("new.txt"), "n\n").unwrap();
    let repo = GixRepo::new(dir.path());

    let scan = repo.scan(Dirty::All).await.unwrap();
    assert_eq!(scan.snapshot.entries.keys().collect::<Vec<_>>(), [&path("new.txt")]);

    assert!(repo.head().await.is_err());
    assert!(repo.branches().await.unwrap().is_empty());
    assert_eq!(repo.default_reference().await.unwrap(), None);
    assert_eq!(repo.resolve_commit("HEAD").await.unwrap(), None);
}

#[tokio::test]
async fn a_repository_without_a_commit_or_a_file_scans_to_an_empty_snapshot() {
    let dir = repo();

    let scan = GixRepo::new(dir.path()).scan(Dirty::All).await.unwrap();

    assert!(scan.snapshot.entries.is_empty());
}

#[tokio::test]
async fn a_detached_head_is_no_branch_and_still_has_a_history() {
    let dir = repo();
    let p = dir.path();
    commit(p, "a.txt", "one");
    let two = commit(p, "a.txt", "two");
    git(p, &["checkout", "-q", "--detach"]);
    let repo = GixRepo::new(p);

    assert_eq!(repo.head().await.unwrap(), two);
    let branches = repo.branches().await.unwrap();
    let branches: Vec<_> = branches.iter().map(|b| (b.name.as_str(), b.current)).collect();
    assert_eq!(branches, [("main", false)]);
}

#[tokio::test]
async fn a_branch_is_found_local_or_remote_tracking_and_otherwise_named_in_the_error() {
    let dir = repo();
    let p = dir.path();
    let m1 = commit(p, "a.txt", "m1");
    git(p, &["branch", "feature"]);
    let m2 = commit(p, "a.txt", "m2");
    git(p, &["update-ref", "refs/remotes/origin/feature", &m2.0]);
    git(p, &["update-ref", "refs/remotes/origin/only-remote", &m2.0]);
    let repo = GixRepo::new(p);

    assert_eq!(repo.branch_tip("feature").await.unwrap(), Some(m1));
    assert_eq!(repo.branch_tip("origin/feature").await.unwrap(), Some(m2.clone()));
    assert_eq!(repo.branch_tip("origin/only-remote").await.unwrap(), Some(m2));
    assert_eq!(repo.branch_tip("nope").await.unwrap(), None);
    assert_eq!(
        repo.branch_tip("HEAD~1").await.unwrap(),
        None,
        "only branches, not revisions"
    );
}

#[tokio::test]
async fn remote_branches_are_listed_unless_a_local_branch_follows_them_at_the_same_commit() {
    let dir = repo();
    let p = dir.path();
    let m1 = commit(p, "a.txt", "m1");
    git(p, &["branch", "feature"]);
    let m2 = commit(p, "a.txt", "m2");
    git(p, &["remote", "add", "origin", "https://example.invalid/r.git"]);
    git(p, &["update-ref", "refs/remotes/origin/main", &m2.0]);
    git(p, &["update-ref", "refs/remotes/origin/feature", &m2.0]);
    git(p, &["update-ref", "refs/remotes/origin/topic", &m1.0]);
    git(
        p,
        &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"],
    );
    let repo = GixRepo::new(p);

    let branches = repo.branches().await.unwrap();

    let listed: Vec<_> = branches.iter().map(|b| (b.name.as_str(), b.remote)).collect();
    assert_eq!(
        listed,
        [
            ("feature", false),
            ("main", false),
            ("origin/feature", true),
            ("origin/topic", true)
        ],
        "origin/main is main's own; origin/HEAD is not a branch; origin/feature is ahead of feature"
    );
    let local = |name: &str| branches.iter().find(|b| b.name == name && !b.remote).unwrap();
    assert!(local("main").default && !local("feature").default);
}

#[tokio::test]
async fn a_dangling_origin_head_is_not_the_default_reference() {
    let dir = repo();
    let p = dir.path();
    commit(p, "a.txt", "m1");
    git(p, &["branch", "-m", "master"]);
    git(
        p,
        &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/gone"],
    );

    let default = GixRepo::new(p).default_reference().await.unwrap();

    assert_eq!(default.as_deref(), Some("refs/heads/master"));
}

#[tokio::test]
async fn a_commit_message_with_crlf_has_a_clean_subject_and_a_body() {
    let dir = repo();
    let p = dir.path();
    std::fs::write(p.join("a.txt"), "a").unwrap();
    git(p, &["add", "-A"]);
    std::fs::write(
        p.join("msg"),
        "Subject line\r\n\r\nFirst body line\r\nSecond body line\r\n",
    )
    .unwrap();
    git(p, &["commit", "-q", "--cleanup=verbatim", "-F", "msg"]);
    let id = CommitId(git(p, &["rev-parse", "HEAD"]));
    let repo = GixRepo::new(p);

    let info = repo.commit_info(&id).await.unwrap();
    let detail = repo.commit_detail(&id).await.unwrap();

    assert_eq!(info.subject, "Subject line");
    assert_eq!(detail.info.subject, "Subject line");
    assert_eq!(
        detail.body.replace("\r\n", "\n").trim_end(),
        "First body line\nSecond body line"
    );
}

#[tokio::test]
async fn a_subject_runs_to_the_end_of_the_first_paragraph_and_a_commit_without_a_body_has_none() {
    let dir = repo();
    let p = dir.path();
    std::fs::write(p.join("a.txt"), "a").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "wrapped\nsubject", "-m", "the body"]);
    let wrapped = CommitId(git(p, &["rev-parse", "HEAD"]));
    let plain = commit(p, "b.txt", "plain");
    let repo = GixRepo::new(p);

    assert_eq!(repo.commit_info(&wrapped).await.unwrap().subject, "wrapped subject");
    assert_eq!(repo.commit_detail(&wrapped).await.unwrap().body.trim_end(), "the body");
    let plain = repo.commit_detail(&plain).await.unwrap();
    assert_eq!((plain.info.subject.as_str(), plain.body.as_str()), ("plain", ""));
    assert_eq!(plain.parents.len(), 1);
}

fn path_bytes(bytes: &[u8]) -> RepoPath {
    RepoPath(bytes.to_vec())
}

/// Records `content` at `name` in the index and commits, without writing the file to disk: not
/// every filesystem takes the name.
fn commit_raw(dir: &Path, name: &[u8], content: &str) -> CommitId {
    let oid = {
        let mut hash = Command::new("git")
            .current_dir(dir)
            .args(["hash-object", "-w", "--stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(&mut hash.stdin.take().unwrap(), content.as_bytes()).unwrap();
        String::from_utf8(hash.wait_with_output().unwrap().stdout)
            .unwrap()
            .trim()
            .to_owned()
    };
    let mut entry = format!("100644,{oid},").into_bytes();
    entry.extend_from_slice(name);
    git_os(
        dir,
        &[
            OsStr::new("update-index"),
            OsStr::new("--add"),
            OsStr::new("--cacheinfo"),
            OsStr::from_bytes(&entry),
        ],
    );
    git(dir, &["commit", "-q", "-m", "raw"]);
    CommitId(git(dir, &["rev-parse", "HEAD"]))
}

fn remove_raw(dir: &Path, name: &[u8]) {
    git_os(
        dir,
        &[
            OsStr::new("update-index"),
            OsStr::new("--force-remove"),
            OsStr::from_bytes(name),
        ],
    );
}

#[tokio::test]
async fn a_file_name_that_is_not_utf8_keeps_its_bytes_through_snapshots_diffs_and_renames() {
    let dir = repo();
    let p = dir.path();
    let content: String = (0..20).map(|i| format!("line {i}\n")).collect();
    let first = commit_raw(p, b"d\xffir/old\xfe.txt", &content);
    remove_raw(p, b"d\xffir/old\xfe.txt");
    let second = commit_raw(p, b"d\xffir/new\xfd.txt", &content);
    let repo = GixRepo::new(p);

    let before = repo.commit_snapshot(&first).await.unwrap();
    let after = repo.commit_snapshot(&second).await.unwrap();
    assert_eq!(
        before.entries.keys().collect::<Vec<_>>(),
        [&path_bytes(b"d\xffir/old\xfe.txt")]
    );
    assert_eq!(
        after.entries.keys().collect::<Vec<_>>(),
        [&path_bytes(b"d\xffir/new\xfd.txt")]
    );

    let diff = repo.diff(&NoStore, &before, &after).await.unwrap().diff;
    assert_eq!(
        diff.renames.0.get(&path_bytes(b"d\xffir/old\xfe.txt")),
        Some(&path_bytes(b"d\xffir/new\xfd.txt"))
    );
    assert_eq!(diff.non_text.len(), 1);
    assert_eq!(diff.non_text[0].kind, NonTextKind::Rename);

    let blob = after.entries.values().next().unwrap().blob;
    let read = repo.read_blob(&path_bytes(b"d\xffir/new\xfd.txt"), blob).await.unwrap();
    assert_eq!(read.as_deref(), Some(content.as_bytes()));
}

#[tokio::test]
async fn the_root_is_found_from_inside_the_tree_and_a_tree_that_is_not_one_says_so() {
    let dir = repo();
    let p = dir.path();
    std::fs::create_dir_all(p.join("a/b")).unwrap();

    assert_eq!(find_root(&p.join("a/b")).unwrap(), std::fs::canonicalize(p).unwrap());

    let plain = tempfile::tempdir().unwrap();
    assert_eq!(
        find_root(plain.path()).unwrap_err(),
        format!("Not a git repository: {}", plain.path().display())
    );

    let bare = tempfile::tempdir().unwrap();
    git(bare.path(), &["init", "-q", "--bare"]);
    assert_eq!(
        find_root(bare.path()).unwrap_err(),
        "Bare repositories are not supported"
    );
}

fn names(files: Vec<PathBuf>) -> BTreeSet<String> {
    files.into_iter().map(|f| f.to_string_lossy().into_owned()).collect()
}

#[tokio::test]
async fn a_worktrees_files_are_what_git_sees_and_a_plain_directorys_are_all_of_them() {
    let io = Budget::new(2);
    let dir = repo();
    let p = dir.path();
    std::fs::write(p.join(".gitignore"), "ignored.txt\n").unwrap();
    std::fs::create_dir_all(p.join("sub")).unwrap();
    std::fs::write(p.join("tracked.txt"), "t").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "c"]);
    std::fs::write(p.join("sub/untracked.txt"), "u").unwrap();
    std::fs::write(p.join("ignored.txt"), "i").unwrap();
    std::fs::remove_file(p.join("tracked.txt")).unwrap();

    let seen = names(list_files(&io, p).await.unwrap());
    assert_eq!(
        seen,
        [".gitignore", "sub/untracked.txt", "tracked.txt"]
            .map(String::from)
            .into(),
        "the index lists what is deleted on disk; ignored files are left out"
    );

    let plain = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(plain.path().join("x/y")).unwrap();
    std::fs::create_dir_all(plain.path().join("empty")).unwrap();
    std::fs::write(plain.path().join("a.txt"), "a").unwrap();
    std::fs::write(plain.path().join("x/y/b.txt"), "b").unwrap();
    assert_eq!(
        names(list_files(&io, plain.path()).await.unwrap()),
        ["a.txt", "x/y/b.txt"].map(String::from).into()
    );
}

#[tokio::test]
async fn a_conflicted_path_is_one_entry_with_what_the_worktree_holds() {
    let dir = repo();
    let p = dir.path();
    commit(p, "a.txt", "base\n");
    git(p, &["checkout", "-q", "-b", "other"]);
    commit(p, "a.txt", "theirs\n");
    git(p, &["checkout", "-q", "main"]);
    commit(p, "a.txt", "ours\n");
    let merge = Command::new("git")
        .current_dir(p)
        .args(["merge", "-q", "other"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .unwrap();
    assert!(!merge.status.success(), "the merge conflicts");
    let held = std::fs::read(p.join("a.txt")).unwrap();
    assert!(String::from_utf8_lossy(&held).contains("<<<<<<<"));

    let scan = GixRepo::new(p).scan(Dirty::All).await.unwrap();

    assert_eq!(scan.snapshot.entries.len(), 1);
    assert_eq!(scan.snapshot.entries[&path("a.txt")].blob, BlobId::of(&held));
}
