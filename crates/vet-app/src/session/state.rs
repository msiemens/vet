//! The review state a session keeps behind its lock, and the lookups every
//! part of the session makes in it.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use vet_core::content::{
    BlobId, FileDiff, NonTextChange, NonTextKind, RepoPath, Snapshot, SnapshotDiff, SnapshotId, SnapshotStatus,
};
use vet_core::filter::ReviewFilter;
use vet_core::interdiff::InterdiffHunk;
use vet_core::resolve::Resolution;
use vet_core::review::{Anchor, CommentTarget, ReviewedSpan, Thread, ThreadId};

use super::views::{ItemView, Lifecycle, Outcome};
use super::{Result, SessionError};
use crate::changeset::{BaseChoice, ChangesetId, CommitId, Source, Version};
use crate::platform::Discussion;
use crate::ports::CommitDetail;
use crate::review::{NonTextAck, RecordedSpan, ReviewId, SpanId};

/// What a request about the changeset's files reads: the changeset itself, the interdiff of
/// two of the review's versions, or one commit's own diff.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum View {
    #[default]
    Changeset,
    /// Version `from` against the later version `to`.
    Interdiff { from: u32, to: u32 },
    /// The `index`th commit of the changeset, oldest first: what it changed against its parent
    /// (for a merge, against the automatic merge of its parents).
    Commit { index: u32 },
}

/// An interdiff computed at the current revision.
pub(super) struct Interdiffed {
    /// A's target rebased onto B's base against B's target, shaped like the changeset's changes
    /// with no review state of its own (nothing remapped, no threads resolved).
    pub(super) changes: Changes,
    /// The hunks of each text file in `changes.diff`, with what they cover in B's changeset.
    pub(super) hunks: BTreeMap<RepoPath, Vec<InterdiffHunk>>,
    pub(super) from: Version,
    pub(super) to: Version,
    /// `to` is the latest version, the one the review state is remapped onto.
    pub(super) markable: bool,
}

/// One commit of the changeset with its own diff.
pub(super) struct CommitUnit {
    pub(super) detail: CommitDetail,
    pub(super) kind: UnitKind,
    /// What the commit is compared with: its parent, or the automatic merge of a merge's parents.
    pub(super) old: Snapshot,
    pub(super) new: Snapshot,
    pub(super) changes: Changes,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum UnitKind {
    Commit,
    /// `remerged`: compared with the automatic merge of its parents. A merge of more than two
    /// parents, or one whose automatic merge could not be made, is compared with its first
    /// parent instead.
    Merge {
        remerged: bool,
    },
}

impl CommitUnit {
    /// A merge whose automatic merge it equals: nothing was decided in it.
    pub(super) fn clean_merge(&self) -> bool {
        self.kind == UnitKind::Merge { remerged: true }
            && self.changes.diff.files.is_empty()
            && self.changes.diff.non_text.is_empty()
    }
}

/// What the current revision's diff needs, computed on first use.
#[derive(Default)]
pub(super) struct Changes {
    pub(super) diff: SnapshotDiff,
    /// Every path the diff mentions, on either side.
    pub(super) changed: BTreeSet<RepoPath>,
    /// Text of the diffed files and of what recorded spans were marked in.
    pub(super) blobs: BTreeMap<BlobId, Vec<u8>>,
    /// Recorded spans as they are now.
    pub(super) remapped: Vec<(SpanId, Vec<ReviewedSpan>)>,
    pub(super) flat: Vec<ReviewedSpan>,
    /// Range threads resolved from their recorded anchors.
    pub(super) resolutions: HashMap<ThreadId, Resolution>,
    /// Both sides of each `diff.non_text` change.
    pub(super) items: Vec<ItemView>,
    /// The interdiffs asked for at this revision, by the versions they compare.
    pub(super) interdiffs: HashMap<(u32, u32), Interdiffed>,
    /// The commits of the changeset, each with its own diff, once the by-commit lens asked.
    pub(super) commits: Option<Vec<CommitUnit>>,
}

/// A started review.
pub(super) struct OpenReview {
    pub(super) id: ReviewId,
    /// The number of the changeset's latest version.
    pub(super) version: u32,
    /// Unix seconds the latest version was recorded.
    pub(super) updated_at: i64,
    /// The number of the version something was last marked in.
    pub(super) last_reviewed: Option<u32>,
}

pub(super) struct State {
    /// What the changeset on view is: its target comes from here.
    pub(super) source: Source,
    pub(super) revision: u32,
    /// The stored changeset; `None` until a review of it started.
    pub(super) changeset: Option<ChangesetId>,
    /// `None` while browsing.
    pub(super) review: Option<OpenReview>,
    pub(super) filter: ReviewFilter,
    pub(super) base: Snapshot,
    pub(super) base_choice: BaseChoice,
    /// The commit `base` is the tree of.
    pub(super) base_commit: CommitId,
    /// The commit target snapshots are built on: `HEAD`, as of the review's
    /// start while one is open.
    pub(super) head: CommitId,
    pub(super) target: Snapshot,
    pub(super) target_status: SnapshotStatus,
    /// Other snapshots that recorded spans refer to.
    pub(super) known: HashMap<SnapshotId, Snapshot>,
    pub(super) recorded: Vec<RecordedSpan>,
    pub(super) acks: Vec<NonTextAck>,
    pub(super) threads: Vec<Thread>,
    /// What the platform last said of a pull request, beside its threads, and when (unix seconds).
    pub(super) discussion: Option<(i64, Discussion)>,
    pub(super) changes: Option<Changes>,
}

impl State {
    /// The computed changes `view` shows.
    pub(super) fn changes_in(&self, view: View) -> &Changes {
        let changes = self.changes.as_ref().expect("computed");
        match view {
            View::Changeset => changes,
            View::Interdiff { from, to } => &changes.interdiffs[&(from, to)].changes,
            View::Commit { index } => &commits(changes)[index as usize].changes,
        }
    }

