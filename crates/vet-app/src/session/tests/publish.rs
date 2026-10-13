//! The state requests read is published whole by the command that changed it: a command that
//! waits on something holds nothing a request needs.

use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::task::{Context, Waker};
use std::time::Duration;

use super::*;

/// A request answers within this long: it is not a latency target, only what turns a request
/// that waits for the command into a failure and not a hang.
const ANSWERED: Duration = Duration::from_secs(10);

/// A reviewing session with a mark in `a.txt` and a comment on its base side, and a command to
/// finish the review that waits for git. Finishing keeps what the open comments are anchored in,
/// which only git holds: the guard returned holds git.
async fn waiting_for_git(
    fx: &Fixture,
) -> (
    tokio::sync::MutexGuard<'_, ()>,
    Pin<Box<impl Future<Output = Result<bool>> + '_>>,
) {
    let thread = NewThread::Range {
        side: Side::Base,
        path: path("a.txt"),
        range: LineRange { start: 5, end: 7 },
    };
    let revision = fx.session.revision().await;
    fx.session
        .create_thread(revision, View::Changeset, thread, "why".into(), 1)
        .await
        .unwrap();
    let paused = fx.git.paused.lock().await;
    let mut moving = Box::pin(fx.session.finish_review());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(moving.as_mut().poll(&mut cx).is_pending(), "the command waits for git");
    (paused, moving)
}

#[tokio::test]
async fn requests_are_answered_with_the_published_state_while_a_command_waits_for_git() {
    let base = numbered(10, &[]);
    let fx = Fixture::reviewing(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[3]))]).await;
    fx.session.toggle_hunk(0, View::Changeset, "a.txt", 0).await.unwrap();
    let base_commit = fx.view.semantic_sides(0).await.unwrap().base_commit;
    let (paused, moving) = waiting_for_git(&fx).await;

    let answered = async {
        let manifest = fx.session.manifest(View::Changeset).await.unwrap();
        let text = fx.session.file_text(View::Changeset, "a.txt").await.unwrap();
        let threads = fx.session.threads(View::Changeset).await.unwrap();
        let sides = fx.view.semantic_sides(0).await.unwrap();
        let files = fx.view.changed_files(0).await.unwrap().len();
        (manifest, text, threads, sides, files)
    };
    let (manifest, text, threads, sides, files) = tokio::time::timeout(ANSWERED, answered)
        .await
        .expect("a request waited for the command");
    assert_eq!(
        (manifest.revision, text.revision, threads.revision),
        (0, 0, 0),
        "the state before the command"
    );
    assert_eq!(manifest.completion.seen_hunks, 1);
    assert_eq!((sides.base_commit, files), (base_commit, 1));
    assert_eq!(fx.session.revision().await, 0);
    assert!(matches!(
        fx.session.lifecycle().await,
        Lifecycle::Open { version: 1, .. }
    ));

    drop(paused);
    moving.await.unwrap();
    assert_eq!(fx.session.revision().await, 1);
    assert_eq!(fx.session.manifest(View::Changeset).await.unwrap().revision, 1);
}

#[tokio::test]
async fn a_settled_tree_is_published_with_its_diff_so_no_request_has_to_install_it() {
    let base = numbered(10, &[]);
    let fx = Fixture::reviewing(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[3]))]).await;
    fx.session.toggle_hunk(0, View::Changeset, "a.txt", 0).await.unwrap();
    let diffs = fx.git.diffs.load(Ordering::Relaxed);

    let revision = fx.settle(&[("a.txt", &numbered(10, &[3, 6]))]).await;
    assert_eq!(
        fx.git.diffs.load(Ordering::Relaxed),
        diffs + 1,
        "settling diffs the new tree"
    );
    let (paused, moving) = waiting_for_git(&fx).await;

    // Showing the revision needs the diff and the marks remapped onto it, which a request that
    // had to install them would do under the token the command holds.
    let manifest = tokio::time::timeout(ANSWERED, fx.session.manifest(View::Changeset))
        .await
        .expect("a request waited for the command")
        .unwrap();
    assert_eq!(manifest.revision, revision);
    assert_eq!(
        manifest.completion.seen_hunks, 1,
        "the mark was remapped onto the new tree"
    );
    assert_eq!(fx.git.diffs.load(Ordering::Relaxed), diffs + 1);

    drop(paused);
    moving.await.unwrap();
}
