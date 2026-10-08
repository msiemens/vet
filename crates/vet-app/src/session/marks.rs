//! Marking hunks seen and acknowledging non-text items. Marking starts the
//! review if none is open.

use std::collections::HashSet;
use std::sync::Arc;

use vet_core::completion::{Completion, HunkState};
use vet_core::content::{BlobId, NonTextKind, RepoPath};
use vet_core::marking::{Region, hunk_regions, hunk_spans};
use vet_core::prepared::PreparedFile;
use vet_core::remap::cut_recorded;
use vet_core::review::{ReviewedSpan, Side};

use super::blobs::Texts;
use super::interdiff::{check_markable, squash_item, update_in};
use super::lens::view_unit;
use super::remap::{Prepared, frame_of, landing, part_completion};
use super::state::{Shown, State, View, ack_of, check_revision, find_diff, find_non_text, outcome, shown_items};
use super::views::{Outcome, ReviewUpdateView};
use super::{LiveSession, Result, SessionError};
use crate::ports::{GitRepo, LiveStore, ReviewWrites};
use crate::review::{RecordedSpan, SpanOrigin};

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// Flips one hunk: seen becomes unseen, anything else (unseen, or seen against an old base)
    /// becomes seen. Marking starts the review if none is open.
    pub async fn toggle_hunk(
        &self,
        revision: u32,
        view: View,
        path: &str,
        hunk: u32,
    ) -> Result<Outcome<ReviewUpdateView>> {
        let (mut st, shown) = self.beginning(view).await?;
        check_revision(&st, revision)?;
        check_markable(&shown)?;
        let state = *update_in(&st, &shown, path)?
            .review
            .hunks
            .get(hunk as usize)
            .ok_or_else(|| SessionError::NotFound(format!("hunk {hunk}")))?;
        if state == HunkState::Informational {
            return Err(SessionError::Invalid("nothing is left to review in this hunk".into()));
        }
        let seen = state != HunkState::Seen;
        let started = seen && self.begin(&mut st).await?;
        let writes = st.writes();
        self.mark_hunks_in(&writes, &mut st, &shown, path, &[hunk], seen)
            .await?;
        Ok(outcome(&st, update_in(&st, &shown, path)?, started))
    }

    /// Marks the hunks of a file that are seen against an old base seen again, against the base
    /// now, and leaves the others as they are. Fails if the file has none.
    pub async fn reconfirm_file(&self, revision: u32, path: &str) -> Result<Outcome<ReviewUpdateView>> {
        let (mut st, shown) = self.writing(View::Changeset).await?;
        check_revision(&st, revision)?;
        let hunks: Vec<u32> = update_in(&st, &shown, path)?
            .review
            .hunks
            .iter()
            .enumerate()
            .filter(|(_, state)| **state == HunkState::SeenAgainstOldBase)
            .map(|(i, _)| i as u32)
            .collect();
        if hunks.is_empty() {
            return Err(SessionError::Invalid(
                "nothing in this file was seen against an old base".into(),
            ));
        }
        let writes = st.writes();
        self.mark_hunks(&writes, &mut st, View::Changeset, path, &hunks, true)
            .await?;
        Ok(outcome(&st, update_in(&st, &shown, path)?, false))
    }

    /// Flips the acknowledgement of one non-text item. Acknowledging starts the review if none
    /// is open. In an interdiff it is the acknowledgement of the newer version's own change.
    pub async fn toggle_item(
        &self,
        revision: u32,
        view: View,
        path: &str,
        kind: NonTextKind,
    ) -> Result<Outcome<ReviewUpdateView>> {
        let (mut st, shown) = self.beginning(view).await?;
        check_revision(&st, revision)?;
        check_markable(&shown)?;
        let acknowledged = !update_in(&st, &shown, path)?.review.acknowledged.contains(&kind);
        if matches!(view, View::Interdiff { .. }) && !has_counterpart(&st, &shown, path, kind) {
            return Err(SessionError::Invalid("nothing is left to review in this change".into()));
        }
        let started = acknowledged && self.begin(&mut st).await?;
        let writes = st.writes();
        self.mark_item(&writes, &mut st, view, path, kind, acknowledged).await?;
        Ok(outcome(&st, update_in(&st, &shown, path)?, started))
    }

    /// Marks everything of a file seen: all its hunks and its non-text items; if it
    /// all is seen already, takes every mark back instead. Marking starts the review
    /// if none is open. In an interdiff only what the newer version's changeset has to
    /// review is marked.
    pub async fn toggle_file_seen(&self, revision: u32, view: View, path: &str) -> Result<Outcome<ReviewUpdateView>> {
        let (mut st, shown) = self.beginning(view).await?;
        check_revision(&st, revision)?;
        check_markable(&shown)?;
        let update = update_in(&st, &shown, path)?;
        let seen = !update.completion.is_complete();
        let started = seen && self.begin(&mut st).await?;
        let (hunks, kinds): (Vec<u32>, Vec<NonTextKind>) = {
            let diff = &shown.diffed(&st).diff;
            let hunks = (0..find_diff(diff, path).map_or(0, |d| d.hunks.len() as u32))
                .filter(|&i| update.review.hunks[i as usize] != HunkState::Informational)
                .collect();
            let kinds = shown_items(diff, &st.filter, path)
                .iter()
                .filter(|n| !matches!(view, View::Interdiff { .. }) || has_counterpart(&st, &shown, path, n.kind))
                .filter(|n| update.review.acknowledged.contains(&n.kind) != seen)
                .map(|n| n.kind)
                .collect();
            (hunks, kinds)
        };
        let writes = st.writes();
        if !hunks.is_empty() {
            self.mark_hunks_in(&writes, &mut st, &shown, path, &hunks, seen).await?;
        }
        for kind in kinds {
            self.mark_item(&writes, &mut st, view, path, kind, seen).await?;
        }
        Ok(outcome(&st, update_in(&st, &shown, path)?, started))
    }

    /// The review state of `path` in the changeset itself, for telling those who show it that a
    /// mark made in an interdiff changed it.
    pub async fn changeset_update(&self, path: &str) -> Result<ReviewUpdateView> {
        let (st, shown) = self.viewing(View::Changeset).await?;
        update_in(&st, &shown, path)
    }

    async fn mark_hunks_in(
        &self,
        writes: &S::Writes,
        st: &mut State,
        shown: &Shown,
        path: &str,
        hunks: &[u32],
        seen: bool,
    ) -> Result<()> {
        match shown {
            Shown::Interdiff(id) => self.mark_interdiff_hunks(writes, st, id, path, hunks, seen).await,
            _ => self.mark_hunks(writes, st, shown.view(), path, hunks, seen).await,
        }
    }

    /// Records the hunks (indices into the file's hunks) of the changeset or of one commit as
    /// seen, or cuts them out of the recorded spans.
    pub(super) async fn mark_hunks(
        &self,
        writes: &S::Writes,
        st: &mut State,
        view: View,
        path: &str,
        hunks: &[u32],
        seen: bool,
    ) -> Result<()> {
        let (base, target, origin) = view_unit(st, view);
        if seen && st.review.is_some() && matches!(view, View::Commit { .. }) {
            // The snapshots a commit's marks are anchored in are not the changeset's own, so
            // they must be there to be loaded once the session is not.
            self.keep_snapshots(writes, st, view).await?;
        }
        let (regions, add) = {
            let diffed = st.diffed_in(view);
            let diff = find_diff(&diffed.diff, path).ok_or_else(|| SessionError::NotFound(format!("file {path}")))?;
            let blob_text = |blob: Option<BlobId>| blob.and_then(|b| diffed.blobs.get(&b)).map(Vec::as_slice);
            let chosen = (hunks.iter())
                .map(|&i| (diff.hunks.get(i as usize)).ok_or_else(|| SessionError::NotFound(format!("hunk {i}"))))
                .collect::<Result<Vec<_>>>()?;
            let regions = chosen.iter().flat_map(|hunk| hunk_regions(diff, hunk)).collect();
            let add = if seen {
                let texts = (blob_text(diff.old_blob), blob_text(diff.new_blob));
                (hunk_spans(diff, &chosen, (base, target), texts).into_iter())
                    .map(|span| span.map(|s| (s, origin.clone())))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| SessionError::NotText(path.into()))?
            } else {
                Vec::new()
            };
            (regions, add)
        };
        self.apply_marks(writes, st, view, regions, add, seen).await
    }

    /// Cuts the lines of `regions`, which are of `view`, out of the recorded spans, then records
    /// `add` as seen, each marked in its origin.
    pub(super) async fn apply_marks(
        &self,
        writes: &S::Writes,
        st: &mut State,
        view: View,
        regions: Vec<Region>,
        add: Vec<(ReviewedSpan, SpanOrigin)>,
        seen: bool,
    ) -> Result<()> {
        self.load_known(st).await?;
        let mut anchored = std::mem::take(&mut st.changes.as_mut().expect("computed").anchored);
        let diffed = st.diffed_in(view);
        let frame = frame_of(st, view, diffed.diff.renames.clone());
        // Only a span with a side that lands in a region's file can lose lines to it.
        let cut_files: HashSet<(RepoPath, Side)> = regions.iter().map(|r| (r.file.clone(), r.side)).collect();
        let cuttable: Vec<&RecordedSpan> = (st.recorded.iter())
            .map(|rec| &**rec)
            .filter(|rec| {
                [(true, &rec.span.base), (false, &rec.span.target)]
                    .into_iter()
                    .filter_map(|(is_base, a)| a.as_ref().map(|a| (is_base, a)))
                    .any(|(is_base, a)| {
                        landing(&frame, is_base, a).is_some_and(|file| cut_files.contains(&(file, a.side)))
                    })
            })
            .collect();
        let prepared = self.prepare(st, &frame, diffed, &mut anchored, &cuttable, &[]).await;
        let cut = prepared.map(
            |Prepared {
                 inputs, spans: slots, ..
             }| {
                let texts = Texts {
                    diffed: &diffed.blobs,
                    other: &anchored,
                };
                let prepared: Vec<PreparedFile> = inputs.iter().map(|i| PreparedFile::new(i, &texts)).collect();
                let mut remove = Vec::new();
                let mut kept: Vec<(ReviewedSpan, SpanOrigin)> = Vec::new();
                for (rec, &(b, t)) in cuttable.iter().zip(&slots) {
                    let (b, t) = (b.map(|i| &prepared[i]), t.map(|i| &prepared[i]));
                    if let Some(rest) = cut_recorded(&rec.span, &regions, b, t) {
                        remove.push(rec.id);
                        kept.extend(rest.into_iter().map(|s| (s, rec.origin.clone())));
                    }
                }
                kept.extend(add);
                (remove, kept)
            },
        );
        st.changes.as_mut().expect("computed").anchored = anchored;
        let (remove, add) = cut?;
        // Without a review there is nothing to un-mark and nothing to store.
        let ids = match &st.review {
            Some(review) => writes.edit_spans(review.id, remove.clone(), add.clone()).await?,
            None => Vec::new(),
        };
        st.recorded.retain(|r| !remove.contains(&r.id));
        st.recorded.extend(
            ids.iter()
                .zip(add)
                .map(|(&id, (span, origin))| Arc::new(RecordedSpan { id, span, origin })),
        );
        if seen {
            self.note_reviewed(writes, st).await?;
        }
        self.remap_marks(st, &remove, &ids).await?;
        self.note_progress(writes, st).await
    }

    /// Records that something was marked in the changeset's latest version, for
    /// the next review to start from.
    async fn note_reviewed(&self, writes: &S::Writes, st: &mut State) -> Result<()> {
        if let Some(review) = &mut st.review
            && review.last_reviewed != Some(review.version)
        {
            writes.set_last_reviewed(review.id, review.version).await?;
            review.last_reviewed = Some(review.version);
        }
        Ok(())
    }

    pub(super) async fn mark_item(
        &self,
        writes: &S::Writes,
        st: &mut State,
        view: View,
        path: &str,
        kind: NonTextKind,
        acknowledged: bool,
    ) -> Result<()> {
        // In an interdiff the acknowledgement is of the changeset's own change.
        let view = if matches!(view, View::Commit { .. }) {
            view
        } else {
            View::Changeset
        };
        let change = {
            let diff = &st.diffed_in(view).diff;
            find_non_text(diff, kind, path).ok_or_else(|| SessionError::NotFound(format!("change of {path}")))?
        };
        let ack = ack_of(change);
        let files: HashSet<RepoPath> = change.from.iter().chain([&change.path]).cloned().collect();
        if let Some(review) = &st.review {
            if acknowledged {
                writes.acknowledge(review.id, ack.clone()).await?;
            } else {
                writes.unacknowledge(review.id, ack.clone()).await?;
            }
        }
        // Only the completion of the changes of the acknowledged change's paths moves.
        let views = st.remapped_views();
        let before: Vec<Completion> = views.iter().map(|&v| part_completion(st, v, &files)).collect();
        if !acknowledged {
            st.acks.retain(|a| *a != ack);
        } else if !st.acks.contains(&ack) {
            st.acks.push(ack);
        }
        for (view, before) in views.into_iter().zip(before) {
            let after = part_completion(st, view, &files);
            let remapped = st.remapped_in_mut(view);
            remapped.completion = remapped.completion + after - before;
        }
        if acknowledged {
            self.note_reviewed(writes, st).await?;
        }
        self.note_progress(writes, st).await
    }
}

/// Whether the newer version's changeset has the change of kind `kind` at `path` to review.
fn has_counterpart(st: &State, shown: &Shown, path: &str, kind: NonTextKind) -> bool {
    shown_items(&shown.diffed(st).diff, &st.filter, path)
        .iter()
        .any(|n| n.kind == kind && squash_item(st, n).is_some())
}
