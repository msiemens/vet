//! The handshake and the frames around requests, responses and events.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Event, Request, Response};

pub const PROTOCOL_VERSION: u32 = 2;

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
    Hello {
        token: String,
        protocol_version: u32,
    },
    Request {
        id: u32,
        request: Request,
    },
    /// Gives up the request `id`: its handler is dropped and answered with
    /// [`ServerMessage::Cancelled`]. Meaningless for a request that has been answered.
    Cancel {
        id: u32,
    },
    /// An entry for the backend's log, which the UI has no file for: an uncaught error, an
    /// unhandled rejection or a failed request. Answered by nothing. The backend logs it under the
    /// target `ui`.
    Log {
        level: LogLevel,
        message: String,
        /// Where it came from (`window.onerror`, `request hover`), a short label.
        context: String,
    },
}

/// How serious an entry of the UI's log is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
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
    /// The request `id` was cancelled and will get no `Response` or `Error`.
    Cancelled {
        id: u32,
    },
    /// Pushed without a request. A connection receives the events of the projects
    /// it opened, until it closes them or disconnects, and the events that belong to
    /// the process once it asked for `Request::Accounts`. Opening a repository that
    /// is already open subscribes to the existing project.
    Event {
        event: Event,
    },
}

/// Assigned by the backend in [`Response::Project`]: the handle to an open repository.
pub type ProjectId = u32;
