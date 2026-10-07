//! Telling a project's subscribers what happened: the protocol events, and the
//! connection type that receives them.

use vet_app::registry::{Subscriber, Subscribers};
use vet_app::session::{Lifecycle, ReviewUpdateView, ThreadView, View};
use vet_mapping as mapping;
use vet_protocol::{Event, ProjectId, Response};
use vet_transport::Events;

use crate::open::Ready;

/// A client connection, subscribed to the events of the projects it opened.
#[derive(Clone)]
pub struct Connection(pub Events);

impl Subscriber for Connection {
    type Event = Event;

    fn send(&self, event: Event) {
        self.0.send(event);
    }

    fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    fn same_connection(&self, other: &Self) -> bool {
        self.0.same_connection(&other.0)
    }
}

/// Tells every subscriber that the project's snapshot is a new `revision`.
pub fn settled(subscribers: &Subscribers<Connection>, project: ProjectId, revision: u32) {
    subscribers.broadcast(Event::SnapshotSettled { project, revision });
}

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
