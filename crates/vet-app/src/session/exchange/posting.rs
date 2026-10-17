use std::collections::HashSet;

use vet_core::num::sat_u32;
use vet_core::review::{
    AnchorState, CommentTarget, Message, MessageId, Outgoing, RemoteComment, Side, Strategy, Thread, ThreadId, Verdict,
};

use super::{SubmitReport, VoteSent, find_message, pull_request};
use crate::platform::{
    ConversationItem, ConversationKind, DiffSide, Discussion, NewConversationComment, NewReply, NewThread, Platform,
    PlatformComment, PlatformError, PostedComment, Posting, Submission, Submitted,
};
use crate::ports::{GitRepo, LiveStore, ReviewWrites};
use crate::session::hold::Held;
use crate::session::state::{State, View};
use crate::session::threads::{edit_thread, find_thread, put_message, put_platform, thread_view};
use crate::session::{LiveSession, Result, SessionError};

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// Posts the viewer's drafts as one review with `verdict` (one of the platform's), then
    /// records where they are on the platform. Drafts whose code is gone, and those in a thread
    /// with an edit conflict, wait. Fails, posting nothing of the review, if the platform refuses.
    pub async fn submit_review<P: Platform + Sync>(
        &self,
        platform: &P,
        verdict: Verdict,
        body: Option<String>,
    ) -> Result<SubmitReport> {
        let traits = platform.traits();
        if traits.posting != Posting::BatchedReview {
            return Err(SessionError::Invalid(
                "this platform takes comments one at a time: post them instead".into(),
            ));
        }
        if !traits.verdicts.contains(&verdict) {
            return Err(SessionError::Invalid(format!("the platform has no verdict {verdict}")));
        }
        let allow_empty = verdict != Verdict::Comment;
        let body = body.map(|b| b.trim().to_owned()).filter(|b| !b.is_empty());
        self.send_drafts(platform, verdict, body, allow_empty).await
    }

    /// Posts the viewer's drafts one after the other, on a platform that takes comments one at a
    /// time, and records where they are on the platform. Drafts whose code is gone (deleted lines
    /// included), and those in a thread with an edit conflict, wait. Stops at the first the
    /// platform refuses; what was posted before it stays posted. Then sends the pending vote,
    /// unless the comments stopped short.
    pub async fn post_drafts<P: Platform + Sync>(&self, platform: &P) -> Result<SubmitReport> {
        let traits = platform.traits();
        if traits.posting != Posting::PerComment {
            return Err(SessionError::Invalid(
                "this platform takes the drafts as a review: submit it instead".into(),
            ));
        }
        let vote_waits = traits.verdict_separate && self.vote_to_send().await?.is_some();
        let mut report = self.send_drafts(platform, Verdict::Comment, None, vote_waits).await?;
        if vote_waits && report.problem.is_none() {
            let (sent, problem) = self.send_vote(platform).await?;
            report.vote = sent;
            report.problem = problem;
        }
        Ok(report)
    }

    async fn send_drafts<P: Platform + Sync>(
        &self,
        platform: &P,
        verdict: Verdict,
        body: Option<String>,
        allow_empty: bool,
    ) -> Result<SubmitReport> {
        let _exchange = self.exchange.lock().await;
        pull_request(&self.published.read())?;
        let (remote, number, submission, skipped) = {
            let (mut st, _) = self.writing(View::Changeset).await?;
            let (remote, number) = pull_request(&st)?;
            let (submission, skipped) = drafts_of(&st, verdict, body.clone());
            if submission.is_empty() && body.is_none() && !allow_empty {
                return Err(SessionError::Invalid("there is nothing to submit".into()));
            }
            let since = (self.clock)();
            for key in keys_in_turn(&submission) {
                self.set_outgoing(&mut st, MessageId(key), Outgoing::Posting { since })
                    .await?;
            }
            (remote, number, submission, skipped)
        };
        // A platform that takes comments one at a time has nothing to be asked when there are none.
        let asked = if submission.is_empty() && platform.traits().posting == Posting::PerComment {
            Ok(Submitted::default())
        } else {
            platform.submit_review(&remote, number, &submission).await
        };
        let (posted, whole) = match asked {
            Ok(posted) => (posted, false),
            Err(e) => (
                Submitted {
                    failed: Some(e),
                    ..Submitted::default()
                },
                true,
            ),
        };
        let mut report = SubmitReport {
            posted: 0,
            failed: 0,
            skipped,
            problem: posted.failed.clone(),
            not_posted: not_posted(&submission, &posted),
            vote: VoteSent::NotSent,
        };
        let mut st = self.hold().await;
        report.posted = self.record_posted(&mut st, &posted).await?;
        report.failed = self.settle_unposted(&mut st, &submission, &posted, whole).await?;
        Ok(report)
    }

    /// Settles the drafts of `submission` that `posted` does not hold. Without a refusal they were
    /// not tried and are drafts again. A refusal fails the draft it was about: every draft when the
    /// platform refused all of them at once (`whole`), else the first that is not posted, the
    /// rest not having been tried. A platform that could not be reached may have taken what was
    /// asked of it, which stays posting until the next sync looks for it there. Answers how many
    /// failed.
    async fn settle_unposted(
        &self,
        st: &mut Held<'_, S::Writes>,
        submission: &Submission,
        posted: &Submitted,
        whole: bool,
    ) -> Result<u32> {
        let done: HashSet<u64> = posted
            .threads
            .iter()
            .flat_map(|t| std::iter::once(t.key).chain(t.replies.iter().map(|r| r.key)))
            .chain(posted.replies.iter().chain(&posted.comments).map(|p| p.key))
            .collect();
        let left: Vec<u64> = keys_in_turn(submission)
            .into_iter()
            .filter(|k| !done.contains(k))
            .collect();
        let asked = match (&posted.failed, whole) {
            (None, _) => 0,
            (Some(_), true) => left.len(),
            (Some(_), false) => left.len().min(1),
        };
        let mut failed = 0;
        for (i, key) in left.into_iter().enumerate() {
            let outgoing = match &posted.failed {
                Some(PlatformError::Unreachable(_)) if i < asked => continue,
                Some(error) if i < asked => {
                    failed += 1;
                    Outgoing::Failed {
                        error: error.to_string(),
                    }
                }
                _ => Outgoing::Draft,
            };
            self.set_outgoing(st, MessageId(key), outgoing).await?;
        }
        Ok(failed)
    }

    /// Records how far the viewer's draft `message` is on its way to the platform.
    async fn set_outgoing(&self, st: &mut Held<'_, S::Writes>, message: MessageId, outgoing: Outgoing) -> Result<()> {
        let Ok((thread, old)) = find_message(st, message) else {
            return Ok(());
        };
        if old.remote.is_some() || old.outgoing == outgoing {
            return Ok(());
        }
        let thread = thread.id;
        let changed = Message {
            outgoing,
            ..old.clone()
        };
        put_message(st, thread, &changed).await
    }

    /// Settles the drafts a post left posting, because Vet stopped or lost the platform's answer
    /// between asking it and hearing back, by what the platform has: a comment of the viewer's with
    /// the same text in the same place, made since posting started and not already Vet's, is the
    /// draft posted; without one the draft is a draft again. Answers whether anything changed.
    pub(super) async fn settle_posting(&self, st: &mut Held<'_, S::Writes>, discussion: &Discussion) -> Result<bool> {
        let posting: Vec<(ThreadId, MessageId)> = st
            .threads
            .iter()
            .flat_map(|t| {
                t.messages
                    .iter()
                    .filter(|m| m.remote.is_none() && matches!(m.outgoing, Outgoing::Posting { .. }))
                    .map(|m| (t.id, m.id))
            })
            .collect();
        if posting.is_empty() {
            return Ok(false);
        }
        let mut threads_known: HashSet<String> = st
            .threads
            .iter()
            .filter_map(|t| t.platform.as_ref()?.remote_id.clone())
            .collect();
        let mut comments_known: HashSet<String> = st
            .threads
            .iter()
            .flat_map(|t| t.messages.iter().filter_map(|m| Some(m.remote.as_ref()?.id.clone())))
            .collect();
        for (thread, message) in posting {
            // A flat thread goes with the last of its comments found on the platform.
            let Ok(t) = find_thread(st, thread).cloned() else {
                continue;
            };
            let Some(m) = t.messages.iter().find(|m| m.id == message).cloned() else {
                continue;
            };
            let Outgoing::Posting { since } = m.outgoing else {
                continue;
            };
            let after = since - POSTING_SLACK;
            let fits = |c: &PlatformComment| {
                c.mine && c.body == m.body && c.created_at >= after && !comments_known.contains(&c.id)
            };
            if t.is_flat() {
                let said = discussion.conversation.iter().find(|i| {
                    i.kind == ConversationKind::Comment { mine: true }
                        && i.body == m.body
                        && i.at >= after
                        && !comments_known.contains(&i.id)
                });
                match said {
                    Some(item) => {
                        comments_known.insert(item.id.clone());
                        self.drop_message(st, thread, message).await?;
                    }
                    None => self.set_outgoing(st, message, Outgoing::Draft).await?,
                }
                continue;
            }
            let found = match t.platform.as_ref().and_then(|p| p.remote_id.clone()) {
                Some(id) => discussion
                    .threads
                    .iter()
                    .find(|d| d.id == id)
                    .and_then(|d| d.comments.iter().find(|c| fits(c)))
                    .map(|c| (None, c)),
                None if t.messages.first().map(|f| f.id) == Some(message) => {
                    let path = posted_path(&t);
                    discussion
                        .threads
                        .iter()
                        .filter(|d| d.path == path && !threads_known.contains(&d.id))
                        .find_map(|d| d.comments.first().filter(|c| fits(c)).map(|c| (Some(d.id.clone()), c)))
                }
                // A reply in a new thread that the platform does not have.
                None => None,
            };
            let Some((new_thread, comment)) = found else {
                self.set_outgoing(st, message, Outgoing::Draft).await?;
                continue;
            };
            comments_known.insert(comment.id.clone());
            if let Some(id) = new_thread {
                threads_known.insert(id.clone());
                let mut platform = t.platform.clone().expect("a draft thread");
                platform.remote_id = Some(id);
                put_platform(st, thread, platform).await?;
            }
            self.record_comment(st, thread, message, comment).await?;
        }
        Ok(true)
    }

    async fn record_posted(&self, st: &mut Held<'_, S::Writes>, posted: &Submitted) -> Result<u32> {
        let mut count = 0;
        for new in &posted.threads {
            let Some(thread) = st
                .threads
                .iter()
                .find(|t| t.messages.first().is_some_and(|m| m.id.0 == new.key))
            else {
                continue;
            };
            let mut platform = thread.platform.clone().expect("a draft thread");
            platform.remote_id = Some(new.thread.clone());
            let id = thread.id;
            put_platform(st, id, platform).await?;
            self.record_comment(st, id, MessageId(new.key), &new.comment).await?;
            for reply in &new.replies {
                self.record_comment(st, id, MessageId(reply.key), &reply.comment)
                    .await?;
                count += 1;
            }
            count += 1;
        }
        for reply in &posted.replies {
            if let Some(thread) = thread_of(st, reply) {
                self.record_comment(st, thread, MessageId(reply.key), &reply.comment)
                    .await?;
                count += 1;
            }
        }
        // A comment on the whole pull request is a thread of its own, which the platform has once
        // its first comment is there. A flat one belongs to the platform's conversation once posted,
        // and is shown there.
        for comment in &posted.comments {
            let Some(id) = thread_of(st, comment) else { continue };
            if find_thread(st, id)?.is_flat() {
                self.drop_message(st, id, MessageId(comment.key)).await?;
                let (remote, number) = pull_request(st)?;
                self.exchange.cache.amend_discussion(&remote, number, |discussion| {
                    discussion.conversation.push(ConversationItem {
                        id: comment.comment.id.clone(),
                        author: comment.comment.author.clone(),
                        avatar: comment.comment.avatar.clone(),
                        at: comment.comment.created_at,
                        kind: ConversationKind::Comment { mine: true },
                        body: comment.comment.body.clone(),
                    });
                });
                count += 1;
                continue;
            }
            self.record_comment(st, id, MessageId(comment.key), &comment.comment)
                .await?;
            let thread = find_thread(st, id)?;
            if !thread.is_posted() {
                let mut platform = thread.platform.clone().expect("a draft thread");
                platform.remote_id = Some(comment.comment.id.clone());
                put_platform(st, id, platform).await?;
            }
            count += 1;
        }
        Ok(count)
    }

    /// Notes that the draft `message` of `thread` is the platform's `comment` now.
    async fn record_comment(
        &self,
        st: &mut Held<'_, S::Writes>,
        thread: ThreadId,
        message: MessageId,
        comment: &PlatformComment,
    ) -> Result<()> {
        let Some(old) = find_thread(st, thread)?
            .messages
            .iter()
            .find(|m| m.id == message)
            .cloned()
        else {
            return Ok(());
        };
        let mut posted = old;
        posted.avatar = comment.avatar.clone();
        posted.outgoing = Outgoing::Draft;
        posted.remote = Some(RemoteComment {
            id: comment.id.clone(),
            synced: comment.body.clone(),
            conflict: None,
        });
        put_message(st, thread, &posted).await
    }

    /// Removes `message` from `thread`, and the thread with its last message.
    async fn drop_message(&self, st: &mut Held<'_, S::Writes>, thread: ThreadId, message: MessageId) -> Result<()> {
        let store = st.writes();
        if find_thread(st, thread)?.messages.iter().all(|m| m.id == message) {
            store.delete_thread(thread).await?;
            st.threads.retain(|t| t.id != thread);
        } else {
            store.delete_message(message).await?;
            edit_thread(st, thread, |t| t.messages.retain(|m| m.id != message));
        }
        Ok(())
    }
}

