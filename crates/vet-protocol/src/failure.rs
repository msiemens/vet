//! Why a request failed, as the shell is told.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What a request failed for. The shell decides what to do about a failure from this and shows
/// the message; it never reads the message to tell kinds apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ErrorKind {
    /// The request was made against a revision the changeset has moved on from; asking again with
    /// the new state is what helps.
    Outdated,
    /// What the request names is not there (any more): a file, hunk, thread, symbol, branch,
    /// project or language.
    NotFound,
    /// The request cannot apply to what it names, or its input is not acceptable.
    Invalid,
    /// The hosting platform refused or could not do what was asked: no or rejected credentials,
    /// missing permission, a rate limit, an answer Vet cannot use.
    Platform,
    /// Something the request needs cannot be reached now: the platform's network, a language
    /// server that does not start, a project that is closing.
    Unavailable,
    /// A failure of Vet's own: git, the store, or a bug.
    Internal,
}

/// A failed request: its kind and the message that is fit to show.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Failure {
    pub kind: ErrorKind,
    pub message: String,
}

impl Failure {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
