//! Marking hunks seen and acknowledging non-text items. Marking starts the
//! review if none is open.

use std::collections::HashSet;
use std::sync::Arc;

use vet_core::completion::{Completion, HunkState};
use vet_core::content::{BlobId, NonTextKind, RepoPath};
use vet_core::marking::{Region, hunk_regions, hunk_spans};
use vet_core::prepared::PreparedFile;
use vet_core::remap::cut_recorded;
use vet_core::review::{ReviewedSpan, Side, SpanState};

use super::blobs::Texts;
use super::interdiff::{HunkMarks, check_markable, interdiff_hunk_marks, squash_item, update_in};
use super::lens::view_unit;
use super::remap::{Prepared, frame_of, landing, part_completion};
use super::state::{Shown, State, View, ack_of, check_revision, find_diff, find_non_text, outcome, shown_items};
use super::views::{Outcome, ReviewUpdateView};
use super::{LiveSession, Result, SessionError};
use crate::ports::{GitRepo, LiveStore, ReviewWrites};
use crate::review::{RecordedSpan, SpanOrigin};

/// What a mark does: records spans of its kind (`true`) or cuts the lines out of the spans of
/// its kind (`false`). The kinds are independent of each other.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mark {
    Seen(bool),
    Flag(bool),
}

impl Mark {
    fn on(self) -> bool {
        matches!(self, Self::Seen(true) | Self::Flag(true))
    }