/// How far before the start of a post, by Vet's clock, the platform's clock may date a comment
/// that post made.
const POSTING_SLACK: i64 = 600;

/// The keys of the drafts of `submission` in the order the platforms post them.
fn keys_in_turn(submission: &Submission) -> Vec<u64> {
    let mut keys = Vec::new();
    for new in &submission.threads {
        keys.push(new.key);
        keys.extend(new.replies.iter().map(|r| r.key));
    }
    keys.extend(submission.replies.iter().map(|r| r.key));
    keys.extend(submission.comments.iter().map(|c| c.key));
    keys
}

/// The path the platform lists a thread posted from `thread` under: none for one on the pull
/// request as a whole.
fn posted_path(thread: &Thread) -> String {
    match &thread.target {
        CommentTarget::Range(a) => a.file.to_string_lossy(),
        CommentTarget::File { path, .. } => path.to_string_lossy(),
        CommentTarget::Changeset => String::new(),
    }
}

/// The texts of the drafts of `submission` that `posted` does not hold.
fn not_posted(submission: &Submission, posted: &Submitted) -> Vec<String> {
    let mut texts = Vec::new();
    let has_reply = |key: u64| {
        posted.replies.iter().any(|p| p.key == key)
            || posted.threads.iter().any(|t| t.replies.iter().any(|p| p.key == key))
    };
    for new in &submission.threads {
        if !posted.threads.iter().any(|t| t.key == new.key) {
            texts.push(new.body.clone());
        }
        texts.extend(new.replies.iter().filter(|r| !has_reply(r.key)).map(|r| r.body.clone()));
    }
    texts.extend(
        submission
            .replies
            .iter()
            .filter(|r| !has_reply(r.key))
            .map(|r| r.body.clone()),
    );
    texts.extend(
        submission
            .comments
            .iter()
            .filter(|c| {
                !posted.comments.iter().any(|p| p.key == c.key) && !posted.threads.iter().any(|t| t.key == c.key)
            })
            .map(|c| c.body.clone()),
    );
    texts
}

