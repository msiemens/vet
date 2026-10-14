//! A session over the real store and git when writing a version's refs fails: the review is
//! left as it was and the same version can be recorded again.

#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use vet_app::ports::Dirty;
use vet_app::session::{Lifecycle, LiveSession, View};
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
    /// A repository with `a.txt` committed and edited in the working tree.
    fn new() -> Self {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(repo.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        git(repo.path(), &["add", "-A"]);
        git(repo.path(), &["commit", "-q", "-m", "base"]);
        let fixture = Self {
            repo,
            data: tempfile::tempdir().unwrap(),
        };
        fixture.edit("one\nTWO\nthree\n");
        fixture
    }

    fn edit(&self, content: &str) {
        std::fs::write(self.repo.path().join("a.txt"), content).unwrap();
    }

    async fn session(&self) -> Session {
        let store = SqliteStore::open(self.data.path()).await.unwrap();
        let repo = self.repo.path().to_str().unwrap().to_owned();
        LiveSession::open(GixRepo::new(self.repo.path()), store, repo, clock)
            .await
            .unwrap()
    }

    /// Makes writing any ref of a changeset fail, or work again: what they need is a directory.
    fn block_refs(&self, blocked: bool) {
        let path = self.repo.path().join(".git/refs/vet");
        if blocked {
            std::fs::write(&path, "in the way").unwrap();
        } else {
            std::fs::remove_file(&path).unwrap();
        }
    }

    /// The directory of the refs of the one changeset the repository has.
    fn changeset_refs(&self) -> PathBuf {
        let below = self.repo.path().join(".git/refs/vet/changesets");
        let mut dirs = std::fs::read_dir(below).unwrap().map(|e| e.unwrap().path());
        let only = dirs.next().expect("a changeset has refs");
        assert!(dirs.next().is_none());
        only
    }
}

async fn settle(session: &Session) -> Result<u32, vet_app::session::SessionError> {
    let scan = session.acquisition().reconcile(Dirty::All).await.unwrap();
    session.settled(scan).await.map(|(revision, _)| revision)
}

async fn versions(session: &Session) -> Vec<u32> {
    session.versions().await.unwrap().iter().map(|v| v.n).collect()
}

#[tokio::test]
async fn a_version_whose_ref_cannot_be_written_is_not_stored_and_settles_again() {
    let fx = Fixture::new();
    let session = fx.session().await;
    session.toggle_hunk(0, View::Changeset, "a.txt", 0).await.unwrap();
    assert_eq!(versions(&session).await, [1]);

    // The agent commits: the next version needs a ref for its new tip, which cannot be written.
    git(fx.repo.path(), &["commit", "-q", "-a", "-m", "agent"]);
    fx.edit("one\nTWO\nthree\nfour\n");
    let blocked = fx.changeset_refs().join("2");
    std::fs::write(&blocked, "in the way").unwrap();
    assert!(settle(&session).await.is_err());
    assert_eq!(
        versions(&session).await,
        [1],
        "nothing was stored for the failed version"
    );

    std::fs::remove_file(&blocked).unwrap();
    settle(&session).await.unwrap();
    assert_eq!(versions(&session).await, [1, 2]);
    assert!(matches!(session.lifecycle().await, Lifecycle::Open { version: 2, .. }));
}

#[tokio::test]
async fn a_review_whose_refs_cannot_be_written_does_not_start() {
    let fx = Fixture::new();
    let session = fx.session().await;
    fx.block_refs(true);
    assert!(session.toggle_hunk(0, View::Changeset, "a.txt", 0).await.is_err());
    assert_eq!(session.lifecycle().await, Lifecycle::Browsing);
    fx.block_refs(false);

    // What a restart finds is what the session showed.
    let again = fx.session().await;
    assert_eq!(again.lifecycle().await, Lifecycle::Browsing);
    again.toggle_hunk(0, View::Changeset, "a.txt", 0).await.unwrap();
    assert_eq!(versions(&again).await, [1]);
}
