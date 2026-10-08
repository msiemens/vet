//! The connection type that receives a project's protocol events, and the event
//! every part of the backend sends.

use vet_app::registry::{Subscriber, Subscribers};
use vet_protocol::{Event, ProjectId};
use vet_transport::Events;

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
