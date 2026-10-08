//! Telling a project's subscribers what a request changed.

use vet_app::session::{Lifecycle, ReviewUpdateView, ThreadView, View};
use vet_mapping as mapping;
use vet_protocol::{Event, Response};

use crate::events::Connection;
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

/// Tells the other subscribers that threads changed.
pub async fn threads_changed(open: &Ready, from: &Connection) {
    let revision = open.session.revision().await;
    open.subscribers.broadcast_except(
        from,
        Event::ThreadsChanged {
            project: open.id,
            revision,
        },
    );
}

pub async fn thread_changed(open: &Ready, from: &Connection, thread: &ThreadView) -> Response {
    threads_changed(open, from).await;
    Response::Thread(mapping::thread(thread))
}

/// The protocol update for the requester; the other subscribers are told too. A mark made in an
/// interdiff or in a commit can change the changeset, whose file everyone shows, the requester
/// included.
pub async fn review_changed(
    open: &Ready,
    from: &Connection,
    view: View,
    update: &ReviewUpdateView,
) -> vet_protocol::ReviewUpdate {
    let mapped = mapping::review_update(update);
    match view {
        View::Changeset => open.subscribers.broadcast_except(
            from,
            Event::ReviewChanged {
                project: open.id,
                revision: update.revision,
                update: mapped.clone(),
            },
        ),
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
