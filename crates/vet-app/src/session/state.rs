//! The review state a session keeps behind its lock, and the lookups every
//! part of the session makes in it.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use vet_core::content::{
    BlobId, FileDiff, NonTextChange, NonTextKind, RepoPath, Snapshot, SnapshotDiff, SnapshotId, SnapshotStatus,
};
use vet_core::filter::ReviewFilter;
use vet_core::resolve::Resolution;
use vet_core::review::{Anchor, CommentTarget, ReviewedSpan, Thread, ThreadId};

use super::views::{ItemView, Lifecycle, Outcome};
use super::{Result, SessionError};
use crate::changeset::{BaseChoice, ChangesetId, CommitId};
use crate::ports::Degraded;
use crate::review::{NonTextAck, RecordedSpan, ReviewId, SpanId};

/// What the current revision's diff needs, computed on first use.
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
    pub(super) degraded: Option<Degraded>,
    pub(super) changes: Option<Changes>,
}

impl State {
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
