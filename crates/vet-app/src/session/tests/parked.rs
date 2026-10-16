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
        let tree = held.parked().get(&Source::WorkingTree).expect("the tree is parked");
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
            held.parked().get(&Source::WorkingTree).unwrap().changes.is_none(),
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
            held.parked().get(&Source::WorkingTree).unwrap().changes.is_some(),
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
