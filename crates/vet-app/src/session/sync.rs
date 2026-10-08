//! Threads of a pull request exchanged with its platform (SPEC M1 "Thread sync"): pulled in when
//! the pull request opens, on fetch and after a review is submitted; edits to posted comments
//! pushed at once; drafts posted together as one review. The platform is asked with the session's
//! lock released, so what the viewer does meanwhile is never held up by the network.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use vet_core::content::{EntryKind, LineRange, RepoPath, Snapshot, SnapshotStatus, is_binary};
use vet_core::review::{
    Anchor, AnchorState, CommentTarget, EditConflict, Message, MessageId, Outgoing, PlatformThread, RemoteComment,
    Side, Thread, ThreadId, ThreadStatus,
};

use super::state::{State, View};
use super::threads::{ThreadView, edit_thread, find_thread, side_snapshot, thread_by_id, thread_view};
use super::{LiveSession, Result, SessionError};
use crate::changeset::{CommitId, Source};
use crate::platform::{
    CommentKind, CommentRef, ConversationItem, ConversationKind, DiffSide, Discussion, MessageSync,
    NewConversationComment, NewReply, NewThread, Platform, PlatformComment, PlatformError, PlatformThreadData,
    PostedComment, Posting, RemoteKey, StatusSync, Submission, Submitted, SyncPlan, reconcile,
};
use crate::ports::{GitRepo, LiveStore};
use crate::review::visible_threads;

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
    /// The platform refused or could not take something Vet had to send; the rest was synced.
    pub problem: Option<PlatformError>,
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

fn message_of(c: &PlatformComment) -> Message {
    Message {
        id: MessageId(0),
        author: c.author.clone(),
        body: c.body.clone(),
        at: c.created_at,
        mine: c.mine,
        avatar: c.avatar.clone(),
        edited_at: None,
        remote: Some(RemoteComment {
            id: c.id.clone(),
            synced: c.body.clone(),
            conflict: None,
        }),
        outgoing: Outgoing::Draft,
    }
}

/// The thread's file and line as a person says it: `job.rs:96`.
fn place_of(thread: &Thread) -> String {
    let name = |path: &RepoPath| {
        let text = path.to_string_lossy();
        text.rsplit('/').next().unwrap_or(&text).to_owned()
    };
    match &thread.target {
        CommentTarget::Range(a) => format!("{}:{}", name(&a.file), a.range.start + 1),
        CommentTarget::File { path, .. } => name(path),
        CommentTarget::Changeset => "the pull request".to_owned(),
    }
}

/// What a sync sends before it changes anything of its own.
#[derive(Default)]
struct Pushes {
    statuses: Vec<(ThreadId, String, bool)>,
    edits: Vec<(MessageId, CommentRef, String)>,
}

#[derive(Default)]
struct Pushed {
    statuses: HashSet<ThreadId>,
    /// The text of each edit that went through.
    edits: HashMap<MessageId, String>,
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// What the platform last said of the pull request on view besides its threads; `None` for
    /// another changeset, and before the first sync.
    pub async fn discussion(&self) -> Option<DiscussionView> {
        let st = self.published.read();
        let Source::PullRequest { number, .. } = &st.source else {
            return None;
        };
        let (synced_at, discussion) = st.discussion.clone()?;
        let unresolved_threads = st
            .threads
            .iter()
            .filter(|t| {
                t.is_posted() && t.status == ThreadStatus::Open && t.platform.as_ref().is_some_and(|p| p.resolvable)
            })
            .count();
        Some(DiscussionView {
            number: *number,
            synced_at,
            discussion,
            unresolved_threads: u32::try_from(unresolved_threads).unwrap_or(u32::MAX),
        })
    }

