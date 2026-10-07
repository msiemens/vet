//! Computing a revision's changes, and remapping recorded spans and thread
//! anchors onto them.

use std::collections::{BTreeMap, HashMap};

use vet_core::content::{Changeset, EntryKind, NonTextKind, RepoPath, Snapshot, SnapshotDiff, SnapshotId};
use vet_core::prepared::PreparedFile;
use vet_core::remap::remap_span;
use vet_core::resolve::{Resolution, resolve_anchor};
use vet_core::review::{Anchor, CommentTarget, FileRemapInput, ReviewedSpan, Side, Thread, ThreadId};

use super::blobs::image_media_type;
use super::state::{Changes, State};
use super::views::{ItemView, ModeChange};
use super::{LiveSession, Result};
use crate::ports::{GitRepo, ReviewStore, Store};
use crate::review::{RecordedSpan, SpanId};

/// Indices into the prepared files of a recorded span's base and target side.
pub(super) type Slot = (Option<usize>, Option<usize>);

pub(super) struct Prepared {
    pub(super) inputs: Vec<FileRemapInput>,
    /// One per recorded span.
    pub(super) spans: Vec<Slot>,
    /// One per thread; `None` unless it is a range thread.
    pub(super) threads: Vec<Option<usize>>,
}

impl<G: GitRepo + Sync, S: Store + ReviewStore + Sync> LiveSession<G, S> {
    pub(super) async fn ensure_changes(&self, st: &mut State) -> Result<()> {
        if st.changes.is_some() {
            return Ok(());
        }
        let diff = self
            .acquisition
            .git
            .diff(&self.acquisition.store, &st.base, &st.target)
            .await?;
        let mut blobs = BTreeMap::new();
        for d in &diff.files {
            for (path, blob) in [(&d.old_path, d.old_blob), (&d.new_path, d.new_blob)] {
                if let (Some(path), Some(blob)) = (path, blob) {
                    self.load(&mut blobs, path, blob).await?;
                }
            }
        }
        let mut items = Vec::new();
        for n in &diff.non_text {
            let before_path = n.from.as_ref().unwrap_or(&n.path);
            items.push(ItemView {
                kind: n.kind,
                before: self.describe(before_path, n.before.as_ref(), n.kind).await,
                after: self.describe(&n.path, n.after.as_ref(), n.kind).await,
                has_preview: n.kind == NonTextKind::Binary
                    && image_media_type(&n.path).is_some()
                    && (n.before.is_some() || n.after.is_some()),
                mode_change: (n.kind == NonTextKind::Mode).then(|| ModeChange::of(n.before.as_ref(), n.after.as_ref())),
            });
        }
        let changed = diff
            .files
            .iter()
            .flat_map(|d| [&d.old_path, &d.new_path])
            .flatten()
            .chain(
                diff.non_text
                    .iter()
                    .flat_map(|n| [Some(&n.path), n.from.as_ref()])
                    .flatten(),
            )
            .cloned()
            .collect();
        st.changes = Some(Changes {
            diff,
            changed,
            blobs,
            remapped: Vec::new(),
            flat: Vec::new(),
            resolutions: HashMap::new(),
            items,
        });
        self.remap(st).await
    }

    /// Remaps every recorded span onto the current changeset (§5.2) and
    /// resolves every range thread's anchor (§5.1), each from what was recorded.
    pub(super) async fn remap(&self, st: &mut State) -> Result<()> {
        let Prepared { inputs, spans, threads } = self.prepare(st).await?;
        let to = Changeset {
            base: st.base.id,
            target: st.target.id,
        };
        let changes = st.changes.as_mut().expect("diffed");
        let prepared: Vec<PreparedFile> = inputs.iter().map(|i| PreparedFile::new(i, &changes.blobs)).collect();

        // Moved content is searched for among the changed files of the anchor's side.
        let (base_inputs, target_inputs) = if threads.iter().any(Option::is_some) {
            changed_inputs(&changes.diff)
        } else {
            Default::default()
        };
        let base_changed: Vec<PreparedFile> = (base_inputs.iter())
            .map(|i| PreparedFile::new(i, &changes.blobs))
            .collect();
        let target_changed: Vec<PreparedFile> = (target_inputs.iter())
            .map(|i| PreparedFile::new(i, &changes.blobs))
            .collect();
        let resolutions: HashMap<ThreadId, Resolution> = st
            .threads
            .iter()
            .zip(&threads)
            .filter_map(|(t, slot)| match (&t.target, slot) {
                (CommentTarget::Range(a), Some(i)) => {
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

        let slots = spans;
        let remapped: Vec<(SpanId, Vec<ReviewedSpan>)> = st
            .recorded
            .iter()
            .zip(&slots)
            .map(|(rec, &(b, t))| {
                (
                    rec.id,
                    remap_span(&rec.span, to, b.map(|i| &prepared[i]), t.map(|i| &prepared[i])),
                )
            })
            .collect();
        drop(prepared);
        changes.flat = remapped.iter().flat_map(|(_, s)| s.iter().cloned()).collect();
        changes.remapped = remapped;
        changes.resolutions = resolutions;
        Ok(())
    }

    /// The file pairs of the anchors' sides, and for each recorded span and
    /// thread the indices of its pairs. Loads the texts into `changes.blobs`.
    pub(super) async fn prepare(&self, st: &mut State) -> Result<Prepared> {
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
        let referenced: Vec<SnapshotId> = st.anchors().map(|a| a.snapshot).collect();
        for &id in &referenced {
            if st.snapshot(id).is_none()
                && let Some((snapshot, _)) = self.acquisition.store.snapshot(id).await?
            {
                st.known.insert(id, snapshot);
            }
        }
        st.known.retain(|id, _| referenced.contains(id));

        let mut blobs = std::mem::take(&mut st.changes.as_mut().expect("diffed").blobs);
        let renames = &st.changes.as_ref().expect("diffed").diff.renames;
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
                let new_path = if is_base || st.target.entries.contains_key(&a.file) {
                    Some(a.file.clone())
                } else {
                    renames.0.get(&a.file).cloned()
                };
                let current = if is_base { &st.base } else { &st.target };
                inputs.push(FileRemapInput {
                    old_blob: file_entry(st.snapshot(a.snapshot), &a.file),
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
                    && let Err(e) = self.load(&mut blobs, path, blob).await
                {
                    loaded = Err(e);
                }
            }
        }
        st.changes.as_mut().expect("diffed").blobs = blobs;
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
