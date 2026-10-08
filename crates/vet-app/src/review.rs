//! Review (§5): the state a viewer builds up on a changeset, and the threads
//! that belong to the changeset itself. Remapping runs per changed file,
//! triggered when either the base or the target blob id changes (the base case
//! is `SeenAgainstOldBase`).

use vet_core::completion::Completion;
use vet_core::content::{FileEntry, NonTextKind, RepoPath};
use vet_core::review::{ReviewedSpan, Thread, ThreadStatus};

use crate::changeset::{BaseChoice, ChangesetId, CommitId};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ReviewId(pub u64);

/// A non-text change acknowledged like a hunk, keyed by its kind and the
/// before and after entries (§5.2). `None` for an added or deleted path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NonTextAck {
    pub kind: NonTextKind,
    pub before: Option<(RepoPath, FileEntry)>,
    pub after: Option<(RepoPath, FileEntry)>,
}

/// Where a span was marked: in the changeset as a whole, or in one commit's own diff. It decides
/// how the span remaps onto another view (design §2.2).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SpanOrigin {
    Squash,
    Commit(CommitId),
}

/// A span as recorded, in the snapshots it was marked in; never remapped in place.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecordedSpan {
    pub id: SpanId,
    pub span: ReviewedSpan,
    pub origin: SpanOrigin,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SpanId(pub u64);

/// How far a review is: the share of the changeset's lines and non-text items that are seen,
/// counted against the review's filter, kept with the review so that a list of changesets shows
/// it without remapping every review's marks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReviewSummary {
    /// Changed lines passing the filter plus non-text items.
    pub total: u32,
    /// Of those, the lines covered by a `Seen` span plus the acknowledged items.
    pub seen: u32,
    /// Of the lines, those covered only by spans seen against an old base.
    pub old_base: u32,
}

impl ReviewSummary {
    pub fn of(completion: &Completion) -> Self {
        Self {
            total: completion.lines + completion.items,
            seen: completion.seen_lines + completion.acknowledged_items,
            old_base: completion.old_base_lines,
        }
    }

    /// Share of the changeset that is seen, in percent; 0 with nothing to review.
    pub fn seen_percent(&self) -> f32 {
        self.percent(self.seen)
    }

    /// Share of the changeset that is seen against an old base only, in percent.
    pub fn old_base_percent(&self) -> f32 {
        self.percent(self.old_base)
    }

    fn percent(&self, part: u32) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            part as f32 / self.total as f32 * 100.0
        }
    }
}

/// The marks of one pass over a changeset, from its first review action to
/// finishing it. A changeset has at most one open review.
pub struct Review {
    pub id: ReviewId,
    pub changeset: ChangesetId,
    /// How the changeset's base was chosen while this review was open.
    pub base: BaseChoice,
    pub spans: Vec<RecordedSpan>,
    pub acks: Vec<NonTextAck>,
    /// The number of the version the viewer last marked something in.
    pub last_reviewed: Option<u32>,
    /// Unix seconds.
    pub opened_at: i64,
    /// Unix seconds; `None` while open.
    pub closed_at: Option<i64>,
    /// Whether the review was complete when it was finished; `None` while open.
    pub complete: Option<bool>,
    /// How far the review was when it was last measured; `None` if it never was.
    pub summary: Option<ReviewSummary>,
}

/// A thread of a changeset. A thread outlives the review it was written in; the
/// review is kept to say where it came from.
#[derive(Clone, PartialEq, Debug)]
pub struct ChangesetThread {
    pub thread: Thread,
    /// The review a local thread was opened in.
    pub review: Option<ReviewId>,
}

/// The threads to show: every open one, every one the platform has, and every one opened in the
/// review that is open. A resolved thread of a finished review is part of that review.
pub fn visible_threads(all: Vec<ChangesetThread>, open: Option<ReviewId>) -> Vec<Thread> {
    all.into_iter()
        .filter(|t| {
            t.thread.status == ThreadStatus::Open || t.thread.is_posted() || (open.is_some() && t.review == open)
        })
        .map(|t| t.thread)
        .collect()
}

#[cfg(test)]
mod tests {
    use vet_core::review::{CommentTarget, PlatformThread, ThreadId};

    use super::*;

    fn thread(id: u64, status: ThreadStatus, review: Option<u64>) -> ChangesetThread {
        ChangesetThread {
            thread: Thread {
                id: ThreadId(id),
                target: CommentTarget::Changeset,
                messages: Vec::new(),
                status,
                platform: None,
            },
            review: review.map(ReviewId),
        }
    }

    #[test]
    fn a_resolved_thread_collapses_into_its_finished_review_and_an_open_one_stays() {
        let mut on_platform = thread(6, ThreadStatus::Resolved, None);
        on_platform.thread.platform = Some(PlatformThread {
            remote_id: Some("T1".to_owned()),
            ..PlatformThread::draft(true)
        });
        let all = vec![
            thread(1, ThreadStatus::Open, Some(1)),
            thread(2, ThreadStatus::Resolved, Some(1)),
            thread(3, ThreadStatus::Open, None),
            thread(4, ThreadStatus::Resolved, Some(2)),
            thread(5, ThreadStatus::Resolved, None),
            on_platform,
        ];
        let ids = |open| {
            visible_threads(all.clone(), open)
                .iter()
                .map(|t| t.id.0)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(None),
            [1, 3, 6],
            "a resolved thread the platform has is shown without a review"
        );
        assert_eq!(ids(Some(ReviewId(2))), [1, 3, 4, 6]);
    }
}
