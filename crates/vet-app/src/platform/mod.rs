//! Hosting platforms: which platform a host is, the credentials used against it, and
//! the pull requests it lists.

mod accounts;
mod cli;
mod credentials;
mod host;
mod model;
mod port;
mod pushes;
mod queue;
mod remote;

pub use accounts::{Account, AccountError, AccountState, Accounts};
pub use credentials::{Credential, CredentialProvider, CredentialSource, CredentialValidator, Token, ValidationError};
pub use host::{PlatformKind, normalize_host};
pub use model::{
    BlockingSource, ChangedLines, ChangesetThreads, CiState, ForcePush, PlatformError, PlatformTraits, Posting,
    PullRequest, PullRequestDetail, PullRequestQuery, Push, QueueTab, ReviewRequest, ReviewVerdict, Verdict, Viewer,
    ViewerReview,
};
pub use port::{Platform, list_rows};
pub use pushes::pushes_from_force_pushes;
#[cfg(test)]
pub(crate) use queue::tests::{pr, viewer};
pub use queue::{PullRequestRow, ReviewState, is_waiting_for_me, matches, review_state, rows};
pub use remote::{RemoteKey, parse_remote};
