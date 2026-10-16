//! The connection type that receives a project's protocol events, and the event
//! every part of the backend sends.

use std::sync::Arc;

use vet_app::registry::{Subscriber, Subscribers};
use vet_protocol::{Event, ProjectId};
use vet_transport::Events;

use crate::Open;

/// A client connection, subscribed to the events of the projects it opened.
#[derive(Clone)]
pub(crate) struct Connection(pub Events);

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

/// Tells every subscriber that the snapshot moved and the changeset is at `revision`; `version` is
/// the open review's latest version, none while browsing. The background work that follows the
/// revision is the supervisor's.
pub(crate) fn snapshot_moved(
    subscribers: &Subscribers<Connection>,
    project: ProjectId,
    revision: u32,
    version: Option<u32>,
) {
    subscribers.broadcast(Event::SnapshotMoved {
        project,
        revision,
        version,
    });
}

/// A change of the filter moved the revision for everyone, though no snapshot moved.
pub(crate) fn filter_changed(open: &Arc<Open>, revision: u32) {
    open.subscribers.broadcast(Event::FilterChanged {
        project: open.id,
        revision,
    });
}

/// The review ended and the changeset is at `revision`, browsing.
pub(crate) fn review_finished(open: &Arc<Open>, revision: u32) {
    open.subscribers.broadcast(Event::ReviewFinished {
        project: open.id,
        revision,
    });
}

/// Tells every subscriber whether edits of the working tree wait to settle into a snapshot.
pub(crate) fn edits_pending(subscribers: &Subscribers<Connection>, project: ProjectId, pending: bool) {
    subscribers.broadcast(Event::EditsPending { project, pending });
}
