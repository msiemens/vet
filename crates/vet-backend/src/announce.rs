//! Telling a project's subscribers what a request changed. Every subscriber is told, the
//! requester included: only the store that asked reads the answer, and the requester's other
//! stores learn of the change from the event.

use vet_app::session::{Lifecycle, ReviewUpdateView, ThreadView, View};
use vet_mapping as mapping;
use vet_protocol::{Event, Response};

use crate::open::Ready;

/// Tells every subscriber that a request started the review, as `started` says.
pub fn announce_start(open: &Ready, started: Option<Lifecycle>) {
    if let Some(lifecycle) = started {
        open.subscribers.broadcast(Event::ReviewStarted {
            project: open.id,
            review: mapping::review_info(lifecycle),
        });
    }
}

/// Tells every subscriber that threads changed.
pub async fn threads_changed(open: &Ready) {
    let revision = open.session.revision().await;
    open.subscribers.broadcast(Event::ThreadsChanged {
        project: open.id,
        revision,
    });
}

pub async fn thread_changed(open: &Ready, thread: &ThreadView) -> Response {
    threads_changed(open).await;
    Response::Thread(mapping::thread(thread))
}

/// The protocol update for the requester; every subscriber is told of the changeset's file. A
/// mark made in an interdiff or in a commit changes the changeset's file too, which is what
/// is announced then.
pub async fn review_changed(open: &Ready, view: View, update: &ReviewUpdateView) -> vet_protocol::ReviewUpdate {
    let mapped = mapping::review_update(update);
    match view {
        View::Changeset => open.subscribers.broadcast(Event::ReviewChanged {
            project: open.id,
            revision: update.revision,
            update: mapped.clone(),
        }),
        View::Interdiff { .. } | View::Commit { .. } => {
            if let Ok(changeset) = open.session.changeset_update(&mapped.path).await {
                open.subscribers.broadcast(Event::ReviewChanged {
                    project: open.id,
                    revision: changeset.revision,
                    update: mapping::review_update(&changeset),
                });
            }
        }
    }
    mapped
}
