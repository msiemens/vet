//! The review's derived values (diffs, interdiffs, commits) are computed once per key, however
//! many requests ask for them and whatever becomes of the requests.

use std::collections::BTreeSet;
use std::pin::Pin;
use std::task::{Context, Waker};

use futures::future::join_all;

use super::*;
use crate::progress::{Task, TaskKind, Tasks};

pub(super) type Heard = Arc<Mutex<Vec<Task>>>;

/// Like [`Fixture::open`], with the session's tasks heard.
pub(super) async fn announcing(base: &[(&str, &str)], dirty: &[(&str, &str)]) -> (Fixture, Heard) {
    let git = FakeGit::default();
    *git.base.lock().unwrap() = Some(git.snapshot(base));
    let first = git.snapshot(dirty);
    git.scans.lock().unwrap().push(first);
    let store = MemStore::default();
    let heard = Heard::default();
    let sink = heard.clone();
    let tasks = Tasks::new(
        std::time::Duration::ZERO,
        Box::new(move |t| sink.lock().unwrap().push(t)),
    );
    let session = LiveSession::open_reporting(git.clone(), store.clone(), REPO.into(), clock, tasks)
        .await
        .unwrap();
    (Fixture { git, store, session }, heard)
}

/// How many tasks titled `title` were started.
fn started(heard: &Heard, title: &str) -> usize {
    let heard = heard.lock().unwrap();
    let ids: BTreeSet<u32> = (heard.iter())
        .filter(|t| t.kind == TaskKind::Diff && t.title == title)
        .map(|t| t.id)
        .collect();
    ids.len()
}

type Call<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + 'a>>;

/// Polls `calls` while the fake is paused until the task `title` started; then the first caller
/// gives up, the fake goes on, and the others finish.
async fn first_gives_up<T>(fx: &Fixture, heard: &Heard, title: &str, mut calls: Vec<Call<'_, T>>) -> Vec<Result<T>> {
    let paused = fx.git.paused.lock().await;
    let mut cx = Context::from_waker(Waker::noop());
    while started(heard, title) == 0 {
        for call in &mut calls {
            assert!(call.as_mut().poll(&mut cx).is_pending(), "the fake is paused");
        }
    }
    for call in &mut calls {
        assert!(call.as_mut().poll(&mut cx).is_pending());
    }
    drop(calls.remove(0));
    drop(paused);
    join_all(calls).await
}

#[tokio::test]
async fn the_requests_for_one_diff_share_it_and_one_giving_up_does_not_start_it_again() {
    let base = numbered(30, &[]);
    let (fx, heard) = announcing(&[("a.txt", &base)], &[("a.txt", &numbered(30, &[3, 20]))]).await;
    let session = &fx.session;
    let mut calls: Vec<Call<'_, usize>> = Vec::new();
    for _ in 0..4 {
        calls.push(Box::pin(async {
            Ok(session.manifest(View::Changeset).await?.files.len())
        }));
        // What the semantic lane asks for the same revision.
        calls.push(Box::pin(async { Ok(session.changed_files(0).await?.len()) }));
    }

    let answers = first_gives_up(&fx, &heard, "Diffing the changes", calls).await;

    assert!(answers.into_iter().all(|a| a.unwrap() == 1));
    assert_eq!(fx.git.diffs.load(Ordering::Relaxed), 1, "one diff");
    assert_eq!(started(&heard, "Diffing the changes"), 1, "one task");
}

#[tokio::test]
async fn the_requests_for_one_interdiff_share_it_and_one_giving_up_does_not_start_it_again() {
    let base = numbered(30, &[]);
    let (fx, heard) = announcing(&[("a.txt", &base)], &[("a.txt", &numbered(30, &[3]))]).await;
    fx.begin().await;
    fx.settle(&[("a.txt", &numbered(30, &[3, 20]))]).await;
    fx.session.manifest(View::Changeset).await.unwrap();
    let between = View::Interdiff { from: 1, to: 2 };
    let calls: Vec<Call<'_, usize>> = (0..8)
        .map(|_| -> Call<'_, usize> { Box::pin(async { Ok(fx.session.manifest(between).await?.files.len()) }) })
        .collect();

    let answers = first_gives_up(&fx, &heard, "Comparing versions", calls).await;

    assert!(answers.into_iter().all(|a| a.unwrap() == 1));
    assert_eq!(started(&heard, "Comparing versions"), 1, "one interdiff");
    fx.session.manifest(between).await.unwrap();
    assert_eq!(started(&heard, "Comparing versions"), 1, "the interdiff is kept");
}

#[tokio::test]
async fn the_requests_for_the_commits_share_them_and_one_giving_up_does_not_start_them_again() {
    let base = numbered(30, &[]);
    let (fx, heard) = announcing(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fx.git.feature_commits(
        &[("a.txt", &base)],
        &[
            ("c1", &[("a.txt", &numbered(30, &[3]))]),
            ("c2", &[("a.txt", &numbered(30, &[3, 20]))]),
        ],
    );
    view_feature(&fx).await;
    fx.session.manifest(View::Changeset).await.unwrap();
    let diffs = fx.git.diffs.load(Ordering::Relaxed);
    let calls: Vec<Call<'_, usize>> = (0..8)
        .map(|_| -> Call<'_, usize> { Box::pin(async { Ok(fx.session.commits().await?.commits.len()) }) })
        .collect();

    let answers = first_gives_up(&fx, &heard, "Comparing the commits", calls).await;

    assert!(answers.into_iter().all(|a| a.unwrap() == 2));
    assert_eq!(started(&heard, "Comparing the commits"), 1);
    assert_eq!(
        fx.git.diffs.load(Ordering::Relaxed),
        diffs + 2,
        "each commit diffed once"
    );
}

#[tokio::test]
async fn a_filter_change_a_started_and_a_finished_review_do_not_diff_again() {
    let base = numbered(30, &[]);
    let fx = Fixture::open(
        &[("a.txt", &base), ("b.txt", "b\n")],
        &[("a.txt", &numbered(30, &[3, 20])), ("b.txt", "b, edited\n")],
    )
    .await;
    fx.session.manifest(View::Changeset).await.unwrap();
    assert_eq!(fx.git.diffs.load(Ordering::Relaxed), 1);

    // Marking starts the review, whose first version is what the changeset changes.
    fx.session.toggle_hunk(0, View::Changeset, "a.txt", 0).await.unwrap();
    let revision = fx.session.exclude_path("b.txt", false).await.unwrap();
    let filtered = fx.session.manifest(View::Changeset).await.unwrap();
    fx.session.changed_files(revision).await.unwrap();
    assert!(!fx.session.finish_review().await.unwrap(), "b.txt's hunk is not seen");
    let browsed = fx.session.manifest(View::Changeset).await.unwrap();

    assert_eq!(
        fx.git.diffs.load(Ordering::Relaxed),
        1,
        "the pair of snapshots was diffed once"
    );
    assert_eq!(filtered.files.len(), 1);
    assert_eq!(filtered.completion.seen_hunks, 1);
    assert_eq!(browsed.completion.seen_hunks, 0, "the finished review's marks are gone");
    let versions = fx.store.0.lock().unwrap().versions.clone();
    let first: Vec<_> = versions[0]
        .change
        .files
        .iter()
        .map(|f| (f.path.clone(), f.delta))
        .collect();
    assert_eq!(
        first,
        [
            (path("a.txt"), LineDelta { added: 2, removed: 2 }),
            (path("b.txt"), LineDelta { added: 1, removed: 1 })
        ],
        "the first version changed what the changeset changes"
    );
}
