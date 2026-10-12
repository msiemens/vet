//! Reconciling Vet's threads of a pull request with what the platform has (SPEC M1 "Thread sync"):
//! what is decided where the two disagree, as data. Applying it is the session's.

use vet_core::review::{CommentTarget, Message, MessageId, Thread, ThreadId, ThreadStatus};

use super::discussion::{Discussion, PlatformComment, PlatformThreadData};
use super::model::ChangesetThreads;

/// What becomes of a thread's state.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum StatusSync {
    Unchanged,
    /// The platform's state is taken over: the viewer did not change it since the last sync.
    Remote {
        status: ThreadStatus,
        resolved_by: Option<String>,
    },
    /// The viewer changed it since the last sync and the platform differs: the viewer's wins and
    /// is sent.
    PushLocal {
        resolved: bool,
    },
    /// The viewer changed it and the platform has that state already.
    Settled {
        resolved_by: Option<String>,
    },
}

/// What becomes of one comment the platform has and Vet has too.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MessageSync {
    Unchanged,
    /// Only the platform changed the text: Vet takes it over.
    TakeRemote {
        body: String,
        edited_at: i64,
    },
    /// Only the viewer changed the text (and it was not pushed yet): it is sent.
    PushLocal,
    /// Both changed it, differently: both texts are kept for the viewer to choose between.
    Conflict {
        remote_body: String,
        remote_at: i64,
    },
    /// Both changed it to the same text.
    Agreed {
        body: String,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ThreadSync {
    pub thread: ThreadId,
    pub status: StatusSync,
    /// Comments the platform has that Vet has not seen, in the platform's order.
    pub added: Vec<PlatformComment>,
    /// Comments Vet had from the platform that are gone from it.
    pub removed: Vec<MessageId>,
    /// Only for the comments that are not unchanged.
    pub messages: Vec<(MessageId, MessageSync)>,
}

/// A thread of Vet's that the platform no longer has.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RemovedThread {
    pub thread: ThreadId,
    /// The texts of the viewer's drafts in it, which go with it.
    pub drafts: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SyncPlan {
    /// Threads the platform has that Vet does not.
    pub added: Vec<PlatformThreadData>,
    pub threads: Vec<ThreadSync>,
    pub removed: Vec<RemovedThread>,
}

/// What to do so that `local` and `remote` agree. A posted thread (or comment) that the platform
/// does not list is gone from it, but only when the listing is complete. `changeset_threads` says
/// whether a comment on the whole pull request is among the threads the platform lists.
pub fn reconcile(local: &[Thread], remote: &Discussion, changeset_threads: ChangesetThreads) -> SyncPlan {
    let mut plan = SyncPlan::default();
    for data in &remote.threads {
        let known = local.iter().find(|t| remote_id(t) == Some(data.id.as_str()));
        match known {
            None => plan.added.push(data.clone()),
            Some(thread) => plan.threads.push(thread_sync(thread, data)),
        }
    }
    if remote.complete {
        // Where a comment on the whole pull request may be a flat one, the platform keeps it in its
        // conversation and not among the review threads.
        let listed =
            |t: &Thread| t.target != CommentTarget::Changeset || changeset_threads == ChangesetThreads::Resolvable;
        for thread in local.iter().filter(|t| t.is_posted() && listed(t)) {
            let id = remote_id(thread).expect("posted");
            if remote.threads.iter().any(|d| d.id == id) {
                continue;
            }
            plan.removed.push(RemovedThread {
                thread: thread.id,
                drafts: thread
                    .messages
                    .iter()
                    .filter(|m| m.remote.is_none())
                    .map(|m| m.body.clone())
                    .collect(),
            });
        }
    }
    plan
}

fn remote_id(thread: &Thread) -> Option<&str> {
    thread.platform.as_ref()?.remote_id.as_deref()
}

fn thread_sync(thread: &Thread, data: &PlatformThreadData) -> ThreadSync {
    let platform = thread.platform.as_ref().expect("a posted thread");
    let local_resolved = thread.status == ThreadStatus::Resolved;
    let status = if !platform.resolvable {
        StatusSync::Unchanged
    } else if platform.status_pending {
        if local_resolved == data.resolved {
            StatusSync::Settled {
                resolved_by: data.resolved_by.clone(),
            }
        } else {
            StatusSync::PushLocal {
                resolved: local_resolved,
            }
        }
    } else if local_resolved != data.resolved || platform.resolved_by != data.resolved_by {
        StatusSync::Remote {
            status: if data.resolved {
                ThreadStatus::Resolved
            } else {
                ThreadStatus::Open
            },
            resolved_by: data.resolved_by.clone(),
        }
    } else {
        StatusSync::Unchanged
    };

    let mut messages = Vec::new();
    let mut removed = Vec::new();
    for message in thread.messages.iter().filter(|m| m.remote.is_some()) {
        let id = &message.remote.as_ref().expect("posted").id;
        match data.comments.iter().find(|c| c.id == *id) {
            None => removed.push(message.id),
            Some(comment) => {
                let sync = message_sync(message, comment);
                if sync != MessageSync::Unchanged {
                    messages.push((message.id, sync));
                }
            }
        }
    }
    let known = |c: &PlatformComment| {
        thread
            .messages
            .iter()
            .any(|m| m.remote.as_ref().is_some_and(|r| r.id == c.id))
    };
    ThreadSync {
        thread: thread.id,
        status,
        added: data.comments.iter().filter(|c| !known(c)).cloned().collect(),
        removed,
        messages,
    }
}

fn message_sync(message: &Message, comment: &PlatformComment) -> MessageSync {
    let remote = message.remote.as_ref().expect("posted");
    let local_changed = message.body != remote.synced;
    let remote_changed = comment.body != remote.synced;
    match (local_changed, remote_changed) {
        (false, false) => MessageSync::Unchanged,
        (false, true) => MessageSync::TakeRemote {
            body: comment.body.clone(),
            edited_at: comment.edited_at,
        },
        (true, false) => MessageSync::PushLocal,
        (true, true) if message.body == comment.body => MessageSync::Agreed {
            body: comment.body.clone(),
        },
        (true, true) => MessageSync::Conflict {
            remote_body: comment.body.clone(),
            remote_at: comment.edited_at,
        },
    }
}

#[cfg(test)]
mod tests {
    use vet_core::content::RepoPath;
    use vet_core::review::{PlatformThread, RemoteComment, Side};

    use super::*;
    use crate::platform::discussion::{DiffSide, PullRequestState};

    fn comment(id: &str, body: &str) -> PlatformComment {
        PlatformComment {
            id: id.to_owned(),
            author: "lena".to_owned(),
            avatar: None,
            body: body.to_owned(),
            created_at: 100,
            edited_at: 100,
            commit: None,
            mine: false,
        }
    }

    fn remote_thread(id: &str, resolved: bool, comments: Vec<PlatformComment>) -> PlatformThreadData {
        PlatformThreadData {
            id: id.to_owned(),
            path: "job.rs".to_owned(),
            side: DiffSide::Right,
            lines: Some((12, 12)),
            outdated: false,
            resolved,
            resolved_by: resolved.then(|| "lena".to_owned()),
            comments,
        }
    }

    fn discussion(threads: Vec<PlatformThreadData>, complete: bool) -> Discussion {
        Discussion {
            title: "Add idempotency keys".to_owned(),
            url: "https://github.com/acme/app/pull/482".to_owned(),
            draft: false,
            source_branch: "keys".to_owned(),
            target_branch: "main".to_owned(),
            state: PullRequestState::Open,
            author: "lena".to_owned(),
            author_name: "lena".to_owned(),
            author_avatar: None,
            reviewers: Vec::new(),
            viewer_verdict: None,
            labels: Vec::new(),
            ci: None,
            conversation: Vec::new(),
            threads,
            complete,
        }
    }

    fn posted(id: u64, remote: &str, messages: Vec<Message>) -> Thread {
        Thread {
            id: ThreadId(id),
            target: CommentTarget::File {
                side: Side::Target,
                path: RepoPath::from("job.rs"),
            },
            messages,
            status: ThreadStatus::Open,
            platform: Some(PlatformThread {
                remote_id: Some(remote.to_owned()),
                resolvable: true,
                resolved_by: None,
                status_pending: false,
            }),
        }
    }

    /// A comment of the viewer's that the platform has; `synced` is what it said at the last sync.
    fn message(id: u64, remote: &str, synced: &str, body: &str) -> Message {
        Message {
            id: MessageId(id),
            mine: true,
            remote: Some(RemoteComment {
                id: remote.to_owned(),
                synced: synced.to_owned(),
                conflict: None,
            }),
            ..Message::written("You".to_owned(), body.to_owned(), 50)
        }
    }

    fn draft(id: u64, body: &str) -> Message {
        Message {
            id: MessageId(id),
            ..Message::written("You".to_owned(), body.to_owned(), 60)
        }
    }

    /// Reconciles as for GitHub, where a comment on the whole pull request is a flat one.
    fn reconcile(local: &[Thread], remote: &Discussion) -> SyncPlan {
        super::reconcile(local, remote, ChangesetThreads::Flat)
    }

    fn only(plan: SyncPlan) -> ThreadSync {
        assert!(plan.added.is_empty() && plan.removed.is_empty(), "{plan:?}");
        let [sync] = <[ThreadSync; 1]>::try_from(plan.threads).expect("one thread");
        sync
    }

    #[test]
    fn a_thread_the_platform_has_and_vet_does_not_is_added_and_one_that_agrees_changes_nothing() {
        let same = posted(1, "T1", vec![message(1, "C1", "a", "a")]);
        let remote = discussion(
            vec![
                remote_thread("T1", false, vec![comment("C1", "a")]),
                remote_thread("T2", false, vec![comment("C2", "b")]),
            ],
            true,
        );
        let plan = reconcile(&[same], &remote);
        assert_eq!(plan.added.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["T2"]);
        assert_eq!(
            plan.threads,
            [ThreadSync {
                thread: ThreadId(1),
                status: StatusSync::Unchanged,
                added: vec![],
                removed: vec![],
                messages: vec![],
            }]
        );
        assert!(plan.removed.is_empty());
    }

    #[test]
    fn the_platforms_status_is_taken_unless_the_viewer_changed_it_since_the_last_sync() {
        let mut thread = posted(1, "T1", vec![message(1, "C1", "a", "a")]);
        let resolved = discussion(vec![remote_thread("T1", true, vec![comment("C1", "a")])], true);
        assert_eq!(
            only(reconcile(std::slice::from_ref(&thread), &resolved)).status,
            StatusSync::Remote {
                status: ThreadStatus::Resolved,
                resolved_by: Some("lena".to_owned())
            }
        );

        // The viewer reopened it meanwhile, which wins and is sent.
        thread.status = ThreadStatus::Open;
        thread.platform.as_mut().unwrap().status_pending = true;
        assert_eq!(
            only(reconcile(std::slice::from_ref(&thread), &resolved)).status,
            StatusSync::PushLocal { resolved: false }
        );

        // The platform already has what the viewer chose.
        thread.status = ThreadStatus::Resolved;
        assert_eq!(
            only(reconcile(std::slice::from_ref(&thread), &resolved)).status,
            StatusSync::Settled {
                resolved_by: Some("lena".to_owned())
            }
        );
    }

    #[test]
    fn a_flat_comment_has_no_status_to_sync() {
        let mut thread = posted(1, "T1", vec![message(1, "C1", "a", "a")]);
        thread.platform.as_mut().unwrap().resolvable = false;
        let remote = discussion(vec![remote_thread("T1", true, vec![comment("C1", "a")])], true);
        assert_eq!(only(reconcile(&[thread], &remote)).status, StatusSync::Unchanged);
    }

    #[test]
    fn an_edit_on_one_side_is_carried_to_the_other_and_one_on_both_sides_is_a_conflict() {
        let sync_of = |synced: &str, local: &str, remote: &str| {
            let thread = posted(1, "T1", vec![message(1, "C1", synced, local)]);
            let mut edited = comment("C1", remote);
            edited.edited_at = 222;
            let plan = reconcile(
                &[thread],
                &discussion(vec![remote_thread("T1", false, vec![edited])], true),
            );
            only(plan).messages
        };
        assert_eq!(sync_of("old", "old", "old"), []);
        assert_eq!(
            sync_of("old", "old", "new"),
            [(
                MessageId(1),
                MessageSync::TakeRemote {
                    body: "new".to_owned(),
                    edited_at: 222
                }
            )]
        );
        assert_eq!(sync_of("old", "mine", "old"), [(MessageId(1), MessageSync::PushLocal)]);
        assert_eq!(
            sync_of("old", "mine", "theirs"),
            [(
                MessageId(1),
                MessageSync::Conflict {
                    remote_body: "theirs".to_owned(),
                    remote_at: 222
                }
            )]
        );
        assert_eq!(
            sync_of("old", "same", "same"),
            [(
                MessageId(1),
                MessageSync::Agreed {
                    body: "same".to_owned()
                }
            )]
        );
    }

    #[test]
    fn replies_arrive_after_what_vet_knows_and_a_comment_gone_from_the_platform_goes_from_vet() {
        let thread = posted(
            1,
            "T1",
            vec![
                message(1, "C1", "a", "a"),
                message(2, "C2", "b", "b"),
                draft(3, "my draft"),
            ],
        );
        let remote = discussion(
            vec![remote_thread("T1", false, vec![comment("C1", "a"), comment("C3", "c")])],
            true,
        );
        let sync = only(reconcile(&[thread], &remote));
        assert_eq!(sync.added, [comment("C3", "c")]);
        assert_eq!(
            sync.removed,
            [MessageId(2)],
            "the draft has no remote comment to be gone"
        );
    }

    #[test]
    fn a_thread_deleted_on_the_platform_goes_with_its_drafts_named_for_the_notice() {
        let thread = posted(
            4,
            "T9",
            vec![
                message(1, "C1", "a", "a"),
                draft(2, "Then use 48 h."),
                draft(3, "Or 36."),
            ],
        );
        let kept = posted(5, "T1", vec![message(4, "C4", "x", "x")]);
        let local_only = Thread {
            platform: Some(PlatformThread::draft(true)),
            ..posted(6, "unused", vec![draft(5, "not posted")])
        };
        let remote = discussion(vec![remote_thread("T1", false, vec![comment("C4", "x")])], true);

        let plan = reconcile(&[thread, kept, local_only], &remote);

        assert_eq!(
            plan.removed,
            [RemovedThread {
                thread: ThreadId(4),
                drafts: vec!["Then use 48 h.".to_owned(), "Or 36.".to_owned()]
            }]
        );
        assert_eq!(plan.threads.len(), 1);
    }

    #[test]
    fn a_comment_on_the_whole_pull_request_is_not_among_the_review_threads_and_stays() {
        let flat = Thread {
            target: CommentTarget::Changeset,
            ..posted(1, "IC1", vec![message(1, "IC1", "a", "a")])
        };
        assert_eq!(reconcile(&[flat], &discussion(vec![], true)), SyncPlan::default());
    }

    #[test]
    fn a_thread_on_the_whole_pull_request_gone_from_a_platform_that_lists_it_among_the_threads_goes() {
        let whole = Thread {
            target: CommentTarget::Changeset,
            ..posted(1, "7/12", vec![message(1, "7/12/1", "a", "a")])
        };
        let kept = Thread {
            target: CommentTarget::Changeset,
            ..posted(2, "7/13", vec![message(2, "7/13/1", "b", "b")])
        };
        let remote = discussion(vec![remote_thread("7/13", false, vec![comment("7/13/1", "b")])], true);

        let plan = super::reconcile(&[whole, kept], &remote, ChangesetThreads::Resolvable);

        assert_eq!(
            plan.removed,
            [RemovedThread {
                thread: ThreadId(1),
                drafts: vec![]
            }]
        );
        assert_eq!(plan.threads.len(), 1);
    }

    #[test]
    fn a_thread_whose_first_comment_is_gone_keeps_the_others() {
        let thread = posted(
            1,
            "7/12",
            vec![message(1, "7/12/1", "a", "a"), message(2, "7/12/2", "b", "b")],
        );
        let remote = discussion(vec![remote_thread("7/12", false, vec![comment("7/12/2", "b")])], true);

        let plan = super::reconcile(&[thread], &remote, ChangesetThreads::Resolvable);

        assert!(plan.removed.is_empty() && plan.added.is_empty(), "{plan:?}");
        assert_eq!(plan.threads[0].removed, [MessageId(1)]);
        assert!(plan.threads[0].added.is_empty());
    }

    #[test]
    fn a_partial_listing_removes_nothing() {
        let thread = posted(1, "T9", vec![message(1, "C1", "a", "a")]);
        let remote = discussion(vec![], false);
        assert_eq!(reconcile(&[thread], &remote), SyncPlan::default());
    }
}
