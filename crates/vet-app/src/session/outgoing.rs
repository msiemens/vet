//! What the viewer's own comments and staged vote wait to send to the platform (A17): the backend
//! decides what is outgoing; the shell only words it.

use vet_core::review::{Message, Outgoing as State, PendingVote, Thread};

use super::LiveSession;
use crate::ports::{GitRepo, LiveStore};

/// What the viewer's own comments and staged vote wait to send to the platform.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Outgoing {
    /// Everything outgoing: the comments the platform does not have, and the staged vote.
    pub count: u32,
    /// How many the platform refused.
    pub failed: u32,
    /// Some are on their way to the platform.
    pub posting: bool,
    /// The drafts and failed comments a post would send.
    pub comments: u32,
    /// A staged vote waits to be sent.
    pub vote: bool,
}

/// Whether the viewer's message waits to be sent: the platform does not have it.
fn waits(m: &Message) -> bool {
    m.remote.is_none()
}

/// What the comments of `threads` and the staged `vote` wait to send.
pub fn outgoing(threads: &[Thread], vote: Option<&PendingVote>) -> Outgoing {
    let mut out = Outgoing::default();
    for m in threads
        .iter()
        .filter(|t| t.platform.is_some())
        .flat_map(|t| t.messages.iter())
        .filter(|m| waits(m))
    {
        out.count += 1;
        match m.outgoing {
            State::Draft => out.comments += 1,
            State::Failed { .. } => {
                out.failed += 1;
                out.comments += 1;
            }
            State::Posting { .. } => out.posting = true,
        }
    }
    if let Some(vote) = vote {
        out.count += 1;
        match vote.outgoing {
            State::Posting { .. } => out.posting = true,
            State::Failed { .. } => out.failed += 1,
            State::Draft => {}
        }
        out.vote = !matches!(vote.outgoing, State::Posting { .. });
    }
    out
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// What the viewer's own comments and staged vote of the pull request on view wait to send.
    pub fn outgoing(&self, vote: Option<&PendingVote>) -> Outgoing {
        outgoing(&self.published.read().threads, vote)
    }
}

#[cfg(test)]
mod tests {
    use vet_core::review::{
        CommentTarget, MessageId, Outgoing as State, PendingVote, PlatformThread, Thread, ThreadId, ThreadStatus,
        Verdict,
    };

    use super::*;

    fn message(id: u64, state: State, posted: bool) -> Message {
        Message {
            id: MessageId(id),
            author: "you".into(),
            body: String::new(),
            at: 1,
            mine: true,
            avatar: None,
            edited_at: None,
            remote: posted.then(|| vet_core::review::RemoteComment {
                id: format!("r{id}"),
                synced: String::new(),
                conflict: None,
            }),
            outgoing: state,
        }
    }

    fn thread(platform: bool, messages: Vec<Message>) -> Thread {
        Thread {
            id: ThreadId(1),
            target: CommentTarget::Changeset,
            status: ThreadStatus::Open,
            messages,
            platform: platform.then(|| PlatformThread::draft(true)),
        }
    }

    #[test]
    fn only_the_unposted_comments_of_a_platforms_threads_are_outgoing() {
        let threads = [
            thread(
                true,
                vec![
                    message(1, State::Draft, false),
                    message(2, State::Failed { error: "locked".into() }, false),
                    message(3, State::Draft, true),
                ],
            ),
            // Without a platform nothing is posted, so nothing is outgoing.
            thread(false, vec![message(4, State::Draft, false)]),
        ];
        let out = outgoing(&threads, None);
        assert_eq!((out.count, out.failed, out.posting, out.comments), (2, 1, false, 2));
        assert!(!out.vote);
    }

    #[test]
    fn a_staged_vote_counts_like_a_comment_and_waits_unless_it_is_on_its_way() {
        let threads = [thread(true, vec![message(1, State::Draft, false)])];
        let vote = |outgoing| PendingVote {
            verdict: Verdict::Approve,
            outgoing,
        };
        let draft = outgoing(&threads, Some(&vote(State::Draft)));
        assert_eq!((draft.count, draft.comments, draft.vote), (2, 1, true));
        let posting = outgoing(&threads, Some(&vote(State::Posting { since: 1 })));
        assert_eq!((posting.count, posting.posting, posting.vote), (2, true, false));
        let failed = outgoing(&threads, Some(&vote(State::Failed { error: "no".into() })));
        assert_eq!((failed.count, failed.failed, failed.vote), (2, 1, true));
    }
}