    /// Brings the threads of the pull request on view up to date with the platform's, both ways:
    /// the platform's threads and comments come in; a status the viewer changed since the last sync
    /// is sent and wins; an edit of a posted comment that was not sent is sent, or kept beside the
    /// platform's text if that changed too; a thread the platform no longer has goes, with the
    /// drafts in it. Fails, changing nothing, if the platform cannot be asked.
    pub async fn sync_threads<P: Platform + Sync>(&self, platform: &P) -> Result<SyncReport> {
        let _exchange = self.exchange.lock().await;
        let source = self.published.read().source.clone();
        let Source::PullRequest { remote, number } = &source else {
            return Err(SessionError::Invalid(
                "the changeset on view is not a pull request".into(),
            ));
        };
        let discussion = platform.discussion(remote, *number).await?;

        let (plan, pushes, settled) = {
            let mut st = self.hold().await;
            if st.source != source {
                return Ok(SyncReport::default());
            }
            let settled = self.settle_posting(&mut st, &discussion).await?;
            let plan = reconcile(&st.threads, &discussion, platform.traits().changeset_threads);
            let pushes = pushes_of(&st.threads, &plan);
            (plan, pushes, settled)
        };
        let (pushed, problem) = push(platform, remote, pushes).await;

        let mut st = self.hold().await;
        if st.source != source {
            return Ok(SyncReport::default());
        }
        let mut report = self.apply(&mut st, &plan, &pushed, &discussion).await?;
        report.changed |= settled;
        report.problem = problem;
        Ok(report)
    }

    async fn apply(
        &self,
        st: &mut State,
        plan: &SyncPlan,
        pushed: &Pushed,
        discussion: &Discussion,
    ) -> Result<SyncReport> {
        let store = &self.acquisition.store;
        let now = (self.clock)();
        let mut report = SyncReport::default();
        let changeset = match st.changeset {
            Some(changeset) => changeset,
            None => {
                let changeset = store.ensure_changeset(self.repo.clone(), st.source.clone()).await?;
                st.changeset = Some(changeset);
                changeset
            }
        };

        for data in &plan.added {
            let target = self.target_of_remote(st, data, now).await?;
            let mut comments = data.comments.iter().map(message_of);
            let Some(opener) = comments.next() else { continue };
            let platform = PlatformThread {
                remote_id: Some(data.id.clone()),
                resolvable: true,
                resolved_by: data.resolved_by.clone(),
                status_pending: false,
            };
            let status = if data.resolved {
                ThreadStatus::Resolved
            } else {
                ThreadStatus::Open
            };
            let (thread, _) = store
                .add_thread(changeset, None, target, opener, Some(platform), status)
                .await?;
            for reply in comments {
                store.add_message(thread, reply).await?;
            }
            report.changed = true;
        }

        for sync in &plan.threads {
            let Ok(thread) = find_thread(st, sync.thread) else {
                continue;
            };
            let mut platform = thread.platform.clone().expect("a posted thread");
            let own = thread.messages.clone();
            match &sync.status {
                StatusSync::Unchanged => {}
                StatusSync::Remote { status, resolved_by } => {
                    store.set_thread_status(sync.thread, *status).await?;
                    platform.resolved_by.clone_from(resolved_by);
                    report.changed = true;
                }
                StatusSync::Settled { resolved_by } => {
                    platform.resolved_by.clone_from(resolved_by);
                    platform.status_pending = false;
                }
                StatusSync::PushLocal { .. } => {
                    if pushed.statuses.contains(&sync.thread) {
                        platform.status_pending = false;
                    }
                }
            }
            if Some(&platform) != find_thread(st, sync.thread)?.platform.as_ref() {
                store.set_thread_platform(sync.thread, platform).await?;
                report.changed = true;
            }
            for id in &sync.removed {
                store.delete_message(*id).await?;
                report.changed = true;
            }
            for comment in &sync.added {
                store.add_message(sync.thread, message_of(comment)).await?;
                report.changed = true;
            }
            for (id, change) in &sync.messages {
                let Some(old) = own.iter().find(|m| m.id == *id) else {
                    continue;
                };
                let mut message = old.clone();
                let remote = message.remote.as_mut().expect("posted");
                match change {
                    MessageSync::Unchanged => continue,
                    MessageSync::TakeRemote { body, .. } => {
                        message.body.clone_from(body);
                        remote.synced.clone_from(body);
                        remote.conflict = None;
                    }
                    MessageSync::Agreed { body } => {
                        remote.synced.clone_from(body);
                        remote.conflict = None;
                    }
                    MessageSync::PushLocal => {
                        let Some(sent) = pushed.edits.get(id) else { continue };
                        remote.synced.clone_from(sent);
                        remote.conflict = None;
                    }
                    MessageSync::Conflict { remote_body, remote_at } => {
                        remote.conflict = Some(EditConflict {
                            remote_body: remote_body.clone(),
                            remote_at: *remote_at,
                        });
                    }
                }
                store.replace_message(message).await?;
                report.changed = true;
            }
        }

        for removed in &plan.removed {
            let Ok(thread) = find_thread(st, removed.thread) else {
                continue;
            };
            report
                .notices
                .extend((!removed.drafts.is_empty()).then(|| SyncNotice::ThreadDeleted {
                    place: place_of(thread),
                    dropped: removed.drafts.clone(),
                }));
            store.delete_thread(removed.thread).await?;
            report.changed = true;
        }

        st.discussion = Some((now, discussion.clone()));
        if report.changed {
            self.reload_threads(st, changeset).await?;
        }
        Ok(report)
    }

