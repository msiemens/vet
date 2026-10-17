use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use vet_core::content::{EntryKind, LineRange, RepoPath, Snapshot, SnapshotStatus, is_binary};
use vet_core::review::{
    Anchor, CommentTarget, EditConflict, Message, MessageId, Outgoing, PlatformThread, RemoteComment, Side, Thread,
    ThreadId, ThreadStatus,
};

use super::{SyncNotice, SyncReport, comment_ref, pull_request};
use crate::changeset::{CommitId, Source};
use crate::platform::{
    CommentRef, DiffSide, Discussion, MessageSync, Platform, PlatformComment, PlatformError, PlatformThreadData,
    RemoteKey, StatusSync, SyncPlan, reconcile,
};
use crate::ports::{GitRepo, LiveStore, ReviewWrites};
use crate::review::visible_threads;
use crate::session::hold::Held;
use crate::session::state::State;
use crate::session::threads::{find_thread, side_snapshot};
use crate::session::{LiveSession, Result, SessionError};

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
                return Ok(SyncReport::source_changed());
            }
            let settled = self.settle_posting(&mut st, &discussion).await?;
            let settled = self.settle_vote(&mut st, &discussion).await? || settled;
            let plan = reconcile(&st.threads, &discussion, platform.traits().changeset_threads);
            let pushes = pushes_of(&st.threads, &plan);
            (plan, pushes, settled)
        };
        let (pushed, problem) = push(platform, remote, pushes).await;

        let mut st = self.hold().await;
        if st.source != source {
            return Ok(SyncReport::source_changed());
        }
        let mut report = self.apply(&mut st, &plan, &pushed, &discussion).await?;
        report.changed |= settled;
        report.problem = problem;
        Ok(report)
    }

    async fn apply(
        &self,
        st: &mut Held<'_, S::Writes>,
        plan: &SyncPlan,
        pushed: &Pushed,
        discussion: &Discussion,
    ) -> Result<SyncReport> {
        let store = st.writes();
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

        // The writes below land one by one: whatever landed before one failed is read back.
        let stored: Result<()> = async {
            for data in &plan.added {
                let target = self.target_of_remote(&store, st, data, now).await?;
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
                report.changed = true;
                for reply in comments {
                    store.add_message(thread, reply).await?;
                }
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
            Ok(())
        }
        .await;

        if report.changed {
            self.reload_threads(st, changeset).await?;
        }
        stored?;
        let (remote, number) = pull_request(st)?;
        report.listing_changed = self.known.record_discussion(&remote, number, now, discussion.clone());
        // Kept for the next run: a revisit shows them before the platform is asked again.
        if let Some((head, updated_at)) = self.known.revision(&remote, number) {
            store
                .set_discussion(remote, number, head, updated_at, now, discussion)
                .await?;
        }
        Ok(report)
    }

    /// Reads the threads from the store again, as they are after something changed them there.
    /// If their anchors cannot be placed, the threads stay as they were.
    pub(in crate::session) async fn reload_threads(
        &self,
        st: &mut State,
        changeset: crate::changeset::ChangesetId,
    ) -> Result<()> {
        let open = st.review.as_ref().map(|r| r.id);
        let threads = visible_threads(self.acquisition.store.threads(changeset).await?, open);
        let previous = std::mem::replace(&mut st.threads, threads);
        // Without the diff the anchors are resolved with the remap that comes with it.
        if st.changes.is_some()
            && let Err(e) = self.resolve_threads(st).await
        {
            st.threads = previous;
            return Err(e);
        }
        Ok(())
    }

    /// Where a thread the platform has is anchored: in the commit its first comment was made on,
    /// for the lines it names, so that where the code went since is for the anchor to resolve. A
    /// thread whose lines cannot be found is attached to its file, or to the pull request.
    async fn target_of_remote(
        &self,
        store: &S::Writes,
        st: &State,
        data: &PlatformThreadData,
        now: i64,
    ) -> Result<CommentTarget> {
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
        store
            .put_snapshot(Snapshot::clone(&snapshot), SnapshotStatus::of(&snapshot), now)
            .await?;
        self.acquisition.keep(&path, entry.blob).await?;
        Ok(CommentTarget::Range(anchor))
    }
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
