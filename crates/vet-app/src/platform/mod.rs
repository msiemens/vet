//! Hosting platforms: which platform a host is, the credentials used against it, and
//! the pull requests it lists.

mod accounts;
mod checks;
mod cli;
mod credentials;
mod discussion;
#[cfg(test)]
mod fake;
mod host;
mod model;
mod port;
mod pushes;
mod queue;
mod remote;
mod sync;

pub use accounts::{Account, AccountError, AccountState, Accounts};
pub use checks::{Check, CheckLog, CommitChecks, LogFailure};
pub use credentials::{Credential, CredentialProvider, CredentialSource, CredentialValidator, Token, ValidationError};
pub use discussion::{
    CommentKind, CommentRef, ConversationItem, DiffSide, Discussion, NewConversationComment, NewReply, NewThread,
    PlatformComment, PlatformThreadData, PostedComment, PostedThread, PullRequestState, Reviewer, ReviewerState,
    Submission, Submitted,
};
#[cfg(test)]
pub(crate) use fake::FakeHosting;
pub use host::{PlatformKind, normalize_host};
pub use model::{
    BlockingSource, ChangedLines, ChangesetThreads, CiState, ForcePush, PlatformError, PlatformTraits, Posting,
    PullRequest, PullRequestDetail, PullRequestHead, PullRequestListing, PullRequestQuery, Push, QueueStatus, QueueTab,
    ReviewDecision, ReviewRequest, ReviewVerdict, Verdict, Viewer, ViewerReview,
};
pub use port::{ListedRows, Platform, commits_checks, list_rows};
pub use pushes::pushes_from_force_pushes;
#[cfg(test)]
pub(crate) use queue::tests::{pr, viewer};
pub use queue::{PullRequestRow, ReviewState, is_waiting_for_me, matches, queue_status, review_state, rows};
pub use remote::{RemoteKey, parse_remote};
pub use sync::{MessageSync, RemovedThread, StatusSync, SyncPlan, ThreadSync, reconcile};