    /// Reads the threads from the store again, as they are after something changed them there.
    async fn reload_threads(&self, st: &mut State, changeset: crate::changeset::ChangesetId) -> Result<()> {
        let open = st.review.as_ref().map(|r| r.id);
        st.threads = visible_threads(self.acquisition.store.threads(changeset).await?, open);
        // Without the diff the anchors are resolved with the remap that comes with it.
        if st.changes.is_some() {
            self.resolve_threads(st).await?;
        }
        Ok(())
    }

    /// Where a thread the platform has is anchored: in the commit its first comment was made on,
    /// for the lines it names, so that where the code went since is for the anchor to resolve. A
    /// thread whose lines cannot be found is attached to its file, or to the pull request.
    async fn target_of_remote(&self, st: &State, data: &PlatformThreadData, now: i64) -> Result<CommentTarget> {
        let side = match data.side {
            DiffSide::Left => Side::Base,
            DiffSide::Right => Side::Target,
        };
        let path = RepoPath::from(data.path.as_str());
        let current = side_snapshot(st, side);
        let fallback = || {
            if current.entries.contains_key(&path) {
                CommentTarget::File {
                    side,
                    path: path.clone(),
                }
            } else {
                CommentTarget::Changeset
            }
        };
        let Some((first, last)) = data.lines else {
            return Ok(fallback());
        };
        let snapshot: Option<Arc<Snapshot>> = match (side, data.comments.first().and_then(|c| c.commit.clone())) {
            (Side::Base, _) => Some(st.base.clone()),
            (Side::Target, Some(commit)) if commit != st.head.0 => self
                .acquisition
                .git
                .commit_snapshot(&CommitId(commit))
                .await
                .ok()
                .map(Arc::new),
            (Side::Target, _) => Some(st.target.clone()),
        };
        let Some(snapshot) = snapshot else {
            return Ok(fallback());
        };
        let Some(entry) = snapshot.entries.get(&path).filter(|e| e.kind == EntryKind::File) else {
            return Ok(fallback());
        };
        let mut blobs = BTreeMap::new();
        if !self.acquisition.fetch(&mut blobs, &path, entry.blob, true).await? || is_binary(&blobs[&entry.blob]) {
            return Ok(fallback());
        }
        let range = LineRange {
            start: first.saturating_sub(1),
            end: last.max(first),
        };
        let Some(anchor) = Anchor::from_text(side, snapshot.id, path.clone(), &blobs[&entry.blob], range) else {
            return Ok(fallback());
        };
        // The snapshot the anchor is placed in must outlive git dropping the commit.
        let store = &self.acquisition.store;
        store
            .put_snapshot(Snapshot::clone(&snapshot), SnapshotStatus::of(&snapshot), now)
            .await?;
        self.acquisition.keep(&path, entry.blob).await?;
        Ok(CommentTarget::Range(anchor))
    }

