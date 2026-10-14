//! The changed symbols of the changeset on view, analysed by the backend and pushed without the
//! window asking. The tests need a real rust-analyzer: they are ignored and run by
//! `ci/tool-tests.sh`.

#![allow(clippy::disallowed_methods)]

use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedReceiver;
#[cfg(debug_assertions)]
use vet_backend::Injected;
use vet_backend::{Orchestrator, open_store};
use vet_protocol::{ChangesetKey, Event, FileSymbols, ProjectId, Request, ServerMessage, SymbolChange};

use crate::common::{Client, config, git, write};

const BASE_LIB: &str = "\
mod other;

pub fn gone() -> u32 {
    1
}

pub fn keep() -> u32 {
    2
}
";

const DIRTY_LIB: &str = "\
mod other;

pub fn keep() -> u32 {
    3
}

pub fn fresh() -> u32 {
    keep()
}
";

const BASE_OTHER: &str = "\
pub fn steady() -> u32 {
    1
}
";

const DIRTY_OTHER: &str = "\
pub fn steady() -> u32 {
    2
}
";

/// A Rust crate whose base (`BASE_LIB`, `BASE_OTHER`) is committed.
fn rust_repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    write(repo.path(), ".gitignore", "target/\n");
    write(
        repo.path(),
        "Cargo.toml",
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(repo.path(), "src/lib.rs", BASE_LIB);
    write(repo.path(), "src/other.rs", BASE_OTHER);
    let lock = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "base"]);
    repo
}

/// The next `FileSymbols` event of `project`, waiting up to `within`.
async fn next_symbols(
    inbox: &mut UnboundedReceiver<ServerMessage>,
    project: ProjectId,
    within: Duration,
) -> Option<FileSymbols> {
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
            event: Event::FileSymbols {
                project: at, symbols, ..
            },
        }) = message
            && at == project
        {
            return Some(symbols);
        }
    }
}

/// The next `FileSymbols` event of `project` for `path`, waiting up to `within`.
#[cfg(debug_assertions)]
async fn next_symbols_for(
    inbox: &mut UnboundedReceiver<ServerMessage>,
    project: ProjectId,
    path: &str,
    within: Duration,
) -> Option<FileSymbols> {
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
            event: Event::FileSymbols {
                project: at, symbols, ..
            },
        }) = message
            && at == project
            && symbols.path == path
        {
            return Some(symbols);
        }
    }
}

/// A backend on a crate with both files changed in the working tree, the review screen shown, and
/// `open` remembered as the files the window has open. The directory the store lives in is
/// returned too, and must be kept: its reader pool opens connections as they are needed, and one
/// opened after the directory was dropped fails ("unable to open database file").
async fn reviewing(open: &[&str]) -> (Client, ProjectId, tempfile::TempDir, tempfile::TempDir) {
    reviewing_with(open, |handler| handler).await
}

/// As `reviewing`, with the orchestrator `wire` hands back; the tests of the retry use it to make
/// the first analyses of a file fail before they run for real.
async fn reviewing_with(
    open: &[&str],
    wire: impl FnOnce(Orchestrator) -> Orchestrator,
) -> (Client, ProjectId, tempfile::TempDir, tempfile::TempDir) {
    let repo = rust_repo();
    write(repo.path(), "src/lib.rs", DIRTY_LIB);
    write(repo.path(), "src/other.rs", DIRTY_OTHER);
    let data = tempfile::tempdir().unwrap();
    let handler = Arc::new(wire(Orchestrator::new(
        open_store(data.path()).await.unwrap(),
        config(),
    )));
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    client
        .call(Request::OpenChangeset {
            project,
            changeset: ChangesetKey::WorkingTree,
        })
        .await
        .unwrap();
    if !open.is_empty() {
        client
            .call(Request::SetOpenFiles {
                project,
                paths: open.iter().map(|s| (*s).to_owned()).collect(),
            })
            .await
            .unwrap();
    }
    client
        .call(Request::SetReviewShown { project, shown: true })
        .await
        .unwrap();
    (client, project, repo, data)
}

