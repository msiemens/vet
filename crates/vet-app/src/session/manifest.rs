//! The manifest: the changeset's files through the review filter, each with
//! its review state and completion.

use std::collections::BTreeMap;
use std::slice;

use vet_core::completion::{Completion, Coverage};
use vet_core::content::{BlobId, FileDiff, LineDelta, NonTextChange, RepoPath, SnapshotDiff};
use vet_core::filter::ReviewFilter;

use super::state::{State, acknowledged, find_diff, lifecycle_of, shown_items};
use super::views::{FileStatus, FileView, ManifestView, ReviewUpdateView, ReviewView, TargetView};
use super::{LiveSession, Result, SessionError};
use crate::editor::line_count;
use crate::ports::{GitRepo, LiveStore};

impl<G: GitRepo + Sync, S: LiveStore + Sync> LiveSession<G, S> {
    pub async fn manifest(&self) -> Result<ManifestView> {
        let mut st = self.state.lock().await;
        self.ensure_changes(&mut st).await?;
        let branches = self.acquisition.git.branches().await?;
        let base = self.base_view(&st, &branches).await?;
        let changes = st.changes.as_ref().expect("computed");
        let cov = Coverage::new(&changes.flat);
        let acked = acknowledged(&st, &changes.diff);
        let compiled = st.filter.compile();

        // Files by path, with their text diff and non-text items.
        type Parts<'a> = (Option<&'a FileDiff>, Vec<usize>);
        let mut by_path: BTreeMap<&RepoPath, Parts> = BTreeMap::new();
        for d in &changes.diff.files {
            by_path.entry(d.path()).or_default().0 = Some(d);
        }
        for (i, n) in changes.diff.non_text.iter().enumerate() {
            by_path.entry(&n.path).or_default().1.push(i);
        }

        // A missing side of a text diff is an empty text.
        let lines = |diff: Option<&FileDiff>, blob: Option<BlobId>| {
            diff.map_or(0, |_| {
                blob.and_then(|b| changes.blobs.get(&b)).map_or(1, |t| line_count(t))
            })
        };
        let mut files = Vec::new();
        for (path, (diff, indices)) in by_path {
            let indices: Vec<usize> = indices
                .into_iter()
                .filter(|&i| compiled.shows(&changes.diff.non_text[i]))
                .collect();
            let items: Vec<NonTextChange> = indices.iter().map(|&i| changes.diff.non_text[i].clone()).collect();
            if !compiled.passes(path) || (diff.is_none() && items.is_empty()) {
                continue;
            }
            let hunks = diff.map(|d| d.hunks.clone()).unwrap_or_default();
            files.push(FileView {
                path: path.clone(),
                old_path: old_path_of(path, diff, &items),
                status: status_of(diff, &items),
                base_lines: lines(diff, diff.and_then(|d| d.old_blob)),
                target_lines: lines(diff, diff.and_then(|d| d.new_blob)),
                delta: LineDelta::of(&hunks),
                hunks,
                review: review_of(&cov, diff, &items, &acked),
                completion: file_completion(&cov, &st.filter, diff, &items, &acked),
                non_text: indices.iter().map(|&i| changes.items[i].clone()).collect(),
            });
        }
        Ok(ManifestView {
            revision: st.revision,
            name: self.name.clone(),
            lifecycle: lifecycle_of(&st),
            base,
            target: TargetView {
                name: "Working tree".into(),
                branch: branches.iter().find(|b| b.current).map(|b| b.name.clone()),
            },
            filter: st.filter.clone(),
            completion: changeset_completion(&cov, &st.filter, &changes.diff, &acked),
            delta: files.iter().map(|f| f.delta).sum(),
            files,
            degraded: st.degraded,
        })
    }
}

/// The base path of a file that is at `path` now and was somewhere else.
pub(super) fn old_path_of(path: &RepoPath, diff: Option<&FileDiff>, items: &[NonTextChange]) -> Option<RepoPath> {
    diff.and_then(|d| d.old_path.clone())
        .or_else(|| items.iter().find_map(|n| n.from.clone()))
        .filter(|old| old != path)
}

pub(super) fn status_of(diff: Option<&FileDiff>, items: &[NonTextChange]) -> FileStatus {
    match diff {
        Some(d) => match (&d.old_path, &d.new_path) {
            (None, _) => FileStatus::Added,
            (_, None) => FileStatus::Deleted,
            (Some(old), Some(new)) if old != new => FileStatus::Renamed,
            _ => FileStatus::Modified,
        },
        None => match items.first() {
            Some(n) if n.before.is_none() => FileStatus::Added,
            Some(n) if n.after.is_none() => FileStatus::Deleted,
            Some(n) if n.from.is_some() => FileStatus::Renamed,
            _ => FileStatus::Modified,
        },
    }
}

fn review_of(cov: &Coverage, diff: Option<&FileDiff>, items: &[NonTextChange], acked: &[NonTextChange]) -> ReviewView {
    ReviewView {
        hunks: diff.map(|d| cov.hunk_states(d)).unwrap_or_default(),
        acknowledged: items.iter().filter(|n| acked.contains(n)).map(|n| n.kind).collect(),
    }
}

/// The review state of one file and the changeset's completion.
pub(super) fn update_of(st: &State, path: &str) -> Result<ReviewUpdateView> {
    let changes = st.changes.as_ref().expect("computed");
    let cov = Coverage::new(&changes.flat);
    let acked = acknowledged(st, &changes.diff);
    let diff = find_diff(&changes.diff, path);
    let items = shown_items(&changes.diff, &st.filter, path);
    let file = diff
        .map(|d| d.path().clone())
        .or_else(|| items.first().map(|n| n.path.clone()))
        .ok_or_else(|| SessionError::NotFound(format!("file {path}")))?;
    Ok(ReviewUpdateView {
        revision: st.revision,
        path: file,
        review: review_of(&cov, diff, &items, &acked),
        completion: file_completion(&cov, &st.filter, diff, &items, &acked),
        changeset: changeset_completion(&cov, &st.filter, &changes.diff, &acked),
    })
}

/// The completion of one file: its text diff, if it has one, and its shown non-text items.
fn file_completion(
    cov: &Coverage,
    filter: &ReviewFilter,
    diff: Option<&FileDiff>,
    items: &[NonTextChange],
    acked: &[NonTextChange],
) -> Completion {
    cov.completion(filter, diff.map(slice::from_ref).unwrap_or_default(), items, acked)
}

/// The completion of everything the diff changes, as far as the filter shows it.
pub(super) fn changeset_completion(
    cov: &Coverage,
    filter: &ReviewFilter,
    diff: &SnapshotDiff,
    acked: &[NonTextChange],
) -> Completion {
    cov.completion(filter, &diff.files, &diff.non_text, acked)
}