    /// Changes the text of one of the viewer's own comments. A draft or a comment outside a
    /// platform changes here. A posted one is also changed on the platform at once; if the platform
    /// has another text than the one this was based on, nothing is sent and the thread shows both
    /// texts until the viewer keeps one (see [`LiveSession::keep_edit`]). A comment the platform
    /// cannot be told about keeps the new text here, and the next sync sends it.
    pub async fn edit_message<P: Platform + Sync>(
        &self,
        platform: &P,
        message: MessageId,
        body: String,
        now: i64,
    ) -> Result<ThreadView> {
        if body.trim().is_empty() {
            return Err(SessionError::Invalid("a message cannot be empty".into()));
        }
        let _exchange = self.exchange.lock().await;
        let (thread, mut edited, remote, comment) = {
            let mut st = self.hold().await;
            let (thread, old) = find_message(&st, message)?;
            if !old.mine {
                return Err(SessionError::Invalid("only your own comments can be edited".into()));
            }
            if old.remote.as_ref().is_some_and(|r| r.conflict.is_some()) {
                return Err(SessionError::Invalid(
                    "this comment was edited in two places: keep one of the two texts first".into(),
                ));
            }
            let mut edited = old.clone();
            let id = thread.id;
            edited.body = body;
            edited.edited_at = Some(now);
            let comment = comment_ref(thread, old);
            self.acquisition.store.replace_message(edited.clone()).await?;
            edit_thread(&mut st, id, |t| replace(t, &edited));
            let remote = comment.as_ref().map(|_| pull_request(&st)).transpose()?;
            (id, edited, remote, comment)
        };
        let (Some((remote, _)), Some(comment)) = (remote, comment) else {
            return self.view_of(thread).await;
        };

        // The platform's text is compared with the one the edit started from.
        let sent = async {
            let current = platform.comment(&remote, &comment).await?;
            let synced = edited.remote.as_ref().expect("posted").synced.clone();
            if current.body != synced && current.body != edited.body {
                return Ok(Err(current));
            }
            platform.edit_comment(&remote, &comment, &edited.body).await.map(Ok)
        }
        .await;
        match sent {
            Ok(Ok(posted)) => {
                let r = edited.remote.as_mut().expect("posted");
                r.synced = posted.body;
                r.conflict = None;
            }
            Ok(Err(current)) => {
                edited.remote.as_mut().expect("posted").conflict = Some(EditConflict {
                    remote_body: current.body,
                    remote_at: current.edited_at,
                });
            }
            Err(e) => return Err(e.into()),
        }
        self.store_message(thread, edited).await?;
        self.view_of(thread).await
    }

    /// Settles an edit made in two places by keeping one of the texts.
    pub async fn keep_edit<P: Platform + Sync>(
        &self,
        platform: &P,
        message: MessageId,
        keep: Keep,
    ) -> Result<ThreadView> {
        let _exchange = self.exchange.lock().await;
        let (thread, mut kept, comment, remote) = {
            let st = self.published.read();
            let (thread, old) = find_message(&st, message)?;
            let conflict = old
                .remote
                .as_ref()
                .and_then(|r| r.conflict.clone())
                .ok_or_else(|| SessionError::Invalid("the comment was not edited in two places".into()))?;
            let mut kept = old.clone();
            if keep == Keep::Remote {
                let r = kept.remote.as_mut().expect("posted");
                kept.body = conflict.remote_body.clone();
                r.synced = conflict.remote_body;
                r.conflict = None;
                kept.edited_at = None;
            }
            (
                thread.id,
                kept,
                comment_ref(thread, old).expect("posted"),
                pull_request(&st)?.0,
            )
        };
        if keep == Keep::Vet {
            let posted = platform.edit_comment(&remote, &comment, &kept.body).await?;
            let r = kept.remote.as_mut().expect("posted");
            r.synced = posted.body;
            r.conflict = None;
        }
        self.store_message(thread, kept).await?;
        self.view_of(thread).await
    }

