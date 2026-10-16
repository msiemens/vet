//! Copying the working tree's content into the store in the background, so the first mark of a
//! review finds it there. Only while the review screen shows the changeset: on the queue nothing is
//! copied, and a review screen that comes back copies once. A tree that settles again ends the
//! copy of the one before.

use std::sync::Arc;

use crate::Open;

/// The kind and slot of the run that copies the working tree's content.
pub(crate) const KIND: &str = "copy";

/// Starts the run that copies the tree on view. It replaces an older run; the supervisor stops it
/// when a tree settles, the review screen goes away or the project closes.
pub(crate) fn start(open: &Arc<Open>) {
    open.supervisor
        .replace(KIND, open.supervisor.track(KIND, run(open.clone())));
}

async fn run(open: Arc<Open>) {
    let Ok(booted) = open.booted().await else {
        return;
    };
    if let Err(e) = booted.session.copy_ahead().await {
        tracing::warn!(error = %e, "could not copy the working tree's content ahead of a review");
    }
}
