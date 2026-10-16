//! A working tree that holds an entry the scan cannot read: reconciliation succeeds, reports
//! everything else, and does not treat files git still tracks as deleted.

#![allow(clippy::disallowed_methods)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use vet_app::acquisition::Acquisition;
use vet_app::ports::{Dirty, Scan};
use vet_core::content::RepoPath;
use vet_infra::git::GixRepo;

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

fn path(name: &str) -> RepoPath {
    RepoPath(name.as_bytes().to_vec())
}

/// Denies reading `path` for as long as it lives, so the tree can be cleaned up afterwards.
struct Denied(PathBuf, std::fs::Permissions);

impl Denied {
    fn new(path: &Path) -> Self {
        let original = std::fs::metadata(path).unwrap().permissions();
        let mut denied = original.clone();
        denied.set_mode(0o000);
        std::fs::set_permissions(path, denied).unwrap();
        Self(path.to_owned(), original)
    }
}

impl Drop for Denied {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.0, self.1.clone());
    }
}

fn names(scan: &Scan) -> Vec<String> {
    scan.snapshot.entries.keys().map(RepoPath::to_string_lossy).collect()
}

async fn reconcile(p: &Path) -> Result<Scan, vet_app::ports::GitError> {
    Acquisition::new(GixRepo::for_test(p), ()).reconcile(Dirty::All).await
}

#[tokio::test]
async fn reconciliation_skips_a_directory_it_cannot_read_and_reports_the_rest() {
    let dir = repo();
    let p = dir.path();
    std::fs::write(p.join("tracked.txt"), "tracked\n").unwrap();
    std::fs::create_dir_all(p.join("locked")).unwrap();
    std::fs::write(p.join("locked/inside.txt"), "inside\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "c"]);
    std::fs::write(p.join("untracked.txt"), "untracked\n").unwrap();

    let locked = p.join("locked");
    let _denied = Denied::new(&locked);
    let scan = reconcile(p).await.unwrap();

    assert_eq!(
        names(&scan),
        ["locked/inside.txt", "tracked.txt", "untracked.txt"],
        "the readable files are reported and the tracked file is not treated as deleted"
    );
    assert!(
        scan.status.content_unavailable.contains(&path("locked/inside.txt")),
        "the tracked file under the unreadable directory could not be read: {:?}",
        scan.status.content_unavailable
    );
}

#[tokio::test]
async fn an_unreadable_untracked_directory_is_left_out_and_reconciliation_succeeds() {
    let dir = repo();
    let p = dir.path();
    std::fs::write(p.join("readable.txt"), "readable\n").unwrap();
    std::fs::create_dir_all(p.join("locked")).unwrap();
    std::fs::write(p.join("locked/secret.txt"), "secret\n").unwrap();

    let locked = p.join("locked");
    let _denied = Denied::new(&locked);
    let scan = reconcile(p).await.unwrap();

    assert_eq!(names(&scan), ["readable.txt"]);
}

#[tokio::test]
async fn an_unreadable_worktree_root_still_fails_the_reconciliation() {
    let dir = repo();
    let p = dir.path();
    std::fs::write(p.join("a.txt"), "a\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "c"]);

    let _denied = Denied::new(p);
    let scan = reconcile(p).await;

    assert!(scan.is_err(), "an unusable tree must still fail");
}

#[tokio::test]
async fn a_tracked_file_that_cannot_be_read_is_kept_as_unavailable() {
    let dir = repo();
    let p = dir.path();
    std::fs::write(p.join("secret.txt"), "secret\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "c"]);

    let secret = p.join("secret.txt");
    let _denied = Denied::new(&secret);
    let scan = reconcile(p).await.unwrap();

    assert_eq!(names(&scan), ["secret.txt"]);
    assert_eq!(scan.status.content_unavailable, vec![path("secret.txt")]);
}