    /// Removes one of the viewer's comments. A thread's first removes the thread, unless the thread
    /// is posted to a platform that keeps it with its other posted comments
    /// ([`crate::platform::PlatformTraits::first_comment_ends_thread`]). A posted one is removed on
    /// the platform first.
    pub async fn remove_message<P: Platform + Sync>(&self, platform: &P, message: MessageId) -> Result<()> {
        let _exchange = self.exchange.lock().await;
        let (thread, ends_thread, posted) = {
            let st = self.published.read();
            let (thread, old) = find_message(&st, message)?;
            if !old.mine {
                return Err(SessionError::Invalid("only your own comments can be removed".into()));
            }
            let posted = comment_ref(thread, old)
                .map(|c| pull_request(&st).map(|(remote, _)| (remote, c)))
                .transpose()?;
            let first = thread.messages.first().map(|m| m.id) == Some(message);
            let others_posted = thread.messages.iter().any(|m| m.id != message && m.remote.is_some());
            let ends_thread =
                first && (posted.is_none() || platform.traits().first_comment_ends_thread || !others_posted);
            (thread.id, ends_thread, posted)
        };
        if let Some((remote, comment)) = posted {
            match platform.delete_comment(&remote, &comment).await {
                Ok(()) | Err(PlatformError::NotFound(_)) => {}
                Err(e) => return Err(e.into()),
            }
        }
        let mut st = self.hold().await;
        if ends_thread {
            self.acquisition.store.delete_thread(thread).await?;
            st.threads.retain(|t| t.id != thread);
            if let Some(changes) = st.changes.as_mut() {
                changes.remapped.resolutions.remove(&thread);
            }
        } else {
            self.acquisition.store.delete_message(message).await?;
            edit_thread(&mut st, thread, |t| t.messages.retain(|m| m.id != message));
        }
        Ok(())
    }

    /// Deletes a thread; this is also how a detached one is discarded. A thread the platform has
    /// is removed there too, which only its author can, by removing its first comment: on a
    /// platform that keeps a thread with its other comments, they stay.
    pub async fn delete_thread<P: Platform + Sync>(&self, platform: &P, thread: ThreadId) -> Result<()> {
        let first = {
            let st = self.published.read();
            let thread = find_thread(&st, thread)?;
            thread
                .is_posted()
                .then(|| thread.messages.first().map(|m| m.id))
                .flatten()
        };
        if let Some(first) = first {
            return self.remove_message(platform, first).await;
        }
        let mut st = self.hold().await;
        find_thread(&st, thread)?;
        self.acquisition.store.delete_thread(thread).await?;
        st.threads.retain(|t| t.id != thread);
        if let Some(changes) = st.changes.as_mut() {
            changes.remapped.resolutions.remove(&thread);
        }
        Ok(())
    }

    /// Posts the viewer's drafts as one review with `verdict` (an id of the platform's), then
    /// records where they are on the platform. Drafts whose code is gone, and those in a thread
    /// with an edit conflict, wait. Fails, posting nothing of the review, if the platform refuses.
    pub async fn submit_review<P: Platform + Sync>(
        &self,
        platform: &P,
        verdict: &str,
        body: Option<String>,
    ) -> Result<SubmitReport> {
        let traits = platform.traits();
        if traits.posting != Posting::BatchedReview {
            return Err(SessionError::Invalid(
                "this platform takes comments one at a time: post them instead".into(),
            ));
        }
        if !traits.verdicts.iter().any(|v| v.id == verdict) {
            return Err(SessionError::Invalid(format!("the platform has no verdict {verdict}")));
        }
        let body = body.map(|b| b.trim().to_owned()).filter(|b| !b.is_empty());
        self.send_drafts(platform, verdict, body, verdict != "COMMENT").await
    }

