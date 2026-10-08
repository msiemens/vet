//! Computing a revision's changes, and remapping recorded spans and thread
//! anchors onto them.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use vet_core::completion::Completion;
use vet_core::content::{
    BlobId, EntryKind, FileDiff, NonTextChange, RenameMap, RepoPath, Snapshot, SnapshotDiff, SnapshotId,
};
use vet_core::prepared::PreparedFile;
use vet_core::remap::{RemapMode, remap_span};
use vet_core::resolve::{Resolution, resolve_anchor};
use vet_core::review::{Anchor, CommentTarget, FileRemapInput, ReviewedSpan, Side, Thread, ThreadId};

use super::blobs::Texts;
use super::derived::{CommitUnit, CommitsBetween, Diffed};
use super::hold::Held;
use super::interdiff::Interdiffing;
use super::manifest::changeset_completion;
use super::state::{Changes, Interdiffed, Remapped, Shown, State, View, acknowledged, files_of};
use super::{LiveSession, Result, SessionError};
use crate::ports::{GitRepo, LiveStore};
use crate::review::{RecordedSpan, SpanId, SpanOrigin};

/// Indices into the prepared files of a recorded span's base and target side.
pub(super) type Slot = (Option<usize>, Option<usize>);

pub(super) struct Prepared {
    pub(super) inputs: Vec<FileRemapInput>,
    /// One per recorded span.
    pub(super) spans: Vec<Slot>,
    /// One per thread; `None` unless it is a range thread.
    pub(super) threads: Vec<Option<usize>>,
}

/// What spans and anchors are remapped onto: the two snapshots of a view's unit, and what was
/// marked in that very unit.
pub(super) struct Frame<'a> {
    pub(super) base: &'a Snapshot,
    pub(super) target: &'a Snapshot,
    /// The origin of the spans marked in this unit: they remap as the same unit, the others as
    /// projections.
    pub(super) origin: SpanOrigin,
    pub(super) renames: RenameMap,
}

impl Frame<'_> {
    fn mode_of(&self, span: &RecordedSpan) -> RemapMode {
        if span.origin == self.origin {
            RemapMode::SameUnit
        } else {
            RemapMode::Projection
        }
    }
}

/// The frame of `view`, a view of the changeset or of one commit, whose changes `ensure_view` computed.
/// `renames` are those of the view's own diff.
pub(super) fn frame_of(st: &State, view: View, renames: RenameMap) -> Frame<'_> {
    match view {
        View::Commit { index } => commit_frame(st, index, renames),
        _ => Frame {
            base: &st.base,
            target: &st.target,
            origin: SpanOrigin::Squash,
            renames,
        },
    }
}

/// The frame of the commit `index`: its own two snapshots, the marks made in it being the ones of its own origin.
fn commit_frame(st: &State, index: u32, renames: RenameMap) -> Frame<'_> {
    let unit = &st.units()[index as usize];
    Frame {
        base: &unit.old,
        target: &unit.new,
        origin: SpanOrigin::Commit(unit.detail.info.id.clone()),
        renames,
    }
}

/// The file a recorded anchor of a side is remapped into in `frame`: the base side stays at its
/// path; the target side follows a rename from `HEAD` if its file is gone.
pub(super) fn landing(frame: &Frame<'_>, is_base: bool, anchor: &Anchor) -> Option<RepoPath> {
    if is_base || frame.target.entries.contains_key(&anchor.file) {
        Some(anchor.file.clone())
    } else {
        frame.renames.0.get(&anchor.file).cloned()
    }
}

/// What of the review state a remap reads.
#[derive(Clone, Copy)]
enum Remapping<'a> {
    /// Every recorded span and, in the changeset, every thread.
    All,
    /// The threads only.
    Threads,
    /// These recorded spans only.
    Spans(&'a [SpanId]),
}

/// Counts the completion of `view` from all of its review state.
pub(super) fn recount(st: &mut State, view: View) {
    let diff = &st.diffed_in(view).diff;
    let acked = acknowledged(st, &diff.non_text);
    let completion = changeset_completion(&st.remapped_in(view).coverage(), &st.filter, diff, &acked);
    st.remapped_in_mut(view).completion = completion;
}