#[tokio::test]
#[ignore = "needs rust-analyzer"]
async fn the_changed_symbols_are_pushed_without_being_asked_for() {
    // The file the window has open is analysed alone first, so this answer is its own.
    let (mut client, project, _repo, _data) = reviewing(&["src/lib.rs"]).await;

    let symbols = next_symbols(&mut client.inbox, project, Duration::from_secs(180))
        .await
        .expect("the backend pushes a file's changed symbols with no request");

    assert_eq!(symbols.path, "src/lib.rs");
    let found: Vec<(String, SymbolChange)> = symbols
        .symbols
        .iter()
        .map(|c| (c.symbol.id.name.join("::"), c.change))
        .collect();
    assert!(found.contains(&("fresh".to_owned(), SymbolChange::Added)), "{found:?}");
    assert!(
        found.contains(&("keep".to_owned(), SymbolChange::Modified)),
        "{found:?}"
    );
}

#[tokio::test]
#[ignore = "needs rust-analyzer"]
async fn the_open_file_is_analysed_first() {
    let (mut client, project, _repo, _data) = reviewing(&["src/other.rs"]).await;

    let first = next_symbols(&mut client.inbox, project, Duration::from_secs(180))
        .await
        .expect("an answer");
    assert_eq!(first.path, "src/other.rs", "the open file comes first");
    let second = next_symbols(&mut client.inbox, project, Duration::from_secs(60))
        .await
        .expect("the other file too");
    assert_eq!(second.path, "src/lib.rs");
}

#[tokio::test]
#[ignore = "needs rust-analyzer"]
async fn nothing_more_is_analysed_once_the_review_screen_goes_away() {
    // The open file is its own batch, analysed before the rest: hiding after it is answered must
    // leave the other file unanalysed.
    let (mut client, project, _repo, _data) = reviewing(&["src/other.rs"]).await;
    let first = next_symbols(&mut client.inbox, project, Duration::from_secs(180))
        .await
        .expect("an answer");
    assert_eq!(first.path, "src/other.rs");

    client
        .call(Request::SetReviewShown { project, shown: false })
        .await
        .unwrap();
    assert!(
        next_symbols(&mut client.inbox, project, Duration::from_secs(10))
            .await
            .is_none(),
        "the other file is not analysed once the screen is away"
    );
}

#[cfg(debug_assertions)]
#[tokio::test]
#[ignore = "needs rust-analyzer"]
async fn a_failure_that_can_pass_is_asked_again_and_only_the_success_is_pushed() {
    // The store fails the file's first analysis; the backend asks again, the second try succeeds,
    // and the client hears the success alone.
    let (mut client, project, _repo, _data) = reviewing_with(&["src/lib.rs"], |handler| {
        handler.with_analysis_failing("src/lib.rs", vec![Injected::Store])
    })
    .await;

    let symbols = next_symbols(&mut client.inbox, project, Duration::from_secs(180))
        .await
        .expect("an answer");
    assert_eq!(symbols.path, "src/lib.rs");
    assert!(symbols.gap.is_none(), "no failure was pushed: {:?}", symbols.gap);
    let found: Vec<(String, SymbolChange)> = symbols
        .symbols
        .iter()
        .map(|c| (c.symbol.id.name.join("::"), c.change))
        .collect();
    assert!(found.contains(&("fresh".to_owned(), SymbolChange::Added)), "{found:?}");
}

#[cfg(debug_assertions)]
#[tokio::test]
#[ignore = "needs rust-analyzer"]
async fn a_lasting_unavailability_is_pushed_at_once() {
    // The file is not there to analyse; that cannot pass, so the failure is pushed and the file is
    // not asked again.
    let (mut client, project, _repo, _data) = reviewing_with(&["src/lib.rs"], |handler| {
        handler.with_analysis_failing("src/lib.rs", vec![Injected::Gone])
    })
    .await;

    let first = next_symbols_for(&mut client.inbox, project, "src/lib.rs", Duration::from_secs(180))
        .await
        .expect("an answer");
    assert!(first.gap.is_some(), "the lasting reason is pushed: {first:?}");
    assert!(
        next_symbols_for(&mut client.inbox, project, "src/lib.rs", Duration::from_secs(10))
            .await
            .is_none(),
        "a lasting reason is not asked again"
    );
}
