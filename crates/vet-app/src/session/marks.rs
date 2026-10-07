//! Marking hunks seen and acknowledging non-text items. Marking starts the
//! review if none is open.

use vet_core::completion::HunkState;
use vet_core::content::{BlobId, NonTextKind};
use vet_core::marking::{Region, hunk_regions, hunk_span};
use vet_core::prepared::PreparedFile;
use vet_core::remap::cut_recorded;
use vet_core::review::ReviewedSpan;

use super::interdiff::{View, check_markable, squash_item, update_in};
use super::lens::view_unit;
use super::remap::{Prepared, frame_of};
use super::state::{State, ack_of, check_revision, find_diff, find_non_text, outcome, shown_items};
use super::views::{Outcome, ReviewUpdateView};
use super::{LiveSession, Result, SessionError};
use crate::ports::{GitRepo, LiveStore};
use crate::review::{RecordedSpan, SpanOrigin};

impl<G: GitRepo + Sync, S: LiveStore + Sync> LiveSession<G, S> {
    /// Flips one hunk: seen becomes unseen, anything else (unseen, or seen against an old base)
    /// becomes seen. Marking starts the review if none is open.
    pub async fn toggle_hunk(
        &self,
        revision: u32,
        view: View,
        path: &str,
        hunk: u32,
    ) -> Result<Outcome<ReviewUpdateView>> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_view(&mut st, view).await?;
        check_markable(&st, view)?;
        let state = *update_in(&st, view, path)?
            .review
            .hunks
            .get(hunk as usize)
            .ok_or_else(|| SessionError::NotFound(format!("hunk {hunk}")))?;
        if state == HunkState::Informational {
            return Err(SessionError::Invalid("nothing is left to review in this hunk".into()));
        }
        let seen = state != HunkState::Seen;
        let started = seen && self.begin(&mut st).await?;
        self.mark_hunks_in(&mut st, view, path, &[hunk], seen).await?;
        Ok(outcome(&st, update_in(&st, view, path)?, started))
    }

    /// Marks the hunks of a file that are seen against an old base seen again, against the base
    /// now, and leaves the others as they are. Fails if the file has none.
    pub async fn reconfirm_file(&self, revision: u32, path: &str) -> Result<Outcome<ReviewUpdateView>> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let hunks: Vec<u32> = update_in(&st, View::Changeset, path)?
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
        self.mark_hunks(&mut st, View::Changeset, path, &hunks, true).await?;
        Ok(outcome(&st, update_in(&st, View::Changeset, path)?, false))
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
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_view(&mut st, view).await?;
        check_markable(&st, view)?;
        let acknowledged = !update_in(&st, view, path)?.review.acknowledged.contains(&kind);
        if matches!(view, View::Interdiff { .. }) && !self.has_counterpart(&st, view, path, kind) {
            return Err(SessionError::Invalid("nothing is left to review in this change".into()));
        }
        let started = acknowledged && self.begin(&mut st).await?;
        self.mark_item(&mut st, view, path, kind, acknowledged).await?;
        Ok(outcome(&st, update_in(&st, view, path)?, started))
    }

    /// Marks everything of a file seen: all its hunks and its non-text items; if it
    /// all is seen already, takes every mark back instead. Marking starts the review
    /// if none is open. In an interdiff only what the newer version's changeset has to
    /// review is marked.
    pub async fn toggle_file_seen(&self, revision: u32, view: View, path: &str) -> Result<Outcome<ReviewUpdateView>> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_view(&mut st, view).await?;
        check_markable(&st, view)?;
        let update = update_in(&st, view, path)?;
        let seen = !update.completion.is_complete();
        let started = seen && self.begin(&mut st).await?;
        let (hunks, kinds): (Vec<u32>, Vec<NonTextKind>) = {
            let diff = &st.changes_in(view).diff;
            let hunks = (0..find_diff(diff, path).map_or(0, |d| d.hunks.len() as u32))
                .filter(|&i| update.review.hunks[i as usize] != HunkState::Informational)
                .collect();
            let kinds = shown_items(diff, &st.filter, path)
                .iter()
                .filter(|n| !matches!(view, View::Interdiff { .. }) || self.has_counterpart(&st, view, path, n.kind))
                .filter(|n| update.review.acknowledged.contains(&n.kind) != seen)
                .map(|n| n.kind)
                .collect();
            (hunks, kinds)
        };
        if !hunks.is_empty() {
            self.mark_hunks_in(&mut st, view, path, &hunks, seen).await?;
        }
        for kind in kinds {
            self.mark_item(&mut st, view, path, kind, seen).await?;
        }
        Ok(outcome(&st, update_in(&st, view, path)?, started))
    }

    /// The review state of `path` in the changeset itself, for telling those who show it that a
    /// mark made in an interdiff changed it.
    pub async fn changeset_update(&self, path: &str) -> Result<ReviewUpdateView> {
        let mut st = self.state.lock().await;
        self.ensure_changes(&mut st).await?;
        update_in(&st, View::Changeset, path)
    }

    /// Whether the newer version's changeset has the change of kind `kind` at `path` to review.
    fn has_counterpart(&self, st: &State, view: View, path: &str, kind: NonTextKind) -> bool {
        shown_items(&st.changes_in(view).diff, &st.filter, path)
            .iter()
            .any(|n| n.kind == kind && squash_item(st, n).is_some())
    }

    async fn mark_hunks_in(&self, st: &mut State, view: View, path: &str, hunks: &[u32], seen: bool) -> Result<()> {
        match view {
            View::Changeset | View::Commit { .. } => self.mark_hunks(st, view, path, hunks, seen).await,
            View::Interdiff { from, to } => self.mark_interdiff_hunks(st, (from, to), path, hunks, seen).await,
        }
    }

    /// Records the hunks (indices into the file's hunks) of the changeset or of one commit as
    /// seen, or cuts them out of the recorded spans.
    pub(super) async fn mark_hunks(
        &self,
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
            self.keep_snapshots(st, view).await?;
        }
        let (regions, add) = {
            let changes = st.changes_in(view);
            let diff = find_diff(&changes.diff, path).ok_or_else(|| SessionError::NotFound(format!("file {path}")))?;
            let blob_text = |blob: Option<BlobId>| blob.and_then(|b| changes.blobs.get(&b)).map(Vec::as_slice);
            let ids = (base, target);
            let (mut regions, mut add) = (Vec::new(), Vec::new());
            for &i in hunks {
                let hunk = diff
                    .hunks
                    .get(i as usize)
                    .ok_or_else(|| SessionError::NotFound(format!("hunk {i}")))?;
                regions.extend(hunk_regions(diff, hunk));
                if seen {
                    let texts = (blob_text(diff.old_blob), blob_text(diff.new_blob));
                    add.push(hunk_span(diff, hunk, ids, texts).ok_or_else(|| SessionError::NotText(path.into()))?);
                }
            }
            (regions, add)
        };
        self.apply_marks(st, view, regions, add, origin, seen).await
    }

    /// Cuts the lines of `regions`, which are of `view`, out of the recorded spans, then records
    /// `add` as seen, marked in `origin`.
    pub(super) async fn apply_marks(
        &self,
        st: &mut State,
        view: View,
        regions: Vec<Region>,
        add: Vec<ReviewedSpan>,
        origin: SpanOrigin,
        seen: bool,
    ) -> Result<()> {
        self.load_known(st).await?;
        let mut changes = std::mem::take(st.changes_in_mut(view));
        let prepared = {
            let frame = frame_of(st, view, changes.diff.renames.clone());
            self.prepare(st, &frame, &mut changes).await
        };
        let cut = prepared.map(
            |Prepared {
                 inputs, spans: slots, ..
             }| {
                let prepared: Vec<PreparedFile> = inputs.iter().map(|i| PreparedFile::new(i, &changes.blobs)).collect();
                let mut remove = Vec::new();
                let mut kept: Vec<(ReviewedSpan, SpanOrigin)> = Vec::new();
                for (rec, &(b, t)) in st.recorded.iter().zip(&slots) {
                    let (b, t) = (b.map(|i| &prepared[i]), t.map(|i| &prepared[i]));
                    if let Some(rest) = cut_recorded(&rec.span, &regions, b, t) {
                        remove.push(rec.id);
                        kept.extend(rest.into_iter().map(|s| (s, rec.origin.clone())));
                    }
                }
                kept.extend(add.into_iter().map(|s| (s, origin.clone())));
                (remove, kept)
            },
        );
        *st.changes_in_mut(view) = changes;
        let (remove, add) = cut?;
        // Without a review there is nothing to un-mark and nothing to store.
        let ids = match &st.review {
            Some(review) => {
                self.acquisition
                    .store
                    .edit_spans(review.id, remove.clone(), add.clone())
                    .await?
            }
            None => Vec::new(),
        };
        st.recorded.retain(|r| !remove.contains(&r.id));
        st.recorded.extend(
            ids.into_iter()
                .zip(add)
                .map(|(id, (span, origin))| RecordedSpan { id, span, origin }),
        );
        if seen {
            self.note_reviewed(st).await?;
        }
        self.remap(st).await
    }

    /// Records that something was marked in the changeset's latest version, for
    /// the next review to start from.
    async fn note_reviewed(&self, st: &mut State) -> Result<()> {
        if let Some(review) = &mut st.review
            && review.last_reviewed != Some(review.version)
        {
            self.acquisition
                .store
                .set_last_reviewed(review.id, review.version)
                .await?;
            review.last_reviewed = Some(review.version);
        }
        Ok(())
    }

    pub(super) async fn mark_item(
        &self,
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
            let diff = &st.changes_in(view).diff;
            find_non_text(diff, kind, path).ok_or_else(|| SessionError::NotFound(format!("change of {path}")))?
        };
        let ack = ack_of(change);
        let store = &self.acquisition.store;
        if let Some(review) = &st.review {
            if acknowledged {
                store.acknowledge(review.id, ack.clone()).await?;
            } else {
                store.unacknowledge(review.id, ack.clone()).await?;
            }
        }
        if !acknowledged {
            st.acks.retain(|a| *a != ack);
        } else if !st.acks.contains(&ack) {
            st.acks.push(ack);
        }
        if acknowledged {
            self.note_reviewed(st).await?;
        }
        Ok(())
    }
}