/// The completion of what in `view` lies in `files`: the text diffs with a side there and the
/// non-text changes of them. A mark or an acknowledgement in `files` changes this part of the
/// view's completion and leaves the rest as it is.
pub(super) fn part_completion(st: &State, view: View, files: &HashSet<RepoPath>) -> Completion {
    let diff = &st.diffed_in(view).diff;
    let within = |p: Option<&RepoPath>| p.is_some_and(|p| files.contains(p));
    let diffs: Vec<&FileDiff> = (diff.files.iter())
        .filter(|d| within(d.old_path.as_ref()) || within(d.new_path.as_ref()))
        .collect();
    let items: Vec<NonTextChange> = (diff.non_text.iter())
        .filter(|n| within(Some(&n.path)) || within(n.from.as_ref()))
        .cloned()
        .collect();
    let coverage = (st.remapped_in(view)).coverage_of(diffs.iter().flat_map(|d| d.old_path.iter().chain(&d.new_path)));
    coverage.completion(&st.filter, diffs, &items, &acknowledged(st, &items))
}

/// What the state lacks to show a view, computed without the token.
enum Missing {
    Diff {
        base: Arc<Snapshot>,
        target: Arc<Snapshot>,
    },
    /// The commits between the keyed commits; none for a changeset that has no commits of its own.
    Commits(Option<CommitsBetween>),
    Interdiff(Box<Interdiffing>),
}

/// A value computed for what was [`Missing`].
enum Derivation {
    Diff(Arc<Diffed>),
    Commits(Arc<Vec<CommitUnit>>),
    Interdiff(Box<Interdiffed>),
}