/// The thread the draft that `posted` is the platform's version of is in.
fn thread_of(st: &State, posted: &PostedComment) -> Option<ThreadId> {
    st.threads
        .iter()
        .find(|t| t.messages.iter().any(|m| m.id.0 == posted.key))
        .map(|t| t.id)
}

/// The drafts of the threads in `st` as a review, and how many cannot be posted.
fn drafts_of(st: &State, verdict: Verdict, body: Option<String>) -> (Submission, u32) {
    let mut submission = Submission {
        verdict,
        body,
        commit: st.head.0.clone(),
        threads: Vec::new(),
        replies: Vec::new(),
        comments: Vec::new(),
    };
    let mut skipped = 0;
    for thread in st.threads.iter().filter(|t| t.platform.is_some()) {
        let drafts: Vec<&Message> = thread.messages.iter().filter(|m| m.remote.is_none()).collect();
        if drafts.is_empty() {
            continue;
        }
        // Posting from a thread waits until its edit conflict is settled, and until the next sync
        // found out whether the platform has what an earlier post sent of it.
        if thread.messages.iter().any(|m| {
            m.remote.as_ref().is_some_and(|r| r.conflict.is_some()) || matches!(m.outgoing, Outgoing::Posting { .. })
        }) {
            skipped += sat_u32(drafts.len());
            continue;
        }
        // A flat comment is no thread on the platform, so nothing in it is a reply in the review:
        // each of its drafts is a comment on the pull request of its own.
        if thread.is_flat() {
            submission.comments.extend(conversation_comments(&drafts));
            continue;
        }
        if thread.is_posted() {
            let id = thread
                .platform
                .as_ref()
                .and_then(|p| p.remote_id.clone())
                .expect("posted");
            submission.replies.extend(drafts.iter().map(|m| NewReply {
                key: m.id.0,
                thread: id.clone(),
                body: m.body.clone(),
            }));
            continue;
        }
        if thread.target == CommentTarget::Changeset {
            submission.comments.extend(conversation_comments(&drafts));
            continue;
        }
        let place = place_for_posting(st, thread);
        let Some((path, side, lines)) = place else {
            skipped += sat_u32(drafts.len());
            continue;
        };
        let mut texts = drafts.into_iter();
        let opener = texts.next().expect("a draft");
        submission.threads.push(NewThread {
            key: opener.id.0,
            path,
            side,
            lines,
            body: opener.body.clone(),
            replies: texts
                .map(|m| NewReply {
                    key: m.id.0,
                    thread: String::new(),
                    body: m.body.clone(),
                })
                .collect(),
        });
    }
    (submission, skipped)
}

