//! Fetching against local bare repositories as remotes, over `file://`. `git` is
//! used only to build the fixtures and to look at the result.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use vet_app::ports::{Fetcher, History};
use vet_infra::git::GixRepo;

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

fn commit(dir: &Path, file: &str, message: &str) -> String {
    std::fs::write(dir.join(file), message).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

/// A bare `origin` with `main` and `feature`, a clone of it, and the work area that feeds the remote.
struct Fixture {
    _dir: TempDir,
    remote: PathBuf,
    upstream_work: PathBuf,
    clone: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (remote, upstream_work, clone) = (
            dir.path().join("origin.git"),
            dir.path().join("work"),
            dir.path().join("clone"),
        );
        std::fs::create_dir_all(&upstream_work).unwrap();
        git(
            dir.path(),
            &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()],
        );
        git(&upstream_work, &["init", "-q", "-b", "main"]);
        commit(&upstream_work, "a.txt", "m1");
        git(&upstream_work, &["checkout", "-q", "-b", "feature"]);
        commit(&upstream_work, "b.txt", "f1");
        git(&upstream_work, &["checkout", "-q", "main"]);
        git(&upstream_work, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git(&upstream_work, &["push", "-q", "origin", "main", "feature"]);
        git(
            dir.path(),
            &[
                "clone",
                "-q",
                &format!("file://{}", remote.display()),
                clone.to_str().unwrap(),
            ],
        );
        Self {
            _dir: dir,
            remote,
            upstream_work,
            clone,
        }
    }

    /// A new commit on the remote's `main`.
    fn advance_remote(&self, message: &str) -> String {
        git(&self.upstream_work, &["checkout", "-q", "main"]);
        let id = commit(&self.upstream_work, "a.txt", message);
        git(&self.upstream_work, &["push", "-q", "origin", "main"]);
        id
    }

    /// Every ref, HEAD and the state of the worktree: what a fetch must leave alone, apart from `refs/remotes`.
    fn outside_remotes(&self) -> String {
        let refs = git(&self.clone, &["for-each-ref", "--format=%(refname) %(objectname)"]);
        let outside: Vec<_> = refs.lines().filter(|l| !l.starts_with("refs/remotes/")).collect();
        format!(
            "{}\nHEAD {}\n{}",
            outside.join("\n"),
            git(&self.clone, &["rev-parse", "HEAD"]),
            git(&self.clone, &["status", "--porcelain", "--ignored"]),
        )
    }

    fn remote_ref(&self, name: &str) -> String {
        git(&self.clone, &["rev-parse", &format!("refs/remotes/origin/{name}")])
    }
}

#[tokio::test]
async fn a_fetch_brings_new_remote_commits_and_moves_only_remote_tracking_refs() {
    let fx = Fixture::new();
    let before = fx.outside_remotes();
    let old_main = fx.remote_ref("main");
    let new_main = fx.advance_remote("m2");
    git(&fx.upstream_work, &["tag", "v1"]);
    git(&fx.upstream_work, &["push", "-q", "origin", "v1"]);
    git(&fx.upstream_work, &["checkout", "-q", "-b", "extra"]);
    let extra = commit(&fx.upstream_work, "e.txt", "e1");
    git(&fx.upstream_work, &["push", "-q", "origin", "extra"]);
    let repo = GixRepo::new(&fx.clone);

    let updated = repo.fetch("origin", None).await.unwrap();

    assert_eq!(updated, 2, "main moved, extra is new");
    assert_ne!(old_main, new_main);
    assert_eq!(fx.remote_ref("main"), new_main);
    assert_eq!(fx.remote_ref("extra"), extra);
    assert_eq!(
        fx.outside_remotes(),
        before,
        "branches, tags, HEAD and the worktree stay as they were"
    );
}

#[tokio::test]
async fn a_fetch_follows_the_refspecs_of_the_remote() {
    let fx = Fixture::new();
    git(&fx.clone, &["update-ref", "-d", "refs/remotes/origin/feature"]);
    git(
        &fx.clone,
        &[
            "config",
            "--replace-all",
            "remote.origin.fetch",
            "+refs/heads/main:refs/remotes/origin/main",
        ],
    );
    let new_main = fx.advance_remote("m2");
    let repo = GixRepo::new(&fx.clone);

    repo.fetch("origin", None).await.unwrap();

    assert_eq!(fx.remote_ref("main"), new_main);
    let refs = git(&fx.clone, &["for-each-ref", "--format=%(refname)", "refs/remotes/"]);
    assert!(!refs.contains("feature"), "only main is configured: {refs}");
}

#[tokio::test]
async fn a_refspec_that_would_write_a_local_branch_is_refused() {
    let fx = Fixture::new();
    git(
        &fx.clone,
        &[
            "config",
            "--replace-all",
            "remote.origin.fetch",
            "+refs/heads/*:refs/heads/*",
        ],
    );
    let before = fx.outside_remotes();
    fx.advance_remote("m2");
    let repo = GixRepo::new(&fx.clone);

    let error = repo.fetch("origin", None).await.unwrap_err();

    assert_eq!(error.0, "origin has no refspec that fetches into refs/remotes");
    assert_eq!(fx.outside_remotes(), before);
}

#[tokio::test]
async fn the_remotes_are_listed_with_their_urls() {
    let fx = Fixture::new();
    git(
        &fx.clone,
        &["remote", "add", "mirror", "https://Git.Example.org:8443/x/y.git"],
    );
    git(&fx.clone, &["remote", "add", "ssh", "git@github.com:x/y.git"]);
    let repo = GixRepo::new(&fx.clone);

    let remotes = repo.remotes().await.unwrap();

    let listed: Vec<_> = remotes
        .iter()
        .map(|r| (r.name.as_str(), r.https_host.as_deref()))
        .collect();
    assert_eq!(
        listed,
        [
            ("mirror", Some("git.example.org:8443")),
            ("origin", None),
            ("ssh", None)
        ]
    );
    assert!(remotes[1].url.starts_with("file://"));
}

#[tokio::test]
async fn a_remote_that_cannot_be_reached_fails_with_a_message_and_changes_nothing() {
    let fx = Fixture::new();
    let before = fx.outside_remotes();
    let old_main = fx.remote_ref("main");
    std::fs::remove_dir_all(&fx.remote).unwrap();
    let repo = GixRepo::new(&fx.clone);

    let error = repo.fetch("origin", None).await.unwrap_err();

    assert!(
        error.0.contains("origin.git"),
        "names what could not be reached: {}",
        error.0
    );
    assert_eq!(fx.remote_ref("main"), old_main);
    assert_eq!(fx.outside_remotes(), before);
    assert!(repo.branches().await.is_ok(), "the repository still works");
}

#[tokio::test]
async fn a_branch_knows_where_the_remote_had_it_at_the_last_fetch() {
    let fx = Fixture::new();
    let repo = GixRepo::new(&fx.clone);
    let main = |branches: Vec<vet_app::ports::BranchInfo>| branches.into_iter().find(|b| b.name == "main").unwrap();
    let at_clone = main(repo.branches().await.unwrap());
    assert_eq!(at_clone.upstream.as_ref(), Some(&at_clone.commit));

    let new_main = fx.advance_remote("m2");
    assert_eq!(
        main(repo.branches().await.unwrap()).upstream,
        Some(at_clone.commit.clone()),
        "not fetched yet"
    );
    repo.fetch("origin", None).await.unwrap();

    let fetched = main(repo.branches().await.unwrap());
    assert_eq!(fetched.upstream.map(|c| c.0), Some(new_main));
    assert_eq!(fetched.commit, at_clone.commit, "the local branch does not move");
}
