//! Reading a commit's tree: the session's flight shares one snapshot between the places that ask
//! for it, joins the reads under way, and keeps a bounded number of them.

use std::pin::Pin;
use std::task::{Context, Waker};

use futures::future::join_all;

use super::*;
use crate::session::derived::KEPT_SNAPSHOTS;

type Call<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + 'a>>;

#[tokio::test]
async fn two_branches_off_one_base_share_their_base_snapshot() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fx.git.feature(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[0]))]);
    fx.git.branch("b1", "tip1", &[("a.txt", &numbered(10, &[1]))], &["m1"]);
    fx.git.branch("b2", "tip2", &[("a.txt", &numbered(10, &[2]))], &["m1"]);
    let branch = |name: &str| Source::Branch { name: name.into() };

    let _ = fx.session.open_changeset(branch("b1")).await.unwrap();
    let first = Arc::clone(&fx.session.hold().await.base);

    let _ = fx.session.open_changeset(Source::WorkingTree).await.unwrap();
    let _ = fx.session.open_changeset(branch("b2")).await.unwrap();
    let second = Arc::clone(&fx.session.hold().await.base);

    assert!(
        Arc::ptr_eq(&first, &second),
        "both branches are rebuilt against the one merge-base tree"
    );
}

#[tokio::test]
async fn concurrent_reads_of_one_commit_walk_its_tree_once() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fx.git.add_commit("c1", "c1", &[("a.txt", &numbered(10, &[1]))]);
    let commit = CommitId("c1".into());
    let before = fx.git.snapshot_reads.load(Ordering::Relaxed);

    let paused = fx.git.paused.lock().await;
    let mut calls: Vec<Call<'_, Arc<Snapshot>>> = (0..3)
        .map(|_| -> Call<'_, Arc<Snapshot>> { Box::pin(fx.session.facts.snapshot_of(&commit)) })
        .collect();
    let mut cx = Context::from_waker(Waker::noop());
    for call in &mut calls {
        assert!(call.as_mut().poll(&mut cx).is_pending(), "the first read is under way");
    }
    assert_eq!(
        fx.git.snapshot_reads.load(Ordering::Relaxed),
        before + 1,
        "the requests join one tree walk"
    );
    drop(paused);

    let answers = join_all(calls).await;
    assert!(answers.iter().all(|a| a.is_ok()));
    assert!(Arc::ptr_eq(answers[0].as_ref().unwrap(), answers[1].as_ref().unwrap()));
}

/// More commits than the flight keeps, so the earliest are surely beyond its small limit.
const MORE_THAN_KEPT: usize = 20;

#[tokio::test]
async fn the_snapshot_flight_evicts_what_it_cannot_keep() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    let commit = |i: usize| CommitId(format!("c{i}"));
    for i in 0..MORE_THAN_KEPT {
        let text = format!("c{i}\n");
        fx.git
            .add_commit(&format!("c{i}"), &format!("c{i}"), &[("a.txt", &text)]);
    }
    for i in 0..MORE_THAN_KEPT {
        fx.session.facts.snapshot_of(&commit(i)).await.unwrap();
    }
    let reads = fx.git.snapshot_reads.load(Ordering::Relaxed);

    fx.session.facts.snapshot_of(&commit(0)).await.unwrap();
    assert_eq!(
        fx.git.snapshot_reads.load(Ordering::Relaxed),
        reads + 1,
        "the oldest read is beyond the limit and was evicted"
    );
    fx.session.facts.snapshot_of(&commit(MORE_THAN_KEPT - 1)).await.unwrap();
    assert_eq!(
        fx.git.snapshot_reads.load(Ordering::Relaxed),
        reads + 1,
        "the most recent read is still kept"
    );
}

#[tokio::test]
async fn a_snapshot_over_the_byte_cap_is_evicted_before_the_entry_limit() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    // Each tree's path alone weighs more than a third of the cap, so three of them outweigh it
    // while the flight may still keep sixteen.
    let each = KEPT_SNAPSHOTS.weight / 3 + (1 << 20);
    let path = "x".repeat(each);
    for i in 0..3 {
        let id = format!("big{i}");
        fx.git.add_commit(&id, &id, &[(path.as_str(), "x\n")]);
    }
    let commit = |i: usize| CommitId(format!("big{i}"));
    for i in 0..3 {
        fx.session.facts.snapshot_of(&commit(i)).await.unwrap();
    }
    let reads = fx.git.snapshot_reads.load(Ordering::Relaxed);

    // Three trees outweigh the cap, so the first was evicted although only three are kept.
    fx.session.facts.snapshot_of(&commit(0)).await.unwrap();
    assert_eq!(
        fx.git.snapshot_reads.load(Ordering::Relaxed),
        reads + 1,
        "the first was evicted by its weight, not by the entry count"
    );
    fx.session.facts.snapshot_of(&commit(2)).await.unwrap();
    assert_eq!(
        fx.git.snapshot_reads.load(Ordering::Relaxed),
        reads + 1,
        "the last read is still kept"
    );
}
