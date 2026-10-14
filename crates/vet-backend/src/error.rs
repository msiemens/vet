//! Why a request failed, as the backend knows it: the errors of the layers below, kept as they
//! are, and what the protocol tells the shell of them (an [`ErrorKind`] and a message).

use std::fmt;

use vet_app::platform::{AccountError, PlatformError};
use vet_app::ports::{GitError, StoreError};
use vet_app::pull_request::FetchError;
use vet_app::registry::NoSuchProject;
use vet_app::servers::ServersError;
use vet_app::session::SessionError;
use vet_protocol::{ErrorKind, Failure};

#[derive(Clone, Debug)]
pub enum Error {
    Session(SessionError),
    Platform(PlatformError),
    Fetch(FetchError),
    Servers(ServersError),
    Account(AccountError),
    Store(StoreError),
    Git(GitError),
    NoSuchProject(NoSuchProject),
    /// The request cannot apply to what it names, or its input is not acceptable. Fit to show.
    Invalid(String),
    /// Something the request needs cannot be reached now. Fit to show.
    Unavailable(String),
    /// A failure of the backend's own, below what the layers above name.
    Internal(String),
    /// `source`, with what the backend was doing when it happened.
    Context {
        doing: String,
        source: Box<Error>,
    },
}

impl Error {
    pub fn context(self, doing: impl Into<String>) -> Self {
        Self::Context {
            doing: doing.into(),
            source: Box::new(self),
        }
    }

    /// A request about the pull request on view, while another kind of changeset is on view.
    pub fn not_a_pull_request() -> Self {
        Self::Invalid("the changeset on view is not a pull request".into())
    }

    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Session(e) => match e {
                SessionError::Stale { .. } => ErrorKind::Outdated,
                SessionError::NotFound(_) => ErrorKind::NotFound,
                SessionError::NotText(_) | SessionError::Base(_) | SessionError::Invalid(_) => ErrorKind::Invalid,
                SessionError::Git(_) | SessionError::Store(_) => ErrorKind::Internal,
                SessionError::Platform(e) => platform_kind(e),
            },
            Self::Platform(e) => platform_kind(e),
            Self::Fetch(e) => match e {
                FetchError::Platform(e) => platform_kind(e),
                FetchError::NoRemote(_) => ErrorKind::NotFound,
                FetchError::Git(_) => ErrorKind::Internal,
            },
            Self::Servers(e) => match e {
                ServersError::UnknownLanguage(_) => ErrorKind::NotFound,
                ServersError::Store(_) => ErrorKind::Internal,
                ServersError::Install(_) => ErrorKind::Unavailable,
            },
            Self::Account(e) => match e {
                AccountError::InvalidHost(_) | AccountError::EmptyToken => ErrorKind::Invalid,
                AccountError::Rejected(_) => ErrorKind::Platform,
                AccountError::Unreachable(_) => ErrorKind::Unavailable,
            },
            Self::Store(_) | Self::Git(_) | Self::Internal(_) => ErrorKind::Internal,
            Self::NoSuchProject(_) => ErrorKind::NotFound,
            Self::Invalid(_) => ErrorKind::Invalid,
            Self::Unavailable(_) => ErrorKind::Unavailable,
            Self::Context { source, .. } => source.kind(),
        }
    }
}

fn platform_kind(error: &PlatformError) -> ErrorKind {
    match error {
        PlatformError::NotFound(_) => ErrorKind::NotFound,
        PlatformError::Unreachable(_) => ErrorKind::Unavailable,
        PlatformError::SignedOut(_)
        | PlatformError::CliUnusable { .. }
        | PlatformError::Rejected(_)
        | PlatformError::Forbidden(_)
        | PlatformError::RateLimited { .. }
        | PlatformError::Unexpected(_) => ErrorKind::Platform,
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(e) => e.fmt(f),
            Self::Platform(e) => e.fmt(f),
            Self::Fetch(e) => e.fmt(f),
            Self::Servers(e) => e.fmt(f),
            Self::Account(e) => e.fmt(f),
            Self::Store(e) => f.write_str(&e.0),
            Self::Git(e) => e.fmt(f),
            Self::NoSuchProject(e) => e.fmt(f),
            Self::Invalid(message) | Self::Unavailable(message) | Self::Internal(message) => f.write_str(message),
            Self::Context { doing, source } => write!(f, "{doing}: {source}"),
        }
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Failure::new(error.kind(), error.to_string())
    }
}

macro_rules! from {
    ($($variant:ident($source:ty)),* $(,)?) => {$(
        impl From<$source> for Error {
            fn from(e: $source) -> Self {
                Self::$variant(e)
            }
        }
    )*};
}

from! {
    Session(SessionError),
    Platform(PlatformError),
    Fetch(FetchError),
    Servers(ServersError),
    Account(AccountError),
    Store(StoreError),
    Git(GitError),
    NoSuchProject(NoSuchProject),
}

pub type Result<T> = std::result::Result<T, Error>;
