//! Computing a revision's changes, and remapping recorded spans and thread
//! anchors onto them.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use tokio::sync::MutexGuard;
use vet_core::content::{BlobId, EntryKind, RenameMap, RepoPath, Snapshot, SnapshotDiff, SnapshotId};
use vet_core::prepared::PreparedFile;
use vet_core::remap::{RemapMode, remap_span};
use vet_core::resolve::{Resolution, resolve_anchor};
use vet_core::review::{Anchor, CommentTarget, FileRemapInput, ReviewedSpan, Side, Thread, ThreadId};

use super::blobs::Texts;
use super::derived::{CommitUnit, CommitsBetween, DiffOf, Diffed};
use super::interdiff::Interdiffing;
use super::state::{Changes, Interdiffed, Remapped, Shown, State, View};
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

/// What the state lacks to show a view, computed with the state released.
enum Missing {
    Diff { base: Snapshot, target: Snapshot },
    Commits(CommitsBetween),
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
    /// The changes of the current revision, computing the diff if the state has none. A command
    /// that holds the state calls this; requests that start out with it use [`LiveSession::viewing`].
    pub(super) async fn ensure_changes(&self, st: &mut State) -> Result<()> {
        if st.changes.is_some() {
            return Ok(());
        }
        let diffed = self.diff_of(&st.base, &st.target).await?;
        self.install_diff(st, diffed).await
    }

    /// The state on view with what `view` shows computed, and what of it is not kept in the state.
    /// What is derived (a diff, an interdiff, the commits) is awaited with the state released, so
    /// that other requests go on meanwhile and requests of the same value share its computation;
    /// only the remap of the review state onto it runs under the state.
    pub(super) async fn viewing(&self, view: View) -> Result<(MutexGuard<'_, State>, Shown)> {
        self.prepared(view, matches!(view, View::Commit { .. })).await
    }

    /// Like [`LiveSession::viewing`]; with `commits`, the commits of the changeset are computed too.
    pub(super) async fn prepared(&self, view: View, commits: bool) -> Result<(MutexGuard<'_, State>, Shown)> {
        let mut st = self.state.lock().await;
        loop {
            let missing = match self.missing(&mut st, view, commits).await? {
                Ok(shown) => return Ok((st, shown)),
                Err(missing) => missing,
            };
            let revision = st.revision;
            drop(st);
            let derived = self.derive(missing).await;
            st = self.state.lock().await;
            if st.revision != revision {
                continue;
            }
            match derived? {
                Derivation::Diff(diffed) => {
                    if st.changes.is_none() {
                        self.install_diff(&mut st, diffed).await?;
                    }
                }
                Derivation::Commits(units) => {
                    if st.changes.as_ref().is_some_and(|c| c.commits.is_none()) {
                        self.install_commits(&mut st, units).await?;
                    }
                }
                Derivation::Interdiff(interdiffed) => return Ok((st, Shown::Interdiff(interdiffed))),
            }
        }
    }

