//! Parking a changeset: what it drops while off view, and what its return rebuilds.
//! A parked working tree whose review is open keeps settling and rebuilding what it needs, so
//! the queue's progress follows it; a changeset that comes back rebuilds its diff lazily.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::*;

#[tokio::test]
async fn switching_away_leaves_the_engine_the_only_holder_of_the_diff() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    // A mark keeps the target it was made against referenced, so the state holds it beside the
    // diff once the tree settles past it.
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    fx.settle(&[("a.txt", "three\n")]).await;
    let diffed = {
        let held = fx.session.hold().await;
        assert_eq!(held.known.len(), 1, "the target the mark was made against is kept");
        Arc::clone(&held.changes().diffed)
    };

    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "branch\n")]);
    view_feature(&fx).await;

    {
        let mut held = fx.session.hold().await;
        let tree = held.parked().tree().expect("the tree is parked");
        assert!(tree.changes.is_none(), "a parked tree keeps no diff");
        assert!(tree.known.is_empty(), "a parked tree keeps no snapshot");
    }
    assert_eq!(
        Arc::strong_count(&diffed),
        2,
        "only the engine and this test hold the diff"
    );
}

#[tokio::test]
async fn a_parked_reviewed_tree_settles_and_rebuilds_its_diff() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "branch\n")]);
    view_feature(&fx).await;
    assert!(fx.session.settle_wanted().get(), "the parked tree's review is open");
    let versions = fx.store.reviewed_versions().len();
    let diffs = fx.git.diffs.load(Ordering::Relaxed);
    {
        let mut held = fx.session.hold().await;
        assert!(
            held.parked().tree().unwrap().changes.is_none(),
            "parking dropped the tree's diff"
        );
    }

    for (i, text) in ["three\n", "four\n"].into_iter().enumerate() {
        fx.settle(&[("a.txt", text)]).await;
        assert_eq!(
            fx.store.reviewed_versions().len(),
            versions + i + 1,
            "each settle is the next version"
        );
        let mut held = fx.session.hold().await;
        assert!(
            held.parked().tree().unwrap().changes.is_some(),
            "the settle rebuilt the diff the queue shows progress from"
        );
    }
    assert_eq!(
        fx.git.diffs.load(Ordering::Relaxed),
        diffs + 2,
        "each parked settle diffs the snapshot it settles"
    );
    assert_eq!(fx.session.source().await, feature(), "the branch stays on view");
}

#[tokio::test]
async fn coming_back_rebuilds_the_diff_once_and_shows_the_same_review() {
    let base = numbered(10, &[]);
    let fx = Fixture::reviewing(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[0]))]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen]);

    fx.git.feature(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[1]))]);
    view_feature(&fx).await;
    // Parking dropped the state's diff; the engine's copy goes too, as eviction would leave it.
    fx.session.derived.diffs.clear();
    let diffs = fx.git.diffs.load(Ordering::Relaxed);
    assert!(fx.session.open_changeset(Source::WorkingTree).await.unwrap().is_some());

    let first = fx.session.manifest(View::Changeset).await.unwrap();
    assert_eq!(
        fx.git.diffs.load(Ordering::Relaxed),
        diffs + 1,
        "the diff is computed once"
    );
    let again = fx.session.manifest(View::Changeset).await.unwrap();
    assert_eq!(fx.git.diffs.load(Ordering::Relaxed), diffs + 1, "and not again");
    assert_eq!(again.revision, first.revision);
    assert_eq!(
        fx.hunks("a.txt").await,
        [HunkState::Seen],
        "the review state is what it was"
    );
}

/// A manifest as it is compared across a switch: the revision and the project's change counter
/// move with the switch on their own.
fn plain(mut manifest: ManifestView) -> ManifestView {
    manifest.revision = 0;
    manifest.seq = 0;
    manifest
}

/// Leaving `source` for the working tree and coming back, with the revision the return reports.
async fn leave_and_return(fx: &Fixture, source: Source) -> u32 {
    assert!(fx.session.open_changeset(Source::WorkingTree).await.unwrap().is_some());
    fx.session.open_changeset(source).await.unwrap().unwrap()
}

#[tokio::test]
async fn a_browsing_branch_with_a_thread_comes_back_the_same_with_a_higher_revision() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fx.git.feature(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[3]))]);
    let revision = view_feature(&fx).await;
    let id = fx.thread(on("a.txt", 2, 4)).await;
    fx.session.finish_review().await.unwrap();

    let before = plain(fx.session.manifest(View::Changeset).await.unwrap());
    let before_threads = fx.session.threads(View::Changeset).await.unwrap().threads;

    let back = leave_and_return(&fx, feature()).await;

    assert!(back > revision);
    assert_eq!(plain(fx.session.manifest(View::Changeset).await.unwrap()), before);
    assert_eq!(
        fx.session.threads(View::Changeset).await.unwrap().threads,
        before_threads
    );
    assert_eq!(before_threads[0].id, id, "the open thread belongs to the changeset");
    assert!(
        fx.session.versions().await.unwrap().is_empty(),
        "browsing keeps no version"
    );
}

