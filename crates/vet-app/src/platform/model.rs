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
    /// Removing the first comment of a thread removes the thread. Otherwise the thread stays with
    /// its other comments, and goes once none is left.
    pub first_comment_ends_thread: bool,
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
            first_comment_ends_thread: true,
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
            // A removed comment is marked deleted; the thread is deleted with its last one.
            first_comment_ends_thread: false,
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
    /// A run the platform holds back until a maintainer approves it (a fork's workflow run).
    NeedsApproval,
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

/// What the reviewers of a pull request decided together, as the platform rolls their reviews up.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ReviewDecision {
    #[default]
    Undecided,
    Approved,
    ChangesRequested,
}

/// How a reviewer holds a pull request back from approval.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChangesAsked {
    /// GitHub's review that requests changes.
    Requested,
    /// Azure DevOps' vote "Rejected".
    Rejected,
    /// Azure DevOps' vote "Waiting for author".
    WaitingForAuthor,
}

/// A reviewer who holds a pull request back from approval, and how.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangesRequest {
    /// As the platform shows the reviewer to people.
    pub reviewer: String,
    pub how: ChangesAsked,
}

/// Where a pull request stands, as the queue's Status column says it. The first that applies of
/// these, in this order, is the pull request's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QueueStatus {
    Draft,
    /// The platform says it cannot be merged as it is: a blocking policy or check did not pass.
    Blocked,
    ChangesRequested,
    WaitingForYou,
    Approved,
}

/// The lines a pull request adds and removes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ChangedLines {
    pub added: u32,
    pub removed: u32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    /// The author's login: what the queue compares with the viewer's.
    pub author: String,
    /// What the author is called when shown: the display name, else the login.
    pub author_name: String,
    /// The address of the author's picture, when the platform gives one that a browser can load.
    pub author_avatar: Option<String>,
    pub source_branch: String,
    pub target_branch: String,
    /// The repository the source branch is in, as the platform names it, when that is not the
    /// repository the pull request is in.
    pub fork: Option<String>,
    /// The commit the source branch points to.
    pub head: String,
    /// Absent when the platform's list does not say.
    pub lines: Option<ChangedLines>,
    /// Comments on the pull request and on its code.
    pub comments: u32,
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
    /// What the reviewers decided together.
    pub decision: ReviewDecision,
    /// Who holds it back from approval, in the platform's order: what a decision to ask for
    /// changes is made of.
    pub changes_requested: Vec<ChangesRequest>,
    /// The platform holds the merge back: a blocking policy or branch protection is not met.
    pub blocked: bool,
    /// Unix seconds.
    pub updated_at: i64,
}

/// What a platform lists of a repository's open pull requests for a query.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PullRequestListing {
    /// Most recently updated first.
    pub pull_requests: Vec<PullRequest>,
    /// The platform stopped at the most it lists: more may be open than are here.
    pub truncated: bool,
}

/// How the platform last named a pull request: what a view of it is headed with. It is stored with
/// the changeset, so a review of it is headed before the platform is asked again.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PullRequestHead {
    pub title: String,
    pub draft: bool,
    /// The source branch is in a fork's repository (§7.5).
    pub fork: bool,
    /// The branch it is merged into.
    pub target_branch: String,
}

impl PullRequestHead {
    pub fn of(pull_request: &PullRequest) -> Self {
        Self {
            title: pull_request.title.clone(),
            draft: pull_request.draft,
            fork: pull_request.fork.is_some(),
            target_branch: pull_request.target_branch.clone(),
        }
    }
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
    /// There is no credential for the host because the `gh` that lists it cannot deliver its
    /// token; `reason` is `gh`'s own.
    CliUnusable {
        host: String,
        reason: String,
    },
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
            Self::CliUnusable { host, reason } => {
                write!(f, "the gh sign-in for {host} is unusable, as reported by gh: {reason}")
            }
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
