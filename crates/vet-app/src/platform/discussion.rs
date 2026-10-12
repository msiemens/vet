//! What people say on a pull request, in terms that hold for every platform: the review threads
//! with their comments, the flat conversation, who reviews it, and what Vet posts.

use super::model::{CiState, PlatformError, ReviewVerdict};

/// Which side of the diff a comment's lines are on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiffSide {
    /// The lines the pull request removes: the file as it is in the base.
    Left,
    /// The lines it keeps or adds: the file as it is in the head.
    Right,
}

/// One comment, as the platform has it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlatformComment {
    pub id: String,
    pub author: String,
    pub avatar: Option<String>,
    pub body: String,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds of the last edit; the creation time if it was never edited.
    pub edited_at: i64,
    /// The commit the comment was made on: what its lines are numbered in.
    pub commit: Option<String>,
    /// The viewer wrote it.
    pub mine: bool,
}

/// A review thread: comments on lines of a file, or on the file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlatformThreadData {
    pub id: String,
    pub path: String,
    pub side: DiffSide,
    /// 1-based, inclusive, in the file at the commit the first comment was made on; `None` for a
    /// comment on the whole file.
    pub lines: Option<(u32, u32)>,
    /// The code the comments are on changed since they were made.
    pub outdated: bool,
    pub resolved: bool,
    pub resolved_by: Option<String>,
    /// The first comment opens the thread.
    pub comments: Vec<PlatformComment>,
}

/// One entry of the flat conversation: a comment on the pull request itself, or a review that was
/// given.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConversationItem {
    pub id: String,
    pub author: String,
    pub avatar: Option<String>,
    /// Unix seconds.
    pub at: i64,
    pub kind: ConversationKind,
    pub body: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConversationKind {
    /// A comment on the pull request; one the viewer wrote they can edit and delete.
    Comment { mine: bool },
    /// A review that was given, with what the reviewer concluded; `mine` when the viewer gave it.
    Review { verdict: Option<ReviewVerdict>, mine: bool },
}

/// Whether the pull request still takes changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
}

/// Where a reviewer stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReviewerState {
    /// Asked, and has not given a verdict (or it was withdrawn).
    Waiting,
    Approved,
    ChangesRequested,
    Commented,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reviewer {
    /// The login, or `organization/team`.
    pub login: String,
    /// What the reviewer is called when shown: the display name, else the login.
    pub name: String,
    pub state: ReviewerState,
}

/// Everything the Comments tool window shows of a pull request besides Vet's own threads.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Discussion {
    pub title: String,
    pub url: String,
    pub draft: bool,
    pub source_branch: String,
    pub target_branch: String,
    pub state: PullRequestState,
    /// The author's login.
    pub author: String,
    /// What the author is called when shown: the display name, else the login.
    pub author_name: String,
    pub author_avatar: Option<String>,
    pub reviewers: Vec<Reviewer>,
    /// The id of the platform's verdict the viewer has voted, on a platform that keeps the vote
    /// apart from the review ([`super::PlatformTraits::verdict_separate`]).
    pub viewer_verdict: Option<String>,
    pub labels: Vec<String>,
    pub ci: Option<CiState>,
    /// Oldest first.
    pub conversation: Vec<ConversationItem>,
    pub threads: Vec<PlatformThreadData>,
    /// Every thread of the pull request is in `threads`. A thread missing from a partial listing
    /// is not gone.
    pub complete: bool,
}

/// Which kind of comment an id names; the platforms keep them apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommentKind {
    /// In a review thread, on code.
    Review,
    /// On the pull request as a whole.
    Conversation,
}

/// A comment to find again on the platform.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CommentRef {
    pub id: String,
    pub kind: CommentKind,
}

/// A new thread of a review. `key` names it in [`Submitted`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NewThread {
    pub key: u64,
    pub path: String,
    pub side: DiffSide,
    /// 1-based, inclusive, in the file at [`Submission::commit`]; `None` for a comment on the file.
    pub lines: Option<(u32, u32)>,
    pub body: String,
    /// What the viewer answered themselves in the thread before posting it.
    pub replies: Vec<NewReply>,
}

/// An answer in a thread. `thread` is the platform's id of the thread it is in, and empty in a
/// [`NewThread`], whose thread has none yet.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NewReply {
    pub key: u64,
    pub thread: String,
    pub body: String,
}

/// A comment on the pull request as a whole.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NewConversationComment {
    pub key: u64,
    pub body: String,
}

/// Everything Vet posts at once: one review with a verdict, or on a platform that takes comments
/// one at a time the drafts to post in turn.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Submission {
    /// The id of one of the platform's verdicts (`PlatformTraits::verdicts`); empty where posting
    /// has none ([`super::Posting::PerComment`]).
    pub verdict: String,
    pub body: Option<String>,
    /// The head commit the lines of `threads` are numbered in.
    pub commit: String,
    pub threads: Vec<NewThread>,
    pub replies: Vec<NewReply>,
    pub comments: Vec<NewConversationComment>,
}

impl Submission {
    /// Whether there is no draft to post, whatever the review's own text.
    pub fn is_empty(&self) -> bool {
        self.threads.is_empty() && self.replies.is_empty() && self.comments.is_empty()
    }
}

/// Where a posted draft is on the platform.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PostedThread {
    pub key: u64,
    pub thread: String,
    pub comment: PlatformComment,
    pub replies: Vec<PostedComment>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PostedComment {
    pub key: u64,
    pub comment: PlatformComment,
}

/// What a [`Submission`] became. On a platform that batches (GitHub) the review is either all there
/// or not at all, and the comments on the pull request as a whole follow it one by one. On one that
/// takes a comment at a time (Azure DevOps) everything is posted one by one, a comment on the pull
/// request as a whole being a thread of its own, among `threads`. Posting stops at the first draft
/// the platform refuses: the drafts before it are here, the reason is in `failed`, and the rest was
/// not tried.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Submitted {
    pub threads: Vec<PostedThread>,
    pub replies: Vec<PostedComment>,
    pub comments: Vec<PostedComment>,
    pub failed: Option<PlatformError>,
}
