//! The problems of the changeset on view, analysed and compared with the base by the backend and
//! pushed without the window asking. The tests need a real rust-analyzer: they are ignored and run
//! by `ci/tool-tests.sh`.

#![allow(clippy::disallowed_methods)]

use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedReceiver;
use vet_backend::{Orchestrator, open_store};
use vet_protocol::{ChangesetKey, Diagnostic, DiagnosticOrigin, Event, ProjectId, Request, ServerMessage, Severity};

use crate::common::{Client, config, git, write};

const BASE: &str = "\
pub fn keep() -> u32 {
    2
}

pub fn trouble() -> u32 {
    let y: u32 = \"y\";
    y
}
";

const DIRTY: &str = "\
pub fn keep() -> u32 {
    3
}

pub fn broken() -> u32 {
    let x: u32 = \"x\";
    x
}

pub fn trouble() -> u32 {
    let y: u32 = \"y\";
    y
}
";

fn rust_repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    write(repo.path(), ".gitignore", "target/\n");
    write(
        repo.path(),
        "Cargo.toml",
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(repo.path(), "src/lib.rs", BASE);
    let lock = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "base"]);
    write(repo.path(), "src/lib.rs", DIRTY);
    repo
}

/// The next `FileDiagnostics` event of `project` that `want` accepts, waiting up to `within`.
async fn next_diagnostics(
    inbox: &mut UnboundedReceiver<ServerMessage>,
    project: ProjectId,
    within: Duration,
    want: fn(&[Diagnostic], bool) -> bool,
) -> Option<(Vec<Diagnostic>, bool)> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        let Ok(message) = tokio::time::timeout(left, inbox.recv()).await else {
            return None;
        };
        if let Some(ServerMessage::Event {
            event:
                Event::FileDiagnostics {
                    project: at,
                    path,
                    diagnostics,
                    compared,
                    ..
                },
        }) = message
            && at == project
            && path == "src/lib.rs"
            && want(&diagnostics, compared)
        {
            return Some((diagnostics, compared));
        }
    }
}

/// A backend on a crate whose working tree introduces one error and keeps another.
async fn reviewing() -> (Client, ProjectId, tempfile::TempDir) {
    let repo = rust_repo();
    let data = tempfile::tempdir().unwrap();
    let handler = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config()));
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    client
        .call(Request::OpenChangeset {
            project,
            changeset: ChangesetKey::WorkingTree,
        })
        .await
        .unwrap();
    client
        .call(Request::SetReviewShown { project, shown: true })
        .await
        .unwrap();
    (client, project, repo)
}

fn has_new(diagnostics: &[Diagnostic], compared: bool) -> bool {
    compared && diagnostics.iter().any(|d| d.origin == DiagnosticOrigin::New)
}

#[tokio::test]
#[ignore = "needs rust-analyzer"]
async fn the_compared_problems_are_pushed_with_the_origins_the_comparison_found() {
    let (mut client, project, _repo) = reviewing().await;

    let (diagnostics, compared) = next_diagnostics(&mut client.inbox, project, Duration::from_secs(180), has_new)
        .await
        .expect("the backend pushes the compared problems with no request");
    assert!(compared);
    assert!(
        diagnostics
            .iter()
            .any(|d| d.origin == DiagnosticOrigin::New && d.severity == Severity::Error),
        "the change's own error is new: {diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.origin == DiagnosticOrigin::Existing && d.severity == Severity::Error),
        "the error the base has too is existing: {diagnostics:?}"
    );
}

#[tokio::test]
#[ignore = "needs rust-analyzer"]
async fn the_first_answer_comes_with_the_origins_pending() {
    let (mut client, project, _repo) = reviewing().await;

    let (_, compared) = next_diagnostics(&mut client.inbox, project, Duration::from_secs(180), |_, _| true)
        .await
        .expect("a first answer");
    assert!(
        !compared,
        "the first answer is the target side, with the origins pending"
    );
}

#[tokio::test]
#[ignore = "needs rust-analyzer"]
async fn nothing_more_is_analysed_once_the_review_screen_goes_away() {
    let (mut client, project, _repo) = reviewing().await;
    // The first answer, with the origins pending, is in.
    let _ = next_diagnostics(&mut client.inbox, project, Duration::from_secs(180), |_, compared| {
        !compared
    })
    .await
    .expect("a first answer");

    client
        .call(Request::SetReviewShown { project, shown: false })
        .await
        .unwrap();
    assert!(
        next_diagnostics(&mut client.inbox, project, Duration::from_secs(10), |_, _| true)
            .await
            .is_none(),
        "no comparison is started once the screen is away"
    );
}
