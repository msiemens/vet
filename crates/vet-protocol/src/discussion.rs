//! A pull request's discussion beside Vet's threads: who it is by and for, what blocks it, its
//! conversation, how the platform is doing, and what was posted.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{CiState, CommentState, PlatformKind};

/// Which of the two texts of a comment that was edited in two places the viewer keeps.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EditChoice {
    /// The viewer's, which is then sent to the platform.
    Vet,
    /// The platform's.
    Remote,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ReviewerState {
    /// Asked, and has not given a verdict.
    Waiting,
    Approved,
    ChangesRequested,
    Commented,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Reviewer {
    /// A login, or `organization/team`.
    pub login: String,
    /// What the reviewer is called when shown: the display name, else the login.
    pub name: String,
    pub state: ReviewerState,
}

/// What holds the merge back, as far as the platform's rules and the data at hand say.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Blocker {
    /// Review threads that are not resolved.
    Threads { count: u32 },
    /// Checks that failed or have not finished.
    Checks,
    /// A reviewer asked for changes.
    ChangesRequested,
}

/// One of the ways to close a review. The adapters spell it as their platform does on the wire;
/// the UI words it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Verdict {
    Comment,
    Approve,
    RequestChanges,
    ApproveWithSuggestions,
    WaitForAuthor,
    Reject,
    /// A verdict Vet does not know the spelling of; `value` is what Vet saw spelled.
    Other {
        value: String,
    },
}

/// The platform's properties the UI branches on (SPEC §11), never its name.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlatformTraits {
    /// A flat feed of comments and reviews beside the threads.
    pub conversation_feed: bool,
    /// Drafts are submitted together with a verdict, rather than one at a time.
    pub batched_review: bool,
    /// In the platform's order.
    pub verdicts: Vec<Verdict>,
    pub verdict_separate: bool,
    pub pipelines: PipelineWords,
}

/// What the platform calls the parts of a run, and how its actions differ.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PipelineWords {
    /// What a run is made of and what is re-run: `job`, `stage`.
    pub group: String,
    /// What a group is made of: `step`, `task`.
    pub step: String,
    pub rerun_failed: String,
    pub rerun_all: String,
    pub rerun_group: String,
    pub cancel: String,
    /// Running everything again also stops a build that runs, so it asks first.
    pub rerun_all_replaces_running: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConversationEntry {
    /// The platform's id: what `EditConversationComment` and `DeleteConversationComment` take.
    pub id: String,
    pub author: String,
    pub avatar: Option<String>,
    /// Unix seconds.
    pub at: u32,
    pub kind: ConversationKind,
    pub body: String,
}

/// What an entry of the conversation is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ConversationKind {
    /// A comment on the pull request; `mine` when the viewer wrote it, who can edit and delete it.
    Comment { mine: bool },
    /// A review that was given, with what the reviewer concluded (none for one that was withdrawn);
    /// `mine` when the viewer gave it.
    Review { verdict: Option<ReviewVerdict>, mine: bool },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ReviewVerdict {
    Approved,
    ChangesRequested,
    Commented,
}

/// How the backend gets on with a hosting platform.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum PlatformConnection {
    Ok,
    /// There is no credential for the host.
    SignedOut,
    /// The platform no longer accepts the credential.
    TokenExpired {
        message: String,
    },
    /// The platform could not be asked or would not do it; fit to show.
    Failed {
        message: String,
        /// Unix seconds when a rate limit ends, if that is what failed and the platform says when.
        retry_at: Option<u32>,
    },
}

/// A hosting platform and how the backend gets on with it, as the footer shows.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlatformStatus {
    pub host: String,
    pub kind: PlatformKind,
    pub connection: PlatformConnection,
}

/// The vote of the viewer's on a pull request that waits to be sent, on its way to the platform
/// like a comment of theirs.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PendingVote {
    /// One of `PlatformTraits::verdicts`.
    pub verdict: Verdict,
    /// Never `posted`: a vote the platform has is no longer pending.
    pub state: CommentState,
}

/// What the Comments tool window shows of the pull request on view, above its threads.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestInfo {
    pub number: u32,
    pub title: String,
    pub url: String,
    /// The pull request's text as its author wrote it (markdown); empty when it has none.
    pub description: String,
    pub draft: bool,
    pub state: PullRequestState,
    /// The author's login.
    pub author: String,
    /// What the author is called when shown: the display name, else the login.
    pub author_name: String,
    pub author_avatar: Option<String>,
    pub source_branch: String,
    pub target_branch: String,
    pub reviewers: Vec<Reviewer>,
    /// The platform's verdict the viewer has voted, on a platform that keeps the vote apart from
    /// posting (`traits.verdict_separate`); absent without a vote.
    pub viewer_verdict: Option<Verdict>,
    /// The vote staged and not sent yet (`StageVote`), on such a platform; sent by `PostDrafts`.
    pub pending_vote: Option<PendingVote>,
    pub labels: Vec<String>,
    /// The checks of the head commit; absent when it has none.
    pub ci: Option<CiState>,
    pub blocking: Vec<Blocker>,
    pub traits: PlatformTraits,
    /// Oldest first; empty unless `traits.conversation_feed`.
    pub conversation: Vec<ConversationEntry>,
    /// Unix seconds of the last sync of the threads; absent before the first.
    pub synced_at: Option<u32>,
    pub platform: PlatformStatus,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NotificationKind {
    /// Something that happened to the viewer's work: a thread deleted on the platform.
    Info,
    /// The platform cannot be synced with; the viewer can sign in again.
    SyncStopped,
}

/// A message for a card in the window's corner, and for the list the footer's bell opens.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Notification {
    pub kind: NotificationKind,
    /// Fit to show.
    pub text: String,
    /// What "Copy text" copies, when there is something worth keeping from it: a dropped draft.
    pub copy_text: Option<String>,
    /// The host a `SyncStopped` is about.
    pub host: Option<String>,
}

/// What a submitted review became.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewSubmitted {
    /// How many drafts the platform has now.
    pub posted: u32,
    /// Drafts that wait: their code is gone, or their thread has an edit conflict to settle.
    pub skipped: u32,
    /// The pending vote was sent and the platform took it.
    pub vote_cast: bool,
    /// Why posting stopped short, fit to show: the drafts it was about failed, or are on their way
    /// when the platform could not be reached.
    pub problem: Option<String>,
}
