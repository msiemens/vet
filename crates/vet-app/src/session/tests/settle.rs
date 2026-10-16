//! The settle loop on the working tree: while it is parked and has no open review, file changes
//! only mark it changed — no scan, no diff — and coming on view settles it once. A parked tree
//! whose review is open keeps settling, so every settled snapshot stays its next version.

use std::future::Future;

use super::*;
use crate::ports::{Clock, Degraded, Millis, WatchEvent, Watcher};
use crate::settle::Settling;

/// Watcher events fed by the test.
struct Channel {
    events: tokio::sync::mpsc::UnboundedReceiver<WatchEvent>,
}

impl Watcher for Channel {
    fn degraded(&self) -> Option<Degraded> {
        None
    }

    fn next(&mut self) -> impl Future<Output = WatchEvent> + Send {
        let events = &mut self.events;
        async move { events.recv().await.expect("the test holds the sender") }
    }
}

/// A debounce that has always already passed, so a settle follows the edit without waiting.
struct Immediate;

impl Clock for Immediate {
    fn now(&self) -> Millis {
        Millis(0)
    }

    fn sleep_until(&self, _: Millis) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }
}

type Sender = tokio::sync::mpsc::UnboundedSender<WatchEvent>;

/// Runs `fx`'s settle loop over a watcher the test feeds, the tree having started at `settled`;
/// answers the sender of watcher events.
fn follow(fx: &Fixture, settled: SnapshotId) -> (tokio::task::JoinHandle<()>, Sender) {
    let (send, events) = tokio::sync::mpsc::unbounded_channel();
    let settling = Settling::new(Channel { events }, Immediate, 0, Some(settled)).wanted(fx.session.settle_wanted());
    let session = fx.session.clone();
    let follower = tokio::spawn(async move {
        session.follow(settling, |_| {}, || {}, |_| {}).await;
    });
    (follower, send)
}

fn dirty(name: &str) -> WatchEvent {
    WatchEvent::Dirty(vec![path(name)])
}

/// Lets the settle loop run a step.
async fn tick() {
    tokio::task::yield_now().await;
}

/// Lets the settle loop run until `cond` holds.
async fn until(cond: impl Fn() -> bool, what: &str) {
    for _ in 0..200 {
        if cond() {
            return;
        }
        tick().await;
    }
    panic!("the settle loop never reached {what}");
}

#[tokio::test]
async fn a_parked_tree_without_a_review_is_not_scanned_until_it_comes_on_view() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[3]))]).await;
    fx.git.feature(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[1]))]);
    let tree_target = fx.session.target_snapshot().await.id;
    fx.session.open_changeset(feature()).await.unwrap().unwrap();
    assert!(!fx.session.settle_wanted().get(), "the tree is parked and not reviewed");

    // An edit of the parked tree is neither scanned nor diffed.
    let edited = numbered(10, &[3, 7]);
    fx.git.queue_scan(&[("a.txt", &edited)]);
    let (follower, events) = follow(&fx, tree_target);
    events.send(dirty("a.txt")).unwrap();
    for _ in 0..20 {
        tick().await;
    }
    assert_eq!(fx.git.scans.lock().unwrap().len(), 1, "no scan while parked");
    assert_eq!(fx.git.diffs.load(Ordering::Relaxed), 0, "no diff while parked");

    // Coming on view scans it once and settles, so it shows the change.
    assert!(fx.session.open_changeset(Source::WorkingTree).await.unwrap().is_some());
    until(|| fx.git.diffs.load(Ordering::Relaxed) == 1, "the tree is diffed").await;
    for _ in 0..5 {
        tick().await;
    }
    assert!(fx.git.scans.lock().unwrap().is_empty(), "the one scan was taken");
    assert_eq!(
        fx.session.target_snapshot().await.id,
        snapshot(&[("a.txt", &edited)]).id,
        "the settled tree shows the edit"
    );
    follower.abort();
}

#[tokio::test]
async fn a_parked_tree_with_an_open_review_records_a_version_for_each_settle() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    let tree_target = fx.session.target_snapshot().await.id;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "branch\n")]);
    fx.session.open_changeset(feature()).await.unwrap().unwrap();
    assert!(fx.session.settle_wanted().get(), "the parked tree's review is open");
    let before = fx.store.reviewed_versions().len();

    let (follower, events) = follow(&fx, tree_target);
    for (i, text) in ["three\n", "four\n"].into_iter().enumerate() {
        fx.git.queue_scan(&[("a.txt", text)]);
        events.send(dirty("a.txt")).unwrap();
        until(
            || fx.store.reviewed_versions().len() == before + i + 1,
            "the next version",
        )
        .await;
    }
    assert_eq!(
        fx.store.reviewed_versions().len(),
        before + 2,
        "each settle is a version"
    );
    assert_eq!(fx.session.source().await, feature(), "the branch stays on view");
    follower.abort();
}
