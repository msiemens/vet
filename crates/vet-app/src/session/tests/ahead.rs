//! The working tree's content copied ahead of a review: the first mark copies only what the
//! copy has not, and joins a copy under way rather than copy the same files again.

use std::collections::BTreeSet;
use std::task::{Context, Waker};
use std::time::Duration;

use super::derived::announcing;
use super::*;
use crate::progress::{Task, TaskKind, TaskState};

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
async fn a_manifest_asked_while_the_first_mark_waits_for_the_copy_is_answered_meanwhile() {
    let base = numbered(10, &[]);
    // The binary file is not among the diff's texts, so the copy reads it, which the test holds.
    let fx = Fixture::open(
        &[("a.txt", &base)],
        &[("a.txt", &numbered(10, &[3])), ("c.bin", "\0bin\n")],
    )
    .await;
    fx.session.manifest(View::Changeset).await.unwrap();

    let paused = fx.git.paused.lock().await;
    let mut copy = Box::pin(fx.session.copy_ahead());
    let mut mark = Box::pin(fx.session.toggle_hunk(0, View::Changeset, "a.txt", 0));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(copy.as_mut().poll(&mut cx).is_pending(), "the copy reads the tree");
    assert!(mark.as_mut().poll(&mut cx).is_pending(), "the mark waits for the copy");

    let manifest = tokio::time::timeout(Duration::from_secs(10), fx.session.manifest(View::Changeset)).await;
    assert_eq!(manifest.expect("answered while the mark waits").unwrap().files.len(), 2);
    assert_eq!(fx.session.lifecycle().await, Lifecycle::Browsing);

    drop(paused);
    let (copied, marked) = futures::future::join(copy, mark).await;
    copied.unwrap();
    assert!(matches!(
        marked.unwrap().started,
        Some(Lifecycle::Open { version: 1, .. })
    ));
}

#[tokio::test]
async fn a_copy_is_a_task_that_counts_the_files_it_stores_and_ends() {
    let base = numbered(10, &[]);
    let (fx, heard) = announcing(
        &[("a.txt", &base)],
        &[("a.txt", &numbered(10, &[3])), ("new.txt", "n\n")],
    )
    .await;

    fx.session.copy_ahead().await.unwrap();

    let heard = heard.lock().unwrap();
    let copy: Vec<&Task> = heard.iter().filter(|t| t.kind == TaskKind::Store).collect();
    assert!(!copy.is_empty(), "the copy was announced");
    assert!(
        copy.iter()
            .all(|t| t.id == copy[0].id && t.title == "Storing changed files")
    );
    assert_eq!(copy[0].state, TaskState::Running);
    let last = copy.last().unwrap();
    assert_eq!(
        (last.done, last.total, &last.state),
        (Some(2), Some(2), &TaskState::Finished)
    );
}

#[tokio::test]
async fn the_copy_takes_the_texts_the_diff_read_and_reads_only_what_the_diff_does_not_hold() {
    let base = numbered(10, &[]);
    let binary = "\0bin\n";
    let dirty = [numbered(10, &[3]), numbered(10, &[5])];
    let fx = Fixture::open(
        &[("a.txt", &base), ("b.txt", &base)],
        &[("a.txt", &dirty[0]), ("b.txt", &dirty[1]), ("c.bin", binary)],
    )
    .await;
    fx.session.manifest(View::Changeset).await.unwrap();
    fx.git.reads.lock().unwrap().clear();

    fx.session.copy_ahead().await.unwrap();

    assert_eq!(*fx.git.reads.lock().unwrap(), [BlobId::of(binary.as_bytes())]);
    let mut want: Vec<BlobId> = [&dirty[0], &dirty[1], binary].map(|d| BlobId::of(d.as_bytes())).into();
    want.sort();
    let mut put = put(&fx);
    put.sort();
    assert_eq!(put, want);
}

#[tokio::test]
async fn a_copy_whose_content_is_not_what_the_tree_says_fails_and_the_first_mark_with_it() {
    let base = numbered(10, &[]);
    let dirty = numbered(10, &[3]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    // The file changed after it was hashed: what is read under its id is other content.
    let id = BlobId::of(dirty.as_bytes());
    fx.git.blobs.lock().unwrap().insert(id, numbered(10, &[4]).into_bytes());

    let copied = fx.session.copy_ahead().await;
    assert!(matches!(copied, Err(SessionError::Git(e)) if e.message().contains("changed while being copied")));
    assert!(fx.session.set_seen(0, "a.txt", &[0], true).await.is_err());
    assert_eq!(fx.session.lifecycle().await, Lifecycle::Browsing);
}

#[tokio::test]
async fn a_mark_while_the_tree_is_copied_waits_for_that_copy_and_no_file_is_copied_twice() {
    let base = numbered(10, &[]);
    // The binary file is not among the diff's texts, so the copy reads it.
    let dirty = [numbered(10, &[3]), numbered(10, &[5]), "\0n\n".to_owned()];
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
