//! The handshake and the frames around requests, responses and events.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Event, Request, Response};

pub const PROTOCOL_VERSION: u32 = 1;

/// The one line the backend prints to stdout once it is listening. The shell
/// reads it to learn the port and the per-launch random token.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Ready {
    pub port: u16,
    pub token: String,
}

/// First message on every connection must be `Hello` with the token from
/// [`Ready`]; there is no unauthenticated endpoint.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ClientMessage {
    Hello { token: String, protocol_version: u32 },
    Request { id: u32, request: Request },
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ServerMessage {
    HelloAck {
        protocol_version: u32,
    },
    Response {
        id: u32,
        response: Response,
    },
    /// `id` is `None` when the failure is not attributable to a request.
    Error {
        id: Option<u32>,
        message: String,
    },
    /// Pushed without a request. A connection receives the events of the sessions
    /// it opened, until it closes them or disconnects, and the events that belong to
    /// the process once it asked for `Request::Accounts`. Opening a repository that
    /// is already open subscribes to the existing session.
    Event {
        event: Event,
    },
}

/// Assigned by the backend in [`Response::Session`].
pub type SessionId = u32;