    fn is_flag(self) -> bool {
        matches!(self, Self::Flag(_))
    }
}

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
        self.toggle_hunk_mark(revision, view, path, hunk, false).await
    }

    /// Flips the flag of one hunk, which is independent of the hunk being seen and stays until
    /// it is taken away. Flagging starts the review if none is open.
    pub async fn toggle_flag(
        &self,
        revision: u32,
        view: View,
        path: &str,
        hunk: u32,
    ) -> Result<Outcome<ReviewUpdateView>> {
        self.toggle_hunk_mark(revision, view, path, hunk, true).await
    }

    /// Flips the flag of one hunk (`flag`) or whether it is seen.
    async fn toggle_hunk_mark(
        &self,
        revision: u32,
        view: View,
        path: &str,
        hunk: u32,
        flag: bool,
    ) -> Result<Outcome<ReviewUpdateView>> {
        let (mut st, shown) = self.beginning(view).await?;
        check_revision(&st, revision)?;
        check_markable(&shown)?;
        let review = update_in(&st, &shown, path)?.review;
        let index = hunk as usize;
        let (state, flagged) = review
            .hunks
            .get(index)
            .zip(review.flagged.get(index))
            .ok_or_else(|| SessionError::NotFound(format!("hunk {hunk}")))?;
        if *state == HunkState::Informational {
            return Err(SessionError::Invalid("nothing is left to review in this hunk".into()));
        }
        let mark = if flag {
            Mark::Flag(!flagged)
        } else {
            Mark::Seen(*state != HunkState::Seen)
        };
        let started = mark.on() && self.begin(&mut st).await?;
        let writes = st.writes();
        self.mark_hunks_in(&writes, &mut st, &shown, path, &[hunk], mark)
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
        self.mark_hunks(&writes, &mut st, View::Changeset, path, &hunks, Mark::Seen(true))
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
        let seen = !update_in(&st, &shown, path)?.completion.is_complete();
        let started = seen && self.begin(&mut st).await?;
        let (hunks, kinds) = file_targets(&st, &shown, path, seen)?;
        let writes = st.writes();
        if !hunks.is_empty() {
            self.mark_hunks_in(&writes, &mut st, &shown, path, &hunks, Mark::Seen(seen))
                .await?;
        }
        for kind in kinds {
            self.mark_item(&writes, &mut st, view, path, kind, seen).await?;
        }
        Ok(outcome(&st, update_in(&st, &shown, path)?, started))
    }

    /// Marks everything of each of `paths` seen, or takes every mark back, whatever the files'
    /// states are. The marks of all the files are recorded together, which remaps the review
    /// once instead of once per file. Answers with every file's state after the marks. Marking
    /// starts the review if none is open.
    pub async fn set_files_seen(
        &self,
        revision: u32,
        view: View,
        paths: &[String],
        seen: bool,
    ) -> Result<Outcome<Vec<ReviewUpdateView>>> {
        let (mut st, shown) = self.beginning(view).await?;
        check_revision(&st, revision)?;
        check_markable(&shown)?;
        let started = seen && self.begin(&mut st).await?;
        let mut regions = Vec::new();
        let mut add = Vec::new();
        let mut items = Vec::new();
        for path in paths {
            let (hunks, kinds) = file_targets(&st, &shown, path, seen)?;
            if !hunks.is_empty() {
                let (r, a) = hunk_marks(&st, &shown, path, &hunks, seen)?;
                regions.extend(r);
                add.extend(a);
            }
            items.extend(kinds.into_iter().map(|kind| (path, kind)));
        }
        let writes = st.writes();
        if !regions.is_empty() || !add.is_empty() {
            let marked = match &shown {
                Shown::Interdiff(_) => View::Changeset,
                shown => shown.view(),
            };
            self.keep_commit_snapshots(&writes, &st, marked, seen).await?;
            self.apply_marks(&writes, &mut st, marked, regions, add, Mark::Seen(seen))
                .await?;
        }
        for (path, kind) in items {
            self.mark_item(&writes, &mut st, view, path, kind, seen).await?;
        }
        let updates = paths
            .iter()
            .map(|path| update_in(&st, &shown, path))
            .collect::<Result<Vec<_>>>()?;
        Ok(outcome(&st, updates, started))
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
        mark: Mark,
    ) -> Result<()> {
        match shown {
            Shown::Interdiff(_) => {
                let (regions, add) = hunk_marks(st, shown, path, hunks, mark.on())?;
                self.apply_marks(writes, st, View::Changeset, regions, add, mark).await
            }
            _ => self.mark_hunks(writes, st, shown.view(), path, hunks, mark).await,
        }
    }

    /// Records the hunks (indices into the file's hunks) of the changeset or of one commit as
    /// marked, or cuts them out of the recorded spans of that kind.
    pub(super) async fn mark_hunks(
        &self,
        writes: &S::Writes,
        st: &mut State,
        view: View,
        path: &str,
        hunks: &[u32],
        mark: Mark,
    ) -> Result<()> {
        self.keep_commit_snapshots(writes, st, view, mark.on()).await?;
        let (regions, add) = view_hunk_marks(st, view, path, hunks, mark.on())?;
        self.apply_marks(writes, st, view, regions, add, mark).await
    }

    async fn keep_commit_snapshots(&self, writes: &S::Writes, st: &State, view: View, seen: bool) -> Result<()> {
        if seen && st.review.is_some() && matches!(view, View::Commit { .. }) {
            // The snapshots a commit's marks are anchored in are not the changeset's own, so
            // they must be there to be loaded once the session is not.
            self.keep_snapshots(writes, st, view).await?;
        }
        Ok(())
    }

    /// Cuts the lines of `regions`, which are of `view`, out of the recorded spans of the kind
    /// `mark`, then records `add` as spans of that kind, each marked in its origin.
    pub(super) async fn apply_marks(
        &self,
        writes: &S::Writes,
        st: &mut State,
        view: View,
        regions: Vec<Region>,
        mut add: Vec<(ReviewedSpan, SpanOrigin)>,
        mark: Mark,
    ) -> Result<()> {
        if mark.is_flag() {
            for (span, _) in &mut add {
                span.state = SpanState::Flagged;
            }
        }
        self.load_known(st).await?;
        let mut anchored = std::mem::take(&mut st.changes.as_mut().expect("computed").anchored);
        let diffed = st.diffed_in(view);
        let frame = frame_of(st, view, diffed.diff.renames.clone());
        // Only a span with a side that lands in a region's file can lose lines to it.
        let cut_files: HashSet<(RepoPath, Side)> = regions.iter().map(|r| (r.file.clone(), r.side)).collect();
        let cuttable: Vec<&RecordedSpan> = (st.recorded.iter())
            .map(|rec| &**rec)
            .filter(|rec| (rec.span.state == SpanState::Flagged) == mark.is_flag())
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
        // The spans are stored and recorded. If they cannot be placed in the views, the views are
        // dropped, to be derived again from the spans by the next request, and the mark is told as
        // failed to show.
        if let Err(e) = self.remap_marks(st, &remove, &ids).await {
            st.changes = None;
            return Err(e);
        }
        if mark == Mark::Seen(true) {
            self.note_reviewed(writes, st).await;
        }
        self.note_progress(writes, st).await;
        Ok(())
    }

    /// Records that something was marked in the changeset's latest version, for
    /// the next review to start from. What cannot be stored is only logged: it only tells where
    /// the next review starts from.
    async fn note_reviewed(&self, writes: &S::Writes, st: &mut State) {
        if let Some(review) = &mut st.review
            && review.last_reviewed != Some(review.version)
        {
            match writes.set_last_reviewed(review.id, review.version).await {
                Ok(()) => review.last_reviewed = Some(review.version),
                Err(e) => tracing::warn!(error = %e.0, "could not store where the review was last marked"),
            }
        }
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
            self.note_reviewed(writes, st).await;
        }
        self.note_progress(writes, st).await;
        Ok(())
    }
}