    pub(super) fn changes_in_mut(&mut self, view: View) -> &mut Changes {
        let changes = self.changes.as_mut().expect("computed");
        match view {
            View::Changeset => changes,
            View::Interdiff { from, to } => &mut changes.interdiffs.get_mut(&(from, to)).expect("computed").changes,
            View::Commit { index } => &mut changes.commits.as_mut().expect("computed")[index as usize].changes,
        }
    }

    /// Every anchor the recorded spans and the range threads are placed by.
    pub(super) fn anchors(&self) -> impl Iterator<Item = &Anchor> {
        let spans = (self.recorded.iter()).flat_map(|r| r.span.base.iter().chain(&r.span.target));
        let threads = self.threads.iter().filter_map(|t| match &t.target {
            CommentTarget::Range(a) => Some(a),
            _ => None,
        });
        spans.chain(threads)
    }

    /// The snapshot `id` if the session has it in memory.
    pub(super) fn snapshot(&self, id: SnapshotId) -> Option<&Snapshot> {
        if id == self.base.id {
            Some(&self.base)
        } else if id == self.target.id {
            Some(&self.target)
        } else {
            self.known.get(&id)
        }
    }
}

/// The commit units of `changes`, which `ensure_view` computed for a commit view.
pub(super) fn commits(changes: &Changes) -> &[CommitUnit] {
    changes.commits.as_deref().expect("computed")
}

pub(super) fn outcome<T>(st: &State, value: T, started: bool) -> Outcome<T> {
    Outcome {
        value,
        started: started.then(|| lifecycle_of(st)),
    }
}

pub(super) fn lifecycle_of(st: &State) -> Lifecycle {
    match &st.review {
        None => Lifecycle::Browsing,
        Some(r) => Lifecycle::Open {
            version: r.version,
            updated_at: r.updated_at,
        },
    }
}

pub(super) fn check_revision(st: &State, revision: u32) -> Result<()> {
    if revision == st.revision {
        Ok(())
    } else {
        Err(SessionError::Stale { current: st.revision })
    }
}

pub(super) fn ack_of(change: &NonTextChange) -> NonTextAck {
    NonTextAck {
        kind: change.kind,
        before: change
            .before
            .clone()
            .map(|e| (change.from.clone().unwrap_or_else(|| change.path.clone()), e)),
        after: change.after.clone().map(|e| (change.path.clone(), e)),
    }
}

/// The changes of `diff` that are acknowledged.
pub(super) fn acknowledged(st: &State, diff: &SnapshotDiff) -> Vec<NonTextChange> {
    diff.non_text
        .iter()
        .filter(|n| st.acks.contains(&ack_of(n)))
        .cloned()
        .collect()
}

pub(super) fn find_diff<'a>(diff: &'a SnapshotDiff, path: &str) -> Option<&'a FileDiff> {
    diff.files.iter().find(|d| d.path().to_string_lossy() == path)
}

pub(super) fn find_non_text<'a>(diff: &'a SnapshotDiff, kind: NonTextKind, path: &str) -> Option<&'a NonTextChange> {
    diff.non_text
        .iter()
        .find(|n| n.kind == kind && n.path.to_string_lossy() == path)
}

/// The non-text changes of the file at `path` that the filter shows.
pub(super) fn shown_items(diff: &SnapshotDiff, filter: &ReviewFilter, path: &str) -> Vec<NonTextChange> {
    let compiled = filter.compile();
    (diff.non_text.iter())
        .filter(|n| n.path.to_string_lossy() == path && compiled.shows(n))
        .cloned()
        .collect()
}
