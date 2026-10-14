//! The changeset list over the real store: each branch's review state, open threads and progress
//! are read in one batched query, and branches with and without a review come out as before.

#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use vet_app::changeset::Source;
use vet_app::session::{LiveSession, NewThread, RowReview, View};
use vet_infra::git::GixRepo;
use vet_infra::store::SqliteStore;

type Session = LiveSession<GixRepo, SqliteStore>;

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

fn clock() -> i64 {
    1
}

struct Fixture {
    repo: TempDir,
    data: TempDir,
}

impl Fixture {
    /// A repository with `main` and two branches on top of it, `feature` and `other`.
    fn new() -> Self {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(repo.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        git(repo.path(), &["add", "-A"]);
        git(repo.path(), &["commit", "-q", "-m", "base"]);
        for (name, text) in [("feature", "one\nTWO\nthree\n"), ("other", "one\ntwo\nTHREE\n")] {
            git(repo.path(), &["checkout", "-q", "-b", name]);
            std::fs::write(repo.path().join("a.txt"), text).unwrap();
            git(repo.path(), &["commit", "-q", "-am", name]);
        }
        git(repo.path(), &["checkout", "-q", "main"]);
        Self {
            repo,
            data: tempfile::tempdir().unwrap(),
        }
    }

    async fn session(&self) -> Session {
        let store = SqliteStore::open(self.data.path()).await.unwrap();
        let root = self.repo.path().to_str().unwrap().to_owned();
        LiveSession::open(GixRepo::new(self.repo.path()), store, root, clock)
            .await
            .unwrap()
    }
}

fn branch(name: &str) -> Source {
    Source::Branch { name: name.to_owned() }
}

#[tokio::test]
async fn the_listing_gives_each_branch_its_review_state_open_threads_and_progress() {
    let fx = Fixture::new();
    let session = fx.session().await;
    // Review `feature` and leave a thread open on it; `other` stays without a review.
    session.open_changeset(branch("feature")).await.unwrap();
    let revision = session.revision().await;
    session
        .toggle_hunk(revision, View::Changeset, "a.txt", 0)
        .await
        .unwrap();
    session
        .create_thread(revision, View::Changeset, NewThread::Changeset, "a note".into(), 1)
        .await
        .unwrap();

    let list = session.changesets().await.unwrap();

    let row = |name: &str| list.rows.iter().find(|row| row.source == branch(name)).unwrap();
    let reviewed = row("feature");
    assert!(
        matches!(reviewed.review, RowReview::Open { version: 1, .. }),
        "{:?}",
        reviewed.review
    );
    assert!(
        matches!(reviewed.review, RowReview::Open { progress: Some(_), .. }),
        "the review's progress is read: {:?}",
        reviewed.review
    );
    assert_eq!(reviewed.open_threads, 1, "its open thread is counted");
    let unreviewed = row("other");
    assert_eq!(unreviewed.review, RowReview::None, "no review is no review");
    assert_eq!(unreviewed.open_threads, 0);
    assert_eq!(list.on_view, branch("feature"));
}
