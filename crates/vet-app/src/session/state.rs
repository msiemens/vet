//! The review state a session keeps behind its lock, and the lookups every
//! part of the session makes in it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use vet_core::completion::{Completion, Coverage};
use vet_core::content::{
    BlobId, FileDiff, NonTextChange, NonTextKind, RepoPath, Snapshot, SnapshotDiff, SnapshotId, SnapshotStatus,
};
use vet_core::filter::ReviewFilter;
use vet_core::resolve::Resolution;
use vet_core::review::{Anchor, CommentTarget, ReviewedSpan, Thread, ThreadId};

use super::derived::{CommitUnit, Compared, Diffed};
use super::views::{Lifecycle, Outcome};
use super::{Result, SessionError};
use crate::changeset::{BaseChoice, ChangesetId, CommitId, Source, Version};
use crate::platform::Discussion;
use crate::review::{NonTextAck, RecordedSpan, ReviewId, ReviewSummary, SpanId};

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

/// What a request about one view reads beside the state: for an interdiff, the interdiff itself,
/// which has no review state of its own and so is not kept in the state.
pub(super) enum Shown {
    Changeset,
    Commit { index: u32 },
    Interdiff(Box<Interdiffed>),
}

impl Shown {
    pub(super) fn view(&self) -> View {
        match self {
            Self::Changeset => View::Changeset,
            Self::Commit { index } => View::Commit { index: *index },
            Self::Interdiff(id) => View::Interdiff {
                from: id.from.n,
                to: id.to.n,
            },
        }
    }

    /// The diff the view shows.
    pub(super) fn diffed<'a>(&'a self, st: &'a State) -> &'a Diffed {
        match self {
            Self::Interdiff(id) => &id.compared.diffed,
            _ => st.diffed_in(self.view()),
        }
    }

    /// Whether marks can be made in the view.
    pub(super) fn markable(&self) -> bool {
        match self {
            Self::Changeset | Self::Commit { .. } => true,
            Self::Interdiff(id) => id.markable,
        }
    }
}

/// The interdiff of two of the review's versions.
pub(super) struct Interdiffed {
    pub(super) compared: Arc<Compared>,
    pub(super) from: Version,
    pub(super) to: Version,
    /// `to` is the latest version, the one the review state is remapped onto.
    pub(super) markable: bool,
}

/// The review state remapped onto one diff.
#[derive(Clone, Default)]
#[cfg_attr(test, derive(PartialEq, Debug))]
pub(super) struct Remapped {
    /// Each recorded span as it is now. A span's remap depends on the span and the diff only, so
    /// a mark remaps the spans it records and leaves the others as they are.
    spans: HashMap<SpanId, Arc<Vec<ReviewedSpan>>>,
    /// The spans with a piece in each file.
    files: HashMap<RepoPath, HashSet<SpanId>>,
    /// Range threads resolved from their recorded anchors; only in the changeset.
    pub(super) resolutions: HashMap<ThreadId, Resolution>,
    /// The completion of the whole diff under the state's filter and acknowledgements.
    pub(super) completion: Completion,
}

impl Remapped {
    /// The remapped spans and resolved threads, with the completion still to count.
    pub(super) fn of(spans: Vec<(SpanId, Vec<ReviewedSpan>)>, resolutions: HashMap<ThreadId, Resolution>) -> Self {
        let mut remapped = Self {
            resolutions,
            ..Self::default()
        };
        for (id, pieces) in spans {
            remapped.insert(id, pieces);
        }
        remapped
    }

    /// The recorded span `id` as it is now; empty if nothing of it is left.
    pub(super) fn span(&self, id: SpanId) -> &[ReviewedSpan] {
        self.spans.get(&id).map_or(&[], |pieces| pieces.as_slice())
    }

    pub(super) fn insert(&mut self, id: SpanId, pieces: Vec<ReviewedSpan>) {
        for file in files_of(&pieces) {
            self.files.entry(file.clone()).or_default().insert(id);
        }
        self.spans.insert(id, Arc::new(pieces));
    }

    pub(super) fn remove(&mut self, id: SpanId) {
        let Some(pieces) = self.spans.remove(&id) else { return };
        for file in files_of(&pieces) {
            if let Some(ids) = self.files.get_mut(file) {
                ids.remove(&id);
                if ids.is_empty() {
                    self.files.remove(file);
                }
            }
        }
    }

    /// The files the pieces of the span `id` are in.
    pub(super) fn files(&self, id: SpanId) -> impl Iterator<Item = &RepoPath> {
        files_of(self.span(id))
    }

    /// The coverage of every span.
    pub(super) fn coverage(&self) -> Coverage<'_> {
        Coverage::new(self.spans.values().flat_map(|pieces| pieces.iter()))
    }

    /// A coverage that answers for `files` as [`Remapped::coverage`] does, made of the spans with a
    /// piece in them only.
    pub(super) fn coverage_of<'f>(&self, files: impl IntoIterator<Item = &'f RepoPath>) -> Coverage<'_> {
        let ids: HashSet<SpanId> = files
            .into_iter()
            .filter_map(|f| self.files.get(f))
            .flatten()
            .copied()
            .collect();
        Coverage::new(ids.into_iter().flat_map(|id| self.span(id)))
    }
}

/// The files the anchors of `pieces` are in.
pub(super) fn files_of(pieces: &[ReviewedSpan]) -> impl Iterator<Item = &RepoPath> {
    pieces
        .iter()
        .flat_map(|p| p.base.iter().chain(&p.target))
        .map(|a| &a.file)
}

