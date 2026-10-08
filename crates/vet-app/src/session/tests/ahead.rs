//! The working tree's content copied ahead of a review: the first mark copies only what the
//! copy has not, and joins a copy under way rather than copy the same files again.

use std::collections::BTreeSet;
use std::task::{Context, Waker};

use super::*;

/// The blobs put into the store so far.
fn put(fx: &Fixture) -> Vec<BlobId> {
    fx.store.0.lock().unwrap().put.clone()
}

#[tokio::test]
async fn after_the_tree_settled_and_was_copied_the_first_mark_copies_nothing() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[3]))]).await;
    let (edited, added) = (numbered(10, &[3, 7]), "n\n");
    fx.settle(&[("a.txt", &edited), ("new.txt", added)]).await;

    fx.session.copy_ahead().await.unwrap();
    let copied = put(&fx);
    assert_eq!(
        BTreeSet::from_iter(copied.iter().copied()),
        BTreeSet::from([BlobId::of(edited.as_bytes()), BlobId::of(added.as_bytes())])
    );

    let marked = fx.session.set_seen(1, "a.txt", &[0], true).await.unwrap();
    assert!(matches!(marked.started, Some(Lifecycle::Open { version: 1, .. })));
    assert_eq!(put(&fx), copied, "the first mark found the content in the store");
}

#[tokio::test]
async fn the_first_mark_copies_only_what_changed_since_the_copy() {
    let base = numbered(10, &[]);
    let added = "n\n";
    let fx = Fixture::open(
        &[("a.txt", &base)],
        &[("a.txt", &numbered(10, &[3])), ("new.txt", added)],
    )
    .await;
    fx.session.copy_ahead().await.unwrap();
    let copied = put(&fx).len();
    let edited = numbered(10, &[3, 7]);
    let revision = fx.settle(&[("a.txt", &edited), ("new.txt", added)]).await;

    fx.session.set_seen(revision, "a.txt", &[0], true).await.unwrap();

    assert_eq!(put(&fx)[copied..], [BlobId::of(edited.as_bytes())]);
}

#[tokio::test]
async fn a_mark_while_the_tree_is_copied_waits_for_that_copy_and_no_file_is_copied_twice() {
    let base = numbered(10, &[]);
    let dirty = [numbered(10, &[3]), numbered(10, &[5]), "n\n".to_owned()];
    let fx = Fixture::open(
        &[("a.txt", &base), ("b.txt", &base)],
        &[("a.txt", &dirty[0]), ("b.txt", &dirty[1]), ("new.txt", &dirty[2])],
    )
    .await;
    fx.session.manifest(View::Changeset).await.unwrap();

    let paused = fx.git.paused.lock().await;
    let mut copy = Box::pin(fx.session.copy_ahead());
    let mut mark = Box::pin(fx.session.set_seen(0, "a.txt", &[0], true));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(copy.as_mut().poll(&mut cx).is_pending(), "the copy reads the tree");
    assert!(mark.as_mut().poll(&mut cx).is_pending(), "the mark waits for the copy");
    drop(paused);
    let (copied, marked) = futures::future::join(copy, mark).await;
    copied.unwrap();
    assert!(marked.unwrap().started.is_some());

    let mut put = put(&fx);
    put.sort();
    let mut want: Vec<BlobId> = dirty.iter().map(|d| BlobId::of(d.as_bytes())).collect();
    want.sort();
    assert_eq!(put, want, "each dirty file copied once");
}
