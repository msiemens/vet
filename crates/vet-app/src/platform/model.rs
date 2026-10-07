//! What hosting platforms say about pull requests, in terms that hold for every
//! platform, and how each platform is described to the rest of Vet (SPEC §11).

use super::host::PlatformKind;

/// What differs between platforms, as data. Vet core and the UI branch on these and
/// never on the platform's name.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlatformTraits {
    pub changeset_threads: ChangesetThreads,
    /// A flat feed of comments and events beside the threads.
    pub conversation_feed: bool,
    pub posting: Posting,
    /// The platform's own choices, in its order.
    pub verdicts: Vec<Verdict>,
    /// The vote is set independently of posting.
    pub verdict_separate: bool,
    pub blocking: Vec<BlockingSource>,
    /// Comments are recorded on a patch set.
    pub revision_scoped: bool,
}

/// What a comment on the whole pull request is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChangesetThreads {
    Resolvable,
    Flat,
    Either,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Posting {
    /// One comment at a time.
    PerComment,
    /// Drafts are submitted together, with a verdict.
    BatchedReview,
}

/// A way to close a review. `id` is for the code, `label` is fit to show.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Verdict {
    pub id: &'static str,
    pub label: &'static str,
}

/// What can hold a merge back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockingSource {
    UnresolvedThreads,
    OpenTasks,
    Labels,
}

impl PlatformTraits {
    pub fn of(kind: PlatformKind) -> Self {
        match kind {
            PlatformKind::GitHub => Self::github(),
            PlatformKind::AzureDevOps => Self::azure_devops(),
        }
    }

    pub fn github() -> Self {
        Self {
            changeset_threads: ChangesetThreads::Flat,
            conversation_feed: true,
            posting: Posting::BatchedReview,
            verdicts: vec![
                Verdict {
                    id: "COMMENT",
                    label: "Comment",
                },
                Verdict {
                    id: "APPROVE",
                    label: "Approve",
                },
                Verdict {
                    id: "REQUEST_CHANGES",
                    label: "Request changes",
                },
            ],
            verdict_separate: false,
            blocking: vec![BlockingSource::UnresolvedThreads],
            revision_scoped: false,
        }
    }

    pub fn azure_devops() -> Self {
        Self {
            changeset_threads: ChangesetThreads::Resolvable,
            conversation_feed: false,
            posting: Posting::PerComment,
            verdicts: vec![
                Verdict {
                    id: "approved",
                    label: "Approve",
                },
                Verdict {
                    id: "approved-with-suggestions",
                    label: "Approve with suggestions",
                },
                Verdict {
                    id: "wait-for-author",
                    label: "Wait for author",
                },
                Verdict {
                    id: "rejected",
                    label: "Reject",
                },
            ],
            verdict_separate: true,
            blocking: vec![BlockingSource::UnresolvedThreads],
            revision_scoped: false,
        }
    }
}

/// The signed-in user of a platform, as far as review requests are concerned.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Viewer {
    pub login: String,
    /// The teams the user is in, as [`ReviewRequest::Team`] names them. Empty when the
    /// platform does not say (a token without the scope to list teams).
    pub teams: Vec<String>,
}

/// The rolled-up state of a commit's checks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CiState {
    Pending,
    Success,
    Failure,
    Neutral,
    Skipped,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ReviewRequest {
    User(String),
    /// `organization/team` on GitHub.
    Team(String),
}

/// What a reviewer concluded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReviewVerdict {
    Approved,
    ChangesRequested,
    Commented,
    /// Withdrawn, for instance because of a push; the reviewer is asked again.
    Dismissed,
}

/// The latest review the viewer gave a pull request.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ViewerReview {
    pub verdict: ReviewVerdict,
    /// The head commit the review was made on, if the platform says.
    pub commit: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    pub source_branch: String,
    pub target_branch: String,
    /// The commit the source branch points to.
    pub head: String,
    pub draft: bool,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    /// The rolled-up checks of `head`; none if it has none.
    pub ci: Option<CiState>,
    /// Counted as far as the platform listed the threads.
    pub unresolved_threads: u32,
    /// Those whose review is asked for and not given yet.
    pub review_requests: Vec<ReviewRequest>,
    pub viewer_review: Option<ViewerReview>,
    /// Unix seconds.
    pub updated_at: i64,
}

/// One state of the source branch.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Push {
    pub head: String,
    /// Unix seconds, if the platform says.
    pub at: Option<i64>,
    /// The push replaced earlier commits rather than adding to them.
    pub forced: bool,
}

/// A force-push the platform recorded on the pull request.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ForcePush {
    pub at: i64,
    pub before: String,
    pub after: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PullRequestDetail {
    pub pull_request: PullRequest,
    /// The commit the changes are measured from: the tip of the target branch on GitHub,
    /// the merge base on Azure DevOps.
    pub base: String,
    /// The source branch is in another repository.
    pub from_fork: bool,
    /// The states of the source branch the platform lets us know of, oldest first, the
    /// current one last.
    pub pushes: Vec<Push>,
}

/// The lists the queue offers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QueueTab {
    /// Review requested from the viewer or their team, and not given since the last push.
    WaitingForMe,
    Mine,
    Assigned,
    AllOpen,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PullRequestQuery {
    pub tab: QueueTab,
    pub author: Option<String>,
    pub label: Option<String>,
    pub draft: Option<bool>,
    pub ci: Option<CiState>,
    pub limit: Option<u32>,
}

impl PullRequestQuery {
    pub fn new(tab: QueueTab) -> Self {
        Self {
            tab,
            author: None,
            label: None,
            draft: None,
            ci: None,
            limit: None,
        }
    }
}

/// Why a platform could not answer. Every message is fit to show.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PlatformError {
    /// There is no credential for the host.
    SignedOut(String),
    /// The platform does not accept the credential.
    Rejected(String),
    /// The credential is accepted but may not do this.
    Forbidden(String),
    NotFound(String),
    /// The platform asks to wait; `retry_at` is unix seconds, if it says when.
    RateLimited {
        message: String,
        retry_at: Option<i64>,
    },
    Unreachable(String),
    /// The platform answered with something Vet cannot use.
    Unexpected(String),
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SignedOut(host) => write!(f, "not signed in to {host}"),
            Self::Rejected(why)
            | Self::Forbidden(why)
            | Self::NotFound(why)
            | Self::Unreachable(why)
            | Self::Unexpected(why) => f.write_str(why),
            Self::RateLimited { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for PlatformError {}