#[tokio::test]
async fn a_reviewed_branch_comes_back_with_the_same_review_and_a_higher_revision() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(
        &[("a.txt", &base), ("x.bin", "a\0\n")],
        &[("a.txt", &base), ("x.bin", "a\0\n")],
    )
    .await;
    fx.git.feature(
        &[("a.txt", &base), ("x.bin", "a\0\n")],
        &[("a.txt", &numbered(10, &[3])), ("x.bin", "b\0\n")],
    );
    let revision = view_feature(&fx).await;
    fx.session.set_seen(revision, "a.txt", &[0], true).await.unwrap();
    let revision = fx.session.revision().await;
    fx.session
        .acknowledge(revision, "x.bin", NonTextKind::Binary, true)
        .await
        .unwrap();
    let id = fx.thread(on("a.txt", 2, 4)).await;
    fx.session.set_lens(Lens::PerCommit).await.unwrap();

    let before = plain(fx.session.manifest(View::Changeset).await.unwrap());
    let before_threads = fx.session.threads(View::Changeset).await.unwrap().threads;
    let before_versions = fx.session.versions().await.unwrap();
    assert_eq!(before.lens, Lens::PerCommit);
    assert!(!before_versions.is_empty(), "an open review keeps its versions");

    let back = leave_and_return(&fx, feature()).await;

    assert!(back > revision);
    assert_eq!(plain(fx.session.manifest(View::Changeset).await.unwrap()), before);
    assert_eq!(
        fx.session.threads(View::Changeset).await.unwrap().threads,
        before_threads
    );
    assert_eq!(fx.session.versions().await.unwrap(), before_versions);
    assert_eq!(before_threads[0].id, id);
}

#[tokio::test]
async fn a_pull_request_with_a_draft_comes_back_the_same_with_a_higher_revision() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fetch_pull_request_7(&fx.git, &[("a.txt", &base)], "pr1", &[("a.txt", &numbered(10, &[3]))]);
    let revision = fx.session.open_changeset(pull_request_7()).await.unwrap().unwrap();
    let id = fx.thread(on("a.txt", 2, 4)).await;

    let before = plain(fx.session.manifest(View::Changeset).await.unwrap());
    let before_threads = fx.session.threads(View::Changeset).await.unwrap().threads;
    let before_versions = fx.session.versions().await.unwrap();
    assert!(
        before_threads[0].platform.as_ref().unwrap().remote_id.is_none(),
        "the thread is a draft the platform does not have"
    );

    let back = leave_and_return(&fx, pull_request_7()).await;

    assert!(back > revision);
    assert_eq!(plain(fx.session.manifest(View::Changeset).await.unwrap()), before);
    assert_eq!(
        fx.session.threads(View::Changeset).await.unwrap().threads,
        before_threads
    );
    assert_eq!(fx.session.versions().await.unwrap(), before_versions);
    assert_eq!(before_threads[0].id, id);
}

#[tokio::test]
async fn only_the_working_tree_is_kept_while_ten_branches_are_visited() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    // A default branch for each branch to have left.
    fx.git.feature(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[0]))]);
    for i in 0..10 {
        fx.git.branch(
            &format!("b{i}"),
            &format!("tip{i}"),
            &[("a.txt", &numbered(10, &[i]))],
            &["m1"],
        );
    }

    for i in 0..10 {
        let revision = fx
            .session
            .open_changeset(Source::Branch { name: format!("b{i}") })
            .await
            .unwrap()
            .unwrap();
        fx.session.set_seen(revision, "a.txt", &[0], true).await.unwrap();
        fx.session.manifest(View::Changeset).await.unwrap();
        let diffed = Arc::clone(&fx.session.hold().await.changes().diffed);
        {
            let mut held = fx.session.hold().await;
            assert_eq!(
                held.parked().tree().unwrap().source,
                Source::WorkingTree,
                "only the working tree is kept while a branch is on view"
            );
        }

        fx.session.open_changeset(Source::WorkingTree).await.unwrap();

        assert_eq!(Arc::strong_count(&diffed), 2, "branch {i}'s state is let go, not kept");
        let mut held = fx.session.hold().await;
        assert!(held.parked().tree().is_none(), "the branch that was left is not kept");
        drop(held);
    }
}

#[tokio::test]
async fn coming_back_writes_no_ref_that_exists() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fx.git.feature(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[3]))]);
    let revision = view_feature(&fx).await;
    fx.session.set_seen(revision, "a.txt", &[0], true).await.unwrap();
    let refs = fx.git.refs.lock().unwrap().clone();
    let writes = fx.git.ref_writes.load(Ordering::Relaxed);
    assert!(!refs.is_empty(), "the review kept its commits");

    let back = leave_and_return(&fx, feature()).await;

    assert!(back > revision);
    assert_eq!(
        fx.git.ref_writes.load(Ordering::Relaxed),
        writes,
        "the refs are already there, so coming back writes none"
    );
    assert_eq!(fx.git.refs.lock().unwrap().clone(), refs);
}