/// What marking the hunks of `view` (the changeset or one commit) seen, or cutting them out of the
/// recorded spans, does.
fn view_hunk_marks(st: &State, view: View, path: &str, hunks: &[u32], seen: bool) -> Result<HunkMarks> {
    let (base, target, origin) = view_unit(st, view);
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
    Ok((regions, add))
}

/// [`view_hunk_marks`] for whatever `shown` is; an interdiff's marks are of the changeset.
fn hunk_marks(st: &State, shown: &Shown, path: &str, hunks: &[u32], seen: bool) -> Result<HunkMarks> {
    match shown {
        Shown::Interdiff(id) => interdiff_hunk_marks(st, id, path, hunks, seen),
        _ => view_hunk_marks(st, shown.view(), path, hunks, seen),
    }
}

/// The hunks (indices into the file's) and the non-text items of `path` that marking it all seen,
/// or all unseen, changes. In an interdiff only what the newer version's changeset has to review.
fn file_targets(st: &State, shown: &Shown, path: &str, seen: bool) -> Result<(Vec<u32>, Vec<NonTextKind>)> {
    let update = update_in(st, shown, path)?;
    let diff = &shown.diffed(st).diff;
    let unchanged = if seen { HunkState::Seen } else { HunkState::Unseen };
    let hunks = (0..find_diff(diff, path).map_or(0, |d| d.hunks.len() as u32))
        .filter(|&i| {
            let state = update.review.hunks[i as usize];
            state != HunkState::Informational && state != unchanged
        })
        .collect();
    let interdiff = matches!(shown, Shown::Interdiff(_));
    let kinds = shown_items(diff, &st.filter, path)
        .iter()
        .filter(|n| !interdiff || has_counterpart(st, shown, path, n.kind))
        .filter(|n| update.review.acknowledged.contains(&n.kind) != seen)
        .map(|n| n.kind)
        .collect();
    Ok((hunks, kinds))
}

/// Whether the newer version's changeset has the change of kind `kind` at `path` to review.
fn has_counterpart(st: &State, shown: &Shown, path: &str, kind: NonTextKind) -> bool {
    shown_items(&shown.diffed(st).diff, &st.filter, path)
        .iter()
        .any(|n| n.kind == kind && squash_item(st, n).is_some())
}