/// Each of `drafts` as a comment on the pull request of its own.
fn conversation_comments<'a>(drafts: &'a [&Message]) -> impl Iterator<Item = NewConversationComment> + 'a {
    drafts.iter().map(|m| NewConversationComment {
        key: m.id.0,
        body: m.body.clone(),
    })
}

/// The file, the side and the 1-based inclusive lines (none for the whole file) of a draft.
type Place = (String, DiffSide, Option<(u32, u32)>);

/// Where a draft thread is posted: the file, the side and the 1-based lines, as the code is now;
/// `None` for a thread with no place left, which includes one whose lines a push deleted: it shows
/// at the deletion point, but there is no code there to comment on.
fn place_for_posting(st: &State, thread: &Thread) -> Option<Place> {
    let view = thread_view(st, thread).ok()?;
    if matches!(
        view.state,
        Some(AnchorState::Inferred {
            strategy: Strategy::Deleted,
            ..
        })
    ) {
        return None;
    }
    let side = |s: Side| match s {
        Side::Base => DiffSide::Left,
        Side::Target => DiffSide::Right,
    };
    match (&thread.target, &view.state) {
        (CommentTarget::Range(a), Some(AnchorState::Exact { .. } | AnchorState::Inferred { .. })) => {
            let at = view.location.as_ref()?;
            Some((
                at.file.to_string_lossy(),
                side(a.side),
                Some((at.range.start + 1, at.range.end.max(at.range.start + 1))),
            ))
        }
        (CommentTarget::File { side: s, .. }, Some(AnchorState::Exact { .. })) => {
            Some((view.path.as_ref()?.to_string_lossy(), side(*s), None))
        }
        _ => None,
    }
}