impl<G, S> LiveSession<G, S>
where
    G: GitRepo + Send + Sync + 'static,
    S: LiveStore + Send + Sync + 'static,
{
    /// The published state with what `view` shows computed, for a request that only reads. What is
    /// derived (a diff, an interdiff, the commits) is awaited without any lock, so that requests of
    /// the same value share its computation. The state keeps the diff and the review state remapped
    /// onto it: when it has not, the request becomes a command for as long as it takes to install
    /// them, which the commands that move the revision see to beforehand (`settled`).
    pub(super) async fn viewing(&self, view: View) -> Result<(Arc<State>, Shown)> {
        self.prepared(view, matches!(view, View::Commit { .. })).await
    }

    /// Like [`LiveSession::viewing`]; with `commits`, the commits of the changeset are computed too.
    pub(super) async fn prepared(&self, view: View, commits: bool) -> Result<(Arc<State>, Shown)> {
        loop {
            let st = self.published.read();
            let missing = match self.missing(&st, view, commits).await? {
                Ok(shown) => return Ok((st, shown)),
                Err(missing) => missing,
            };
            let derived = self.derive(missing).await;
            let now = self.published.read();
            if now.revision != st.revision {
                continue;
            }
            match derived? {
                Derivation::Interdiff(interdiffed) => return Ok((now, Shown::Interdiff(interdiffed))),
                derived => {
                    let mut held = self.hold().await;
                    if held.revision == st.revision {
                        self.install(&mut held, derived).await?;
                    }
                }
            }
        }
    }

    /// Like [`LiveSession::viewing`], for a command: the token is held on return, so the state
    /// is the one the command changes. The derived values are awaited without it.
    pub(super) async fn writing(&self, view: View) -> Result<(Held<'_>, Shown)> {
        let mut st = self.hold().await;
        loop {
            let missing = match self.missing(&st, view, matches!(view, View::Commit { .. })).await? {
                Ok(shown) => return Ok((st, shown)),
                Err(missing) => missing,
            };
            let revision = st.revision;
            drop(st);
            let derived = self.derive(missing).await;
            st = self.hold().await;
            if st.revision != revision {
                continue;
            }
            match derived? {
                Derivation::Interdiff(interdiffed) => return Ok((st, Shown::Interdiff(interdiffed))),
                derived => self.install(&mut st, derived).await?,
            }
        }
    }

    /// Puts what was derived for the revision into the state, unless it has it.
    async fn install(&self, st: &mut State, derived: Derivation) -> Result<()> {
        match derived {
            Derivation::Diff(diffed) if st.changes.is_none() => self.install_diff(st, diffed).await,
            Derivation::Commits(units) if st.changes.as_ref().is_some_and(|c| c.commits.is_none()) => {
                self.install_commits(st, units).await
            }
            _ => Ok(()),
        }
    }

    /// What `view` shows if the state has it, else what is missing.
    async fn missing(&self, st: &State, view: View, commits: bool) -> Result<std::result::Result<Shown, Missing>> {
        if st.changes.is_none() {
            let (base, target) = (st.base.clone(), st.target.clone());
            return Ok(Err(Missing::Diff { base, target }));
        }
        if commits && st.changes().commits.is_none() {
            return Ok(Err(Missing::Commits(self.commits_key(st))));
        }
        match view {
            View::Changeset => Ok(Ok(Shown::Changeset)),
            View::Commit { index } if (index as usize) < st.units().len() => Ok(Ok(Shown::Commit { index })),
            View::Commit { index } => Err(SessionError::NotFound(format!("commit {index}"))),
            View::Interdiff { from, to } => {
                let asked = self.interdiffing(st, from, to).await?;
                Ok(match self.derived.interdiffs.peek(&asked.key) {
                    Some(compared) => Ok(Shown::Interdiff(Box::new(asked.with(compared)))),
                    None => Err(Missing::Interdiff(Box::new(asked))),
                })
            }
        }
    }

    async fn derive(&self, missing: Missing) -> Result<Derivation> {
        Ok(match missing {
            Missing::Diff { base, target } => Derivation::Diff(self.diff_of(&base, &target).await?),
            Missing::Commits(Some(key)) => Derivation::Commits(self.commits_of(key).await?),
            Missing::Commits(None) => Derivation::Commits(Arc::default()),
            Missing::Interdiff(asked) => {
                let compared = self.interdiff_of(asked.key.clone()).await?;
                Derivation::Interdiff(Box::new(asked.with(compared)))
            }
        })
    }

    /// Makes `diffed` the current revision's diff and remaps the review state onto it.
    pub(super) async fn install_diff(&self, st: &mut State, diffed: Arc<Diffed>) -> Result<()> {
        st.changes = Some(Changes::new(diffed));
        self.remap(st).await
    }

    /// Remaps every recorded span onto the changeset and, once computed, each commit's own diff
    /// (§5.2), and resolves every range thread's anchor (§5.1) in the changeset, each from what
    /// was recorded.
    pub(super) async fn remap(&self, st: &mut State) -> Result<()> {
        self.load_known(st).await?;
        let mut result = Ok(());
        for view in st.remapped_views() {
            if result.is_ok() {
                result = self.remap_view(st, view).await;
            }
        }
        result?;
        self.note_progress(st).await
    }

    /// Remaps onto one view of the changeset or of a commit, and counts its completion; only the
    /// changeset resolves threads.
    pub(super) async fn remap_view(&self, st: &mut State, view: View) -> Result<()> {
        let (pieces, resolutions) = self.remapped(st, view, Remapping::All).await?;
        *st.remapped_in_mut(view) = Remapped::of(pieces, resolutions);
        recount(st, view);
        Ok(())
    }

    /// Resolves every range thread's anchor in the changeset again, after the threads changed;
    /// the spans stay as they are remapped.
    pub(super) async fn resolve_threads(&self, st: &mut State) -> Result<()> {
        self.load_known(st).await?;
        let (_, resolutions) = self.remapped(st, View::Changeset, Remapping::Threads).await?;
        st.remapped_in_mut(View::Changeset).resolutions = resolutions;
        Ok(())
    }

    /// Brings every view up to date with a mark, which recorded the spans `added` and dropped the
    /// spans `removed`: only the added spans are remapped, and each view's completion changes by
    /// the files the added and the dropped spans are in.
    pub(super) async fn remap_marks(&self, st: &mut State, removed: &[SpanId], added: &[SpanId]) -> Result<()> {
        self.load_known(st).await?;
        for view in st.remapped_views() {
            let (pieces, _) = self.remapped(st, view, Remapping::Spans(added)).await?;
            let remapped = st.remapped_in(view);
            let files: HashSet<RepoPath> = (removed.iter())
                .flat_map(|&id| remapped.files(id))
                .chain(pieces.iter().flat_map(|(_, p)| files_of(p)))
                .cloned()
                .collect();
            let before = part_completion(st, view, &files);
            let remapped = st.remapped_in_mut(view);
            for &id in removed {
                remapped.remove(id);
            }
            for (id, pieces) in pieces {
                remapped.insert(id, pieces);
            }
            let after = part_completion(st, view, &files);
            let remapped = st.remapped_in_mut(view);
            remapped.completion = remapped.completion + after - before;
        }
        Ok(())
    }

    /// What `remapping` names of the review state, remapped onto the diff of `view` (the spans)
    /// and resolved there (the threads, in the changeset only). The texts read that the diff does
    /// not have are kept in the changes.
    async fn remapped(
        &self,
        st: &mut State,
        view: View,
        remapping: Remapping<'_>,
    ) -> Result<(Vec<(SpanId, Vec<ReviewedSpan>)>, HashMap<ThreadId, Resolution>)> {
        let mut anchored = std::mem::take(&mut st.changes.as_mut().expect("computed").anchored);
        #[cfg(test)]
        let mut counted: Vec<RepoPath> = Vec::new();
        let remapped = {
            let recorded = st.recorded.iter().map(|r| &**r);
            let spans: Vec<&RecordedSpan> = match remapping {
                Remapping::All => recorded.collect(),
                Remapping::Threads => Vec::new(),
                Remapping::Spans(ids) => recorded.filter(|r| ids.contains(&r.id)).collect(),
            };
            let threads: Vec<&Thread> = match remapping {
                Remapping::All | Remapping::Threads if view == View::Changeset => st.threads.iter().collect(),
                _ => Vec::new(),
            };
            #[cfg(test)]
            counted.extend(
                (spans.iter())
                    .flat_map(|r| r.span.base.iter().chain(&r.span.target))
                    .map(|a| a.file.clone()),
            );
            self.remapped_with(st, view, &spans, &threads, &mut anchored).await
        };
        let changes = st.changes.as_mut().expect("computed");
        changes.anchored = anchored;
        #[cfg(test)]
        for file in counted {
            *changes.remaps.entry(file).or_default() += 1;
        }
        remapped
    }

    async fn remapped_with(
        &self,
        st: &State,
        view: View,
        spans: &[&RecordedSpan],
        threads: &[&Thread],
        anchored: &mut Arc<BTreeMap<BlobId, Vec<u8>>>,
    ) -> Result<(Vec<(SpanId, Vec<ReviewedSpan>)>, HashMap<ThreadId, Resolution>)> {
        let diffed = st.diffed_in(view);
        let frame = frame_of(st, view, diffed.diff.renames.clone());
        let Prepared {
            inputs,
            spans: span_slots,
            threads: thread_slots,
        } = self.prepare(st, &frame, diffed, anchored, spans, threads).await?;
        let texts = Texts {
            diffed: &diffed.blobs,
            other: anchored,
        };
        let resolving = view == View::Changeset;
        let to = vet_core::content::Changeset {
            base: frame.base.id,
            target: frame.target.id,
        };
        let prepared: Vec<PreparedFile> = inputs.iter().map(|i| PreparedFile::new(i, &texts)).collect();

        // Moved content is searched for among the changed files of the anchor's side.
        let (base_inputs, target_inputs) = if resolving && thread_slots.iter().any(Option::is_some) {
            changed_inputs(&diffed.diff)
        } else {
            Default::default()
        };
        let base_changed: Vec<PreparedFile> = (base_inputs.iter()).map(|i| PreparedFile::new(i, &texts)).collect();
        let target_changed: Vec<PreparedFile> = (target_inputs.iter()).map(|i| PreparedFile::new(i, &texts)).collect();
        let resolutions: HashMap<ThreadId, Resolution> = threads
            .iter()
            .zip(&thread_slots)
            .filter_map(|(t, slot)| match (&t.target, slot) {
                (CommentTarget::Range(a), Some(i)) if resolving => {
                    let changed = if a.side == Side::Base {
                        &base_changed
                    } else {
                        &target_changed
                    };
                    Some((t.id, resolve_anchor(a, &prepared[*i], changed)))
                }
                _ => None,
            })
            .collect();
        drop((base_changed, target_changed));

        let remapped = spans
            .iter()
            .zip(&span_slots)
            .map(|(rec, &(b, t))| {
                let (b, t) = (b.map(|i| &prepared[i]), t.map(|i| &prepared[i]));
                (rec.id, remap_span(&rec.span, to, b, t, frame.mode_of(rec)))
            })
            .collect();
        Ok((remapped, resolutions))
    }

    /// Loads the snapshots the recorded spans and the range threads are anchored in, and drops
    /// the others.
    pub(super) async fn load_known(&self, st: &mut State) -> Result<()> {
        let referenced: Vec<SnapshotId> = st.anchors().map(|a| a.snapshot).collect();
        for &id in &referenced {
            if st.snapshot(id).is_none()
                && let Some((snapshot, _)) = self.acquisition.store.snapshot(id).await?
            {
                st.known.insert(id, Arc::new(snapshot));
            }
        }
        st.known.retain(|id, _| referenced.contains(id));
        Ok(())
    }

    /// The file pairs of the anchors' sides, and for each of `spans` and `threads` the indices of
    /// its pairs. Loads the texts `diffed` does not have into `anchored`.
    pub(super) async fn prepare(
        &self,
        st: &State,
        frame: &Frame<'_>,
        diffed: &Diffed,
        anchored: &mut Arc<BTreeMap<BlobId, Vec<u8>>>,
        spans: &[&RecordedSpan],
        threads: &[&Thread],
    ) -> Result<Prepared> {
        let anchors = |r: &RecordedSpan| {
            [(true, r.span.base.clone()), (false, r.span.target.clone())]
                .into_iter()
                .filter_map(|(base, a)| a.map(|a| (base, a)))
                .collect::<Vec<_>>()
        };
        let thread_anchor = |t: &Thread| match &t.target {
            CommentTarget::Range(a) => Some((a.side == Side::Base, a.clone())),
            _ => None,
        };
        // Anchored snapshots may be the ones the view is of, which the session holds apart.
        let find = |id: SnapshotId| match id {
            _ if id == frame.base.id => Some(frame.base),
            _ if id == frame.target.id => Some(frame.target),
            _ => st.snapshot(id),
        };

        let file_entry = |snapshot: Option<&Snapshot>, path: &RepoPath| {
            snapshot
                .and_then(|s| s.entries.get(path))
                .filter(|e| e.kind == EntryKind::File)
                .map(|e| e.blob)
        };
        // One prepared file per anchored file and side.
        let mut index: HashMap<(bool, RepoPath, SnapshotId), usize> = HashMap::new();
        let mut inputs: Vec<FileRemapInput> = Vec::new();
        let mut at = |is_base: bool, a: &Anchor| {
            let key = (is_base, a.file.clone(), a.snapshot);
            *index.entry(key).or_insert_with(|| {
                let new_path = landing(frame, is_base, a);
                let current = if is_base { frame.base } else { frame.target };
                inputs.push(FileRemapInput {
                    old_blob: file_entry(find(a.snapshot), &a.file),
                    new_blob: new_path.as_ref().and_then(|p| file_entry(Some(current), p)),
                    old_path: Some(a.file.clone()),
                    new_path,
                });
                inputs.len() - 1
            })
        };
        let spans = spans
            .iter()
            .map(|rec| {
                let mut slot = [None, None];
                for (is_base, a) in anchors(rec) {
                    slot[if is_base { 0 } else { 1 }] = Some(at(is_base, &a));
                }
                (slot[0], slot[1])
            })
            .collect();
        let threads = threads
            .iter()
            .map(|t| thread_anchor(t).map(|(is_base, a)| at(is_base, &a)))
            .collect();
        let mut loaded = Ok(());
        for input in &inputs {
            for (path, blob) in [(&input.old_path, input.old_blob), (&input.new_path, input.new_blob)] {
                if let (Some(path), Some(blob)) = (path, blob)
                    && !diffed.blobs.contains_key(&blob)
                    && !anchored.contains_key(&blob)
                    && let Err(e) = self.acquisition.load(Arc::make_mut(anchored), path, blob).await
                {
                    loaded = Err(e);
                }
            }
        }
        loaded.map(|()| Prepared { inputs, spans, threads })
    }
}

/// The changed text files of the diff as seen on the base side and on the
/// target side, for finding moved content.
fn changed_inputs(diff: &SnapshotDiff) -> (Vec<FileRemapInput>, Vec<FileRemapInput>) {
    let (mut base, mut target) = (Vec::new(), Vec::new());
    for d in &diff.files {
        if let (Some(path), Some(blob)) = (&d.old_path, d.old_blob) {
            base.push(FileRemapInput {
                old_path: Some(path.clone()),
                new_path: Some(path.clone()),
                old_blob: Some(blob),
                new_blob: Some(blob),
            });
        }
        if d.new_path.is_some() && d.new_blob.is_some() {
            target.push(FileRemapInput {
                old_path: d.old_path.clone(),
                new_path: d.new_path.clone(),
                old_blob: d.old_blob,
                new_blob: d.new_blob,
            });
        }
    }
    (base, target)
}