    /// Posts the viewer's drafts one after the other, on a platform that takes comments one at a
    /// time, and records where they are on the platform. Drafts whose code is gone, and those in a
    /// thread with an edit conflict, wait. Stops at the first the platform refuses; what was
    /// posted before it stays posted.
    pub async fn post_drafts<P: Platform + Sync>(&self, platform: &P) -> Result<SubmitReport> {
        if platform.traits().posting != Posting::PerComment {
            return Err(SessionError::Invalid(
                "this platform takes the drafts as a review: submit it instead".into(),
            ));
        }
        self.send_drafts(platform, "", None, false).await
    }

    /// Sets the viewer's vote on a platform that keeps it apart from posting; `verdict` is an id of
    /// the platform's.
    pub async fn set_vote<P: Platform + Sync>(&self, platform: &P, verdict: &str) -> Result<()> {
        let traits = platform.traits();
        if !traits.verdict_separate {
            return Err(SessionError::Invalid(
                "this platform sets the vote with the review".into(),
            ));
        }
        if !traits.verdicts.iter().any(|v| v.id == verdict) {
            return Err(SessionError::Invalid(format!("the platform has no verdict {verdict}")));
        }
        let (remote, number) = pull_request(&self.published.read())?;
        platform.vote(&remote, number, verdict).await?;
        Ok(())
    }

    async fn send_drafts<P: Platform + Sync>(
        &self,
        platform: &P,
        verdict: &str,
        body: Option<String>,
        allow_empty: bool,
    ) -> Result<SubmitReport> {
        let _exchange = self.exchange.lock().await;
        pull_request(&self.published.read())?;
        let (remote, number, submission, skipped) = {
            let (mut st, _) = self.writing(View::Changeset).await?;
            let (remote, number) = pull_request(&st)?;
            let (submission, skipped) = drafts_of(&st, verdict, body.clone());
            if submission.threads.is_empty()
                && submission.replies.is_empty()
                && submission.comments.is_empty()
                && body.is_none()
                && !allow_empty
            {
                return Err(SessionError::Invalid("there is nothing to submit".into()));
            }
            let since = (self.clock)();
            for key in keys_in_turn(&submission) {
                self.set_outgoing(&mut st, MessageId(key), Outgoing::Posting { since })
                    .await?;
            }
            (remote, number, submission, skipped)
        };
        let (posted, whole) = match platform.submit_review(&remote, number, &submission).await {
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
        st: &mut State,
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
    async fn set_outgoing(&self, st: &mut State, message: MessageId, outgoing: Outgoing) -> Result<()> {
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
        self.acquisition.store.replace_message(changed.clone()).await?;
        edit_thread(st, thread, |t| replace(t, &changed));
        Ok(())
    }

    /// Settles the drafts a post left posting, because Vet stopped or lost the platform's answer
    /// between asking it and hearing back, by what the platform has: a comment of the viewer's with
    /// the same text in the same place, made since posting started and not already Vet's, is the
    /// draft posted; without one the draft is a draft again. Answers whether anything changed.
    async fn settle_posting(&self, st: &mut State, discussion: &Discussion) -> Result<bool> {
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
                self.acquisition
                    .store
                    .set_thread_platform(thread, platform.clone())
                    .await?;
                edit_thread(st, thread, |t| t.platform = Some(platform));
            }
            self.record_comment(st, thread, message, comment).await?;
        }
        Ok(true)
    }