/// The current revision's diff, held by the engine, and the review state remapped onto it.
#[derive(Clone)]
pub(super) struct Changes {
    pub(super) diffed: Arc<Diffed>,
    pub(super) remapped: Remapped,
    /// Texts of what recorded spans and threads are anchored in that the diffs do not have.
    pub(super) anchored: Arc<BTreeMap<BlobId, Vec<u8>>>,
    /// The commits of the changeset, once the by-commit lens asked.
    pub(super) commits: Option<Commits>,
    /// How often a recorded span anchored in each file was remapped, onto any view.
    #[cfg(test)]
    pub(super) remaps: HashMap<RepoPath, usize>,
}

impl Changes {
    pub(super) fn new(diffed: Arc<Diffed>) -> Self {
        Self {
            diffed,
            remapped: Remapped::default(),
            anchored: Arc::default(),
            commits: None,
            #[cfg(test)]
            remaps: HashMap::new(),
        }
    }
}

/// The commits of the changeset, held by the engine, and the review state remapped onto each.
#[derive(Clone)]
pub(super) struct Commits {
    pub(super) units: Arc<Vec<CommitUnit>>,
    /// One per unit.
    pub(super) remapped: Vec<Remapped>,
}

/// A started review.
#[derive(Clone)]
pub(super) struct OpenReview {
    pub(super) id: ReviewId,
    /// The number of the changeset's latest version.
    pub(super) version: u32,
    /// Unix seconds the latest version was recorded.
    pub(super) updated_at: i64,
    /// The number of the version something was last marked in.
    pub(super) last_reviewed: Option<u32>,
    /// How far the review is, as last stored.
    pub(super) summary: Option<ReviewSummary>,
}

#[derive(Clone)]
pub(super) struct State {
    /// What the changeset on view is: its target comes from here.
    pub(super) source: Source,
    pub(super) revision: u32,
    /// The stored changeset; `None` until a review of it started.
    pub(super) changeset: Option<ChangesetId>,
    /// `None` while browsing.
    pub(super) review: Option<OpenReview>,
    pub(super) filter: ReviewFilter,
    /// The snapshots are shared: a command copies the state it changes, and a tree of 100k
    /// entries is not copied with it.
    pub(super) base: Arc<Snapshot>,
    pub(super) base_choice: BaseChoice,
    /// The commit `base` is the tree of.
    pub(super) base_commit: CommitId,
    /// The commit target snapshots are built on: `HEAD`, as of the review's
    /// start while one is open.
    pub(super) head: CommitId,
    pub(super) target: Arc<Snapshot>,
    pub(super) target_status: SnapshotStatus,
    /// Other snapshots that recorded spans refer to.
    pub(super) known: HashMap<SnapshotId, Arc<Snapshot>>,
    pub(super) recorded: Vec<Arc<RecordedSpan>>,
    pub(super) acks: Vec<NonTextAck>,
    pub(super) threads: Vec<Thread>,
    /// What the platform last said of a pull request, beside its threads, and when (unix seconds).
    pub(super) discussion: Option<(i64, Discussion)>,
    pub(super) changes: Option<Changes>,
}

impl State {
    /// The changes of the current revision, which the caller made sure are computed.
    pub(super) fn changes(&self) -> &Changes {
        self.changes.as_ref().expect("computed")
    }

    /// The commit units, which the caller made sure are computed.
    pub(super) fn units(&self) -> &[CommitUnit] {
        &self.changes().commits.as_ref().expect("computed").units
    }

    /// The diff of the changeset or of one of its commits.
    pub(super) fn diffed_in(&self, view: View) -> &Diffed {
        match view {
            View::Commit { index } => &self.units()[index as usize].diffed,
            View::Changeset => &self.changes().diffed,
            View::Interdiff { .. } => unreachable!("an interdiff is not kept in the state"),
        }
    }

    /// The review state remapped onto the changeset or onto one of its commits.
    pub(super) fn remapped_in(&self, view: View) -> &Remapped {
        let changes = self.changes();
        match view {
            View::Commit { index } => &changes.commits.as_ref().expect("computed").remapped[index as usize],
            View::Changeset => &changes.remapped,
            View::Interdiff { .. } => unreachable!("an interdiff has no review state of its own"),
        }
    }

    /// The views the review state is remapped onto: the changeset and, once computed, each commit.
    pub(super) fn remapped_views(&self) -> Vec<View> {
        let commits = (self.changes.as_ref())
            .and_then(|c| c.commits.as_ref())
            .map_or(0, |c| c.units.len() as u32);
        std::iter::once(View::Changeset)
            .chain((0..commits).map(|index| View::Commit { index }))
            .collect()
    }

    pub(super) fn remapped_in_mut(&mut self, view: View) -> &mut Remapped {
        let changes = self.changes.as_mut().expect("computed");
        match view {
            View::Commit { index } => &mut changes.commits.as_mut().expect("computed").remapped[index as usize],
            View::Changeset => &mut changes.remapped,
            View::Interdiff { .. } => unreachable!("an interdiff has no review state of its own"),
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
            self.known.get(&id).map(|s| &**s)
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

/// The changes of `changes` that are acknowledged.
pub(super) fn acknowledged<'a>(st: &State, changes: impl IntoIterator<Item = &'a NonTextChange>) -> Vec<NonTextChange> {
    changes
        .into_iter()
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