    /// What `view` shows if the state and the kept values have it, else what is missing.
    async fn missing(&self, st: &mut State, view: View, commits: bool) -> Result<std::result::Result<Shown, Missing>> {
        if st.changes.is_none() {
            let key = DiffOf {
                old: st.base.id,
                new: st.target.id,
            };
            let Some(diffed) = self.derived.diffs.peek(&key) else {
                let (base, target) = (st.base.clone(), st.target.clone());
                return Ok(Err(Missing::Diff { base, target }));
            };
            self.install_diff(st, diffed).await?;
        }
        if commits && st.changes().commits.is_none() {
            let units = match self.commits_key(st) {
                None => Arc::default(),
                Some(key) => match self.derived.commits.peek(&key) {
                    Some(units) => units,
                    None => return Ok(Err(Missing::Commits(key))),
                },
            };
            self.install_commits(st, units).await?;
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
            Missing::Commits(key) => Derivation::Commits(self.commits_of(key).await?),
            Missing::Interdiff(asked) => {
                let compared = self.interdiff_of(asked.key.clone()).await?;
                Derivation::Interdiff(Box::new(asked.with(compared)))
            }
        })
    }

    /// Makes `diffed` the current revision's diff and remaps the review state onto it.
    async fn install_diff(&self, st: &mut State, diffed: Arc<Diffed>) -> Result<()> {
        st.changes = Some(Changes::new(diffed));
        self.remap(st).await
    }

    /// Remaps every recorded span onto the changeset and, once computed, each commit's own diff
    /// (§5.2), and resolves every range thread's anchor (§5.1) in the changeset, each from what
    /// was recorded.
    pub(super) async fn remap(&self, st: &mut State) -> Result<()> {
        self.load_known(st).await?;
        let mut result = self.remap_view(st, View::Changeset).await;
        let count = (st.changes.as_ref())
            .and_then(|c| c.commits.as_ref())
            .map_or(0, |commits| commits.units.len());
        for index in 0..count as u32 {
            if result.is_ok() {
                result = self.remap_view(st, View::Commit { index }).await;
            }
        }
        result?;
        self.note_progress(st).await
    }

    /// Remaps onto one view of the changeset or of a commit; only the changeset resolves threads.
    pub(super) async fn remap_view(&self, st: &mut State, view: View) -> Result<()> {
        let mut anchored = std::mem::take(&mut st.changes.as_mut().expect("computed").anchored);
        let remapped = self.remapped(st, view, &mut anchored).await;
        st.changes.as_mut().expect("computed").anchored = anchored;
        *st.remapped_in_mut(view) = remapped?;
        Ok(())
    }

    /// The review state remapped onto the diff of `view`; `anchored` receives the texts it reads
    /// that the diff does not have.
    async fn remapped(&self, st: &State, view: View, anchored: &mut BTreeMap<BlobId, Vec<u8>>) -> Result<Remapped> {
        let diffed = st.diffed_in(view);
        let frame = frame_of(st, view, diffed.diff.renames.clone());
        let Prepared { inputs, spans, threads } = self.prepare(st, &frame, diffed, anchored).await?;
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
        let (base_inputs, target_inputs) = if resolving && threads.iter().any(Option::is_some) {
            changed_inputs(&diffed.diff)
        } else {
            Default::default()
        };
        let base_changed: Vec<PreparedFile> = (base_inputs.iter()).map(|i| PreparedFile::new(i, &texts)).collect();
        let target_changed: Vec<PreparedFile> = (target_inputs.iter()).map(|i| PreparedFile::new(i, &texts)).collect();
        let resolutions: HashMap<ThreadId, Resolution> = st
            .threads
            .iter()
            .zip(&threads)
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

        let remapped: Vec<(SpanId, Vec<ReviewedSpan>)> = st
            .recorded
            .iter()
            .zip(&spans)
            .map(|(rec, &(b, t))| {
                let (b, t) = (b.map(|i| &prepared[i]), t.map(|i| &prepared[i]));
                (rec.id, remap_span(&rec.span, to, b, t, frame.mode_of(rec)))
            })
            .collect();
        Ok(Remapped {
            flat: remapped.iter().flat_map(|(_, s)| s.iter().cloned()).collect(),
            spans: remapped,
            resolutions,
        })
    }

    /// Loads the snapshots the recorded spans and the range threads are anchored in, and drops
    /// the others.
    pub(super) async fn load_known(&self, st: &mut State) -> Result<()> {
        let referenced: Vec<SnapshotId> = st.anchors().map(|a| a.snapshot).collect();
        for &id in &referenced {
            if st.snapshot(id).is_none()
                && let Some((snapshot, _)) = self.acquisition.store.snapshot(id).await?
            {
                st.known.insert(id, snapshot);
            }
        }
        st.known.retain(|id, _| referenced.contains(id));
        Ok(())
    }

    /// The file pairs of the anchors' sides, and for each recorded span and
    /// thread the indices of its pairs. Loads the texts `diffed` does not have into `anchored`.
    pub(super) async fn prepare(
        &self,
        st: &State,
        frame: &Frame<'_>,
        diffed: &Diffed,
        anchored: &mut BTreeMap<BlobId, Vec<u8>>,
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

        let renames = &frame.renames;
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
                // The base side stays at its path; the target side follows a
                // rename from `HEAD` if its file is gone.
                let new_path = if is_base || frame.target.entries.contains_key(&a.file) {
                    Some(a.file.clone())
                } else {
                    renames.0.get(&a.file).cloned()
                };
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
        let mut spans: Vec<Slot> = Vec::new();
        for rec in &st.recorded {
            let mut slot = [None, None];
            for (is_base, a) in anchors(rec) {
                slot[if is_base { 0 } else { 1 }] = Some(at(is_base, &a));
            }
            spans.push((slot[0], slot[1]));
        }
        let threads = st
            .threads
            .iter()
            .map(|t| thread_anchor(t).map(|(is_base, a)| at(is_base, &a)))
            .collect();
        let mut loaded = Ok(());
        for input in &inputs {
            for (path, blob) in [(&input.old_path, input.old_blob), (&input.new_path, input.new_blob)] {
                if let (Some(path), Some(blob)) = (path, blob)
                    && !diffed.blobs.contains_key(&blob)
                    && let Err(e) = self.acquisition.load(anchored, path, blob).await
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