    async fn record_posted(&self, st: &mut State, posted: &Submitted) -> Result<u32> {
        let store = &self.acquisition.store;
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
            store.set_thread_platform(id, platform.clone()).await?;
            edit_thread(st, id, |t| t.platform = Some(platform));
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
                if let Some((_, discussion)) = st.discussion.as_mut() {
                    discussion.conversation.push(ConversationItem {
                        id: comment.comment.id.clone(),
                        author: comment.comment.author.clone(),
                        avatar: comment.comment.avatar.clone(),
                        at: comment.comment.created_at,
                        kind: ConversationKind::Comment { mine: true },
                        body: comment.comment.body.clone(),
                    });
                }
                count += 1;
                continue;
            }
            self.record_comment(st, id, MessageId(comment.key), &comment.comment)
                .await?;
            let thread = find_thread(st, id)?;
            if !thread.is_posted() {
                let mut platform = thread.platform.clone().expect("a draft thread");
                platform.remote_id = Some(comment.comment.id.clone());
                store.set_thread_platform(id, platform.clone()).await?;
                edit_thread(st, id, |t| t.platform = Some(platform));
            }
            count += 1;
        }
        Ok(count)
    }

    /// Notes that the draft `message` of `thread` is the platform's `comment` now.
    async fn record_comment(
        &self,
        st: &mut State,
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
        self.acquisition.store.replace_message(posted.clone()).await?;
        edit_thread(st, thread, |t| replace(t, &posted));
        Ok(())
    }

    /// Removes `message` from `thread`, and the thread with its last message.
    async fn drop_message(&self, st: &mut State, thread: ThreadId, message: MessageId) -> Result<()> {
        let store = &self.acquisition.store;
        if find_thread(st, thread)?.messages.iter().all(|m| m.id == message) {
            store.delete_thread(thread).await?;
            st.threads.retain(|t| t.id != thread);
        } else {
            store.delete_message(message).await?;
            edit_thread(st, thread, |t| t.messages.retain(|m| m.id != message));
        }
        Ok(())
    }

    /// Changes the text of one of the viewer's comments in the conversation of the pull request on
    /// view, on the platform at once.
    pub async fn edit_conversation_comment<P: Platform + Sync>(
        &self,
        platform: &P,
        comment: &str,
        body: String,
    ) -> Result<()> {
        if body.trim().is_empty() {
            return Err(SessionError::Invalid("a message cannot be empty".into()));
        }
        let _exchange = self.exchange.lock().await;
        let remote = {
            let st = self.published.read();
            own_conversation_comment(&st, comment)?;
            pull_request(&st)?.0
        };
        let posted = platform
            .edit_comment(&remote, &conversation_ref(comment), &body)
            .await?;
        let mut st = self.hold().await;
        if let Some(item) = conversation_mut(&mut st).and_then(|c| c.iter_mut().find(|i| i.id == comment)) {
            item.body = posted.body;
        }
        Ok(())
    }

    /// Deletes one of the viewer's comments in the conversation of the pull request on view, on the
    /// platform at once.
    pub async fn remove_conversation_comment<P: Platform + Sync>(&self, platform: &P, comment: &str) -> Result<()> {
        let _exchange = self.exchange.lock().await;
        let remote = {
            let st = self.published.read();
            own_conversation_comment(&st, comment)?;
            pull_request(&st)?.0
        };
        match platform.delete_comment(&remote, &conversation_ref(comment)).await {
            Ok(()) | Err(PlatformError::NotFound(_)) => {}
            Err(e) => return Err(e.into()),
        }
        let mut st = self.hold().await;
        if let Some(conversation) = conversation_mut(&mut st) {
            conversation.retain(|i| i.id != comment);
        }
        Ok(())
    }

    async fn store_message(&self, thread: ThreadId, message: Message) -> Result<()> {
        self.acquisition.store.replace_message(message.clone()).await?;
        let mut st = self.hold().await;
        edit_thread(&mut st, thread, |t| replace(t, &message));
        Ok(())
    }

    async fn view_of(&self, thread: ThreadId) -> Result<ThreadView> {
        let (st, _) = self.viewing(View::Changeset).await?;
        thread_by_id(&st, thread)
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

fn conversation_ref(comment: &str) -> CommentRef {
    CommentRef {
        id: comment.to_owned(),
        kind: CommentKind::Conversation,
    }
}

fn conversation_mut(st: &mut State) -> Option<&mut Vec<ConversationItem>> {
    st.discussion.as_mut().map(|(_, d)| &mut d.conversation)
}

/// Fails unless `comment` is a comment of the viewer's in the conversation the last sync found.
fn own_conversation_comment(st: &State, comment: &str) -> Result<()> {
    let item = st
        .discussion
        .as_ref()
        .and_then(|(_, d)| d.conversation.iter().find(|i| i.id == comment))
        .ok_or_else(|| SessionError::NotFound(format!("comment {comment}")))?;
    match item.kind {
        ConversationKind::Comment { mine: true } => Ok(()),
        _ => Err(SessionError::Invalid(
            "only your own comments on the pull request can be changed".into(),
        )),
    }
}

fn replace(thread: &mut Thread, message: &Message) {
    if let Some(m) = thread.messages.iter_mut().find(|m| m.id == message.id) {
        *m = message.clone();
    }
}

fn find_message(st: &State, id: MessageId) -> Result<(&Thread, &Message)> {
    st.threads
        .iter()
        .find_map(|t| t.messages.iter().find(|m| m.id == id).map(|m| (t, m)))
        .ok_or_else(|| SessionError::NotFound(format!("message {}", id.0)))
}

fn pushes_of(threads: &[Thread], plan: &SyncPlan) -> Pushes {
    let mut pushes = Pushes::default();
    for sync in &plan.threads {
        let Some(thread) = threads.iter().find(|t| t.id == sync.thread) else {
            continue;
        };
        if let (StatusSync::PushLocal { resolved }, Some(remote)) = (
            &sync.status,
            thread.platform.as_ref().and_then(|p| p.remote_id.as_ref()),
        ) {
            pushes.statuses.push((thread.id, remote.clone(), *resolved));
        }
        for (id, change) in &sync.messages {
            let message = thread.messages.iter().find(|m| m.id == *id);
            if let (MessageSync::PushLocal, Some(message)) = (change, message)
                && let Some(comment) = comment_ref(thread, message)
            {
                pushes.edits.push((*id, comment, message.body.clone()));
            }
        }
    }
    pushes
}

/// Sends what `pushes` holds, stopping at the first refusal, which is also returned.
async fn push<P: Platform + Sync>(platform: &P, remote: &RemoteKey, pushes: Pushes) -> (Pushed, Option<PlatformError>) {
    let mut pushed = Pushed::default();
    for (thread, id, resolved) in pushes.statuses {
        match platform.set_thread_resolved(remote, &id, resolved).await {
            Ok(()) => {
                pushed.statuses.insert(thread);
            }
            Err(e) => return (pushed, Some(e)),
        }
    }
    for (message, comment, body) in pushes.edits {
        match platform.edit_comment(remote, &comment, &body).await {
            Ok(sent) => {
                pushed.edits.insert(message, sent.body);
            }
            Err(e) => return (pushed, Some(e)),
        }
    }
    (pushed, None)
}

/// The drafts of the threads in `st` as a review, and how many cannot be posted.
fn drafts_of(st: &State, verdict: &str, body: Option<String>) -> (Submission, u32) {
    let mut submission = Submission {
        verdict: verdict.to_owned(),
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
            skipped += u32::try_from(drafts.len()).unwrap_or(u32::MAX);
            continue;
        }
        // A flat comment is no thread on the platform, so nothing in it is a reply in the review:
        // each of its drafts is a comment on the pull request of its own.
        if thread.is_flat() {
            submission
                .comments
                .extend(drafts.iter().map(|m| NewConversationComment {
                    key: m.id.0,
                    body: m.body.clone(),
                }));
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
            submission
                .comments
                .extend(drafts.iter().map(|m| NewConversationComment {
                    key: m.id.0,
                    body: m.body.clone(),
                }));
            continue;
        }
        let place = place_for_posting(st, thread);
        let Some((path, side, lines)) = place else {
            skipped += u32::try_from(drafts.len()).unwrap_or(u32::MAX);
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

/// The file, the side and the 1-based inclusive lines (none for the whole file) of a draft.
type Place = (String, DiffSide, Option<(u32, u32)>);

/// Where a draft thread is posted: the file, the side and the 1-based lines, as the code is now;
/// `None` for a thread with no place left.
fn place_for_posting(st: &State, thread: &Thread) -> Option<Place> {
    let view = thread_view(st, thread);
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
