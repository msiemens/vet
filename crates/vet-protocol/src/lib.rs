//! Shell <-> backend protocol (SPEC §4.1): one protocol over one loopback
//! WebSocket in every topology, JSON text frames.
//!
//! Ids and counts are `u32` so they map to plain TS `number`s.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const PROTOCOL_VERSION: u32 = 0;

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

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ServerMessage {
    HelloAck { protocol_version: u32 },
    Response { id: u32, response: Response },
    /// `id` is `None` when the failure is not attributable to a request.
    Error { id: Option<u32>, message: String },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum Request {
    /// Round-trip probe, used to measure transport latency (the spike's hover budget).
    Ping { payload: String },
    /// Lists the changeset under review (spike: the mock's corpus).
    OpenChangeset,
    /// Both sides of one file plus its hunks and inline marks.
    GetFile { path: String },
    /// Hover at a target-side position; measures the transport for the spike.
    Hover { path: String, line: u32, col: u32 },
    /// Marks the given hunks of a file seen or unseen. Review state lives in the
    /// backend (I5); the reply carries the file's resulting state.
    SetSeen { path: String, hunks: Vec<u32>, seen: bool },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum Response {
    Pong { payload: String },
    Changeset { name: String, files: Vec<FileEntry> },
    File { path: String, base: String, target: String, hunks: Vec<Hunk>, marks: Vec<Mark> },
    Hover { text: String },
    Seen { path: String, seen: Vec<bool> },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileEntry {
    pub path: String,
    pub base_lines: u32,
    pub target_lines: u32,
    /// `(base_len, target_len)` of each hunk, in order: enough for a shell to
    /// size its views before it has the file's text.
    pub hunks: Vec<(u32, u32)>,
    /// One seen flag per hunk.
    pub seen: Vec<bool>,
}

/// A changed region. Line numbers are 0-based; a pure insertion has
/// `base_len == 0`, a pure deletion `target_len == 0`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Hunk {
    pub base_start: u32,
    pub base_len: u32,
    pub target_start: u32,
    pub target_len: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MarkKind {
    /// A changed symbol (derived, §7.3).
    Symbol,
    /// Reachability of the changed code (derived, §7.4).
    Reachability,
    /// An executor-relevant or whitespace note.
    Note,
}

/// An inline decoration on the target side. `from`/`to` are UTF-16 columns, as
/// CodeMirror counts them.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Mark {
    pub line: u32,
    pub from: u32,
    pub to: u32,
    pub kind: MarkKind,
}
