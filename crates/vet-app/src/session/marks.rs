//! Marking hunks seen and acknowledging non-text items. Marking starts the
//! review if none is open.

use vet_core::completion::HunkState;
use vet_core::content::{BlobId, NonTextKind};
use vet_core::marking::{hunk_regions, hunk_span};
use vet_core::prepared::PreparedFile;
use vet_core::remap::cut_recorded;

use super::manifest::update_of;
use super::remap::Prepared;
use super::state::{State, ack_of, check_revision, find_diff, find_non_text, outcome, shown_items};
use super::views::{Outcome, ReviewUpdateView};
use super::{LiveSession, Result, SessionError};
use crate::ports::{GitRepo, ReviewStore, Store};
use crate::review::RecordedSpan;

impl<G: GitRepo + Sync, S: Store + ReviewStore + Sync> LiveSession<G, S> {
    /// Flips one hunk: seen becomes unseen, anything else (unseen, or seen against an old base)
    /// becomes seen. Marking starts the review if none is open.
    pub async fn toggle_hunk(&self, revision: u32, path: &str, hunk: u32) -> Result<Outcome<ReviewUpdateView>> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let state = *update_of(&st, path)?
            .review
            .hunks
            .get(hunk as usize)
            .ok_or_else(|| SessionError::NotFound(format!("hunk {hunk}")))?;
        let seen = state != HunkState::Seen;
        let started = seen && self.begin(&mut st).await?;
        self.mark_hunks(&mut st, path, &[hunk], seen).await?;
        Ok(outcome(&st, update_of(&st, path)?, started))
    }

    /// Flips the acknowledgement of one non-text item. Acknowledging starts the review if none
    /// is open.
    pub async fn toggle_item(&self, revision: u32, path: &str, kind: NonTextKind) -> Result<Outcome<ReviewUpdateView>> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let acknowledged = !update_of(&st, path)?.review.acknowledged.contains(&kind);
        let started = acknowledged && self.begin(&mut st).await?;
        self.mark_item(&mut st, path, kind, acknowledged).await?;
        Ok(outcome(&st, update_of(&st, path)?, started))
    }

    /// Marks everything of a file seen: all its hunks and its non-text items; if it
    /// all is seen already, takes every mark back instead. Marking starts the review
    /// if none is open.
    pub async fn toggle_file_seen(&self, revision: u32, path: &str) -> Result<Outcome<ReviewUpdateView>> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let seen = !update_of(&st, path)?.completion.is_complete();
        let started = seen && self.begin(&mut st).await?;
        let (hunks, kinds) = {
            let diff = &st.changes.as_ref().expect("computed").diff;
            let hunks = find_diff(diff, path).map_or(0, |d| d.hunks.len() as u32);
            let kinds: Vec<NonTextKind> = shown_items(diff, &st.filter, path)
                .iter()
                .filter(|n| st.acks.contains(&ack_of(n)) != seen)
                .map(|n| n.kind)
                .collect();
            (hunks, kinds)
        };
        if hunks > 0 {
            self.mark_hunks(&mut st, path, &(0..hunks).collect::<Vec<_>>(), seen)
                .await?;
        }
        for kind in kinds {
            self.mark_item(&mut st, path, kind, seen).await?;
        }
        Ok(outcome(&st, update_of(&st, path)?, started))
    }

    /// Records the hunks (indices into the file's hunks) as seen, or cuts them out of the
    /// recorded spans.
    pub(super) async fn mark_hunks(&self, st: &mut State, path: &str, hunks: &[u32], seen: bool) -> Result<()> {
        let Prepared {
            inputs, spans: slots, ..
        } = self.prepare(st).await?;
        let (remove, add) = {
            let changes = st.changes.as_ref().expect("computed");
            let prepared: Vec<PreparedFile> = inputs.iter().map(|i| PreparedFile::new(i, &changes.blobs)).collect();
            let diff = find_diff(&changes.diff, path).ok_or_else(|| SessionError::NotFound(format!("file {path}")))?;
            let blob_text = |blob: Option<BlobId>| blob.and_then(|b| changes.blobs.get(&b)).map(Vec::as_slice);
            let ids = (st.base.id, st.target.id);
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
            let mut remove = Vec::new();
            let mut kept = Vec::new();
            for (rec, &(b, t)) in st.recorded.iter().zip(&slots) {
                let (b, t) = (b.map(|i| &prepared[i]), t.map(|i| &prepared[i]));
                if let Some(rest) = cut_recorded(&rec.span, &regions, b, t) {
                    remove.push(rec.id);
                    kept.extend(rest);
                }
            }
            kept.extend(add);
            (remove, kept)
        };
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
        st.recorded
            .extend(ids.into_iter().zip(add).map(|(id, span)| RecordedSpan { id, span }));
        self.remap(st).await
    }

    pub(super) async fn mark_item(
        &self,
        st: &mut State,
        path: &str,
        kind: NonTextKind,
        acknowledged: bool,
    ) -> Result<()> {
        let change = {
            let diff = &st.changes.as_ref().expect("computed").diff;
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
        Ok(())
    }
}
