//! Threads of a pull request exchanged with its platform (SPEC §7.4): pulled in when
//! the pull request opens, on fetch and after a review is submitted; edits to posted comments
//! pushed at once; drafts posted together as one review. The platform is asked with the session's
//! lock released, so what the viewer does meanwhile is never held up by the network.

mod edits;
mod outgoing;
mod posting;
mod sync;
mod unread;
mod votes;

use vet_core::num::sat_u32;
use vet_core::review::{CommentTarget, Message, MessageId, Thread, ThreadId, ThreadStatus};

pub use self::outgoing::Outgoing;
pub use self::unread::{Unread, is_unread};
use super::state::{State, View};
use super::threads::{ThreadView, put_message, thread_by_id};
use super::{LiveSession, Result, SessionError};
use crate::changeset::Source;
use crate::platform::{CiState, CommentKind, CommentRef, Discussion, PlatformError, RemoteKey, ReviewerState};
use crate::ports::{GitRepo, LiveStore};

/// Which of an edit's two texts the viewer keeps.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Keep {
    /// The viewer's, which is then sent to the platform.
    Vet,
    /// The platform's.
    Remote,
}

/// Something the viewer should hear about that a sync did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SyncNotice {
    /// The platform no longer has a thread, so Vet dropped it. `dropped` are the texts of the
    /// viewer's drafts that went with it.
    ThreadDeleted { place: String, dropped: Vec<String> },
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SyncReport {
    pub notices: Vec<SyncNotice>,
    /// Threads were added, changed or removed.
    pub changed: bool,
    /// What the queue shows of the pull request changed with what the platform said of it.
    pub listing_changed: bool,
    /// The platform refused or could not take something Vet had to send; the rest was synced.
    pub problem: Option<PlatformError>,
    /// The changeset on view was another by the time the platform answered, so nothing was synced.
    pub source_changed: bool,
}

impl SyncReport {
    fn source_changed() -> Self {
        Self {
            source_changed: true,
            ..Self::default()
        }
    }
}

/// What the platform said of the pull request on view when it was last synced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DiscussionView {
    pub number: u64,
    /// Unix seconds.
    pub synced_at: i64,
    pub discussion: Discussion,
    /// Threads the platform has that are open and can be resolved.
    pub unresolved_threads: u32,
    /// What stands between the pull request and its merge, as the last sync found it (§7.2).
    pub blocking: Vec<Blocker>,
}

/// What a view of a pull request names as standing in the merge's way.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Blocker {
    /// `u32` threads that are open and can be resolved.
    Threads(u32),
    /// Checks that have failed or are still running.
    Checks,
    /// A reviewer has asked for changes.
    ChangesRequested,
}

/// What stands in the merge's way: threads that can be resolved, checks that have failed or
/// are not done, and a reviewer who asked for changes. No `unresolved_threads` among them, so
/// what a run synced is what the viewer shows.
fn blocking(discussion: &Discussion, unresolved_threads: u32) -> Vec<Blocker> {
    let mut blocking = Vec::new();
    if unresolved_threads > 0 {
        blocking.push(Blocker::Threads(unresolved_threads));
    }
    if matches!(discussion.ci, Some(CiState::Failure | CiState::Pending)) {
        blocking.push(Blocker::Checks);
    }
    if discussion
        .reviewers
        .iter()
        .any(|r| r.state == ReviewerState::ChangesRequested)
    {
        blocking.push(Blocker::ChangesRequested);
    }
    blocking
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SubmitReport {
    /// How many drafts were posted.
    pub posted: u32,
    /// How many drafts the platform refused, which are marked failed with its reason.
    pub failed: u32,
    /// Drafts that were not posted, because the code they were on is gone or because the thread
    /// waits for an edit conflict to be settled.
    pub skipped: u32,
    /// Why posting stopped short: the platform refused the review, a comment on the pull request as
    /// a whole, or on a platform that takes comments one at a time, the next draft; or it could not
    /// be reached.
    pub problem: Option<PlatformError>,
    /// The texts of the drafts that were not posted, which stay drafts.
    pub not_posted: Vec<String>,
    /// What came of the pending vote.
    pub vote: VoteSent,
}

/// What a post did with the viewer's pending vote.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum VoteSent {
    /// There was none to send, or the comments before it were not all posted.
    #[default]
    NotSent,
    Cast,
    /// The platform refused it or could not be reached; `SubmitReport::problem` says why.
    Failed,
}

fn pull_request(st: &State) -> Result<(RemoteKey, u64)> {
    match &st.source {
        Source::PullRequest { remote, number } => Ok((remote.clone(), *number)),
        _ => Err(SessionError::Invalid(
            "the changeset on view is not a pull request".into(),
        )),
    }
}

/// How the platform finds a comment of `thread`.
fn comment_ref(thread: &Thread, message: &Message) -> Option<CommentRef> {
    Some(CommentRef {
        id: message.remote.as_ref()?.id.clone(),
        kind: if thread.target == CommentTarget::Changeset {
            CommentKind::Conversation
        } else {
            CommentKind::Review
        },
    })
}

fn find_message(st: &State, id: MessageId) -> Result<(&Thread, &Message)> {
    st.threads
        .iter()
        .find_map(|t| t.messages.iter().find(|m| m.id == id).map(|m| (t, m)))
        .ok_or_else(|| SessionError::NotFound(format!("message {}", id.0)))
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// What the platform last said of the pull request on view besides its threads; `None` for
    /// another changeset, and before the pull request was first synced. What was not read from the
    /// platform in this run is read from the store, so a revisit shows it at once.
    pub async fn discussion(&self) -> Option<DiscussionView> {
        let source = self.published.read().source.clone();
        let Source::PullRequest { remote, number } = &source else {
            return None;
        };
        let (synced_at, discussion) = match self.known.discussion(remote, *number) {
            Some(found) => found,
            None => {
                let stored = self
                    .acquisition
                    .store
                    .stored_discussion(remote.clone(), *number)
                    .await
                    .ok()
                    .flatten()?;
                (stored.synced_at, stored.discussion)
            }
        };
        let st = self.published.read();
        let unresolved_threads = sat_u32(
            st.threads
                .iter()
                .filter(|t| {
                    t.is_posted() && t.status == ThreadStatus::Open && t.platform.as_ref().is_some_and(|p| p.resolvable)
                })
                .count(),
        );
        let blocking = blocking(&discussion, unresolved_threads);
        Some(DiscussionView {
            number: *number,
            synced_at,
            discussion,
            unresolved_threads,
            blocking,
        })
    }

    async fn store_message(&self, thread: ThreadId, message: Message) -> Result<()> {
        let mut st = self.hold().await;
        put_message(&mut st, thread, &message).await
    }

    async fn view_of(&self, thread: ThreadId) -> Result<ThreadView> {
        let (st, _) = self.viewing(View::Changeset).await?;
        thread_by_id(&st, thread)
    }
}
