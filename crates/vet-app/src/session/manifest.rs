//! The manifest: the changeset's files through the review filter, each with
//! its review state and completion.

use std::collections::BTreeMap;
use std::slice;

use vet_core::completion::{Completion, Coverage};
use vet_core::content::{BlobId, FileDiff, LineDelta, NonTextChange, RepoPath, SnapshotDiff};
use vet_core::filter::{CompiledFilter, Exclusion, ReviewFilter};

use super::derived::Diffed;
use super::state::{Remapped, Shown, State, View, acknowledged, find_diff, lifecycle_of, shown_items};
use super::views::{
    ExcludedBy, ExcludedFile, ExcludedGroup, FileStatus, FileView, ManifestView, ReviewUpdateView, ReviewView,
    TargetView,
};
use super::{LiveSession, Result, SessionError};
use crate::changeset::{CommitId, Source};
use crate::editor::line_count;
use crate::ports::{BranchInfo, GitRepo, LiveStore, ReviewWrites};
use crate::review::ReviewSummary;

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    pub async fn manifest(&self, view: View) -> Result<ManifestView> {
        let (st, shown) = self.viewing(view).await?;
        match &shown {
            Shown::Interdiff(id) => return self.interdiff_manifest(&st, id).await,
            Shown::Commit { index } => return self.commit_manifest(&st, *index).await,
            Shown::Changeset => {}
        }
        let branches = self.acquisition.git.branches().await?;
        let facts = self.lens_facts(&st, &branches).await?;
        let changes = st.changes();
        let cov = changes.remapped.coverage();
        let acked = acknowledged(&st, &changes.diffed.diff.non_text);
        let compiled = st.filter.compile(&changes.diffed.diff.generated);
        let files = file_views(&changes.diffed, &st.filter, |diff, items| {
            (
                review_of(&cov, diff, items, &acked),
                file_completion(&cov, &compiled, diff, items, &acked),
            )
        });
        Ok(ManifestView {
            revision: st.revision,
            seq: st.seq,
            name: self.name.clone(),
            lifecycle: lifecycle_of(&st),
            base: facts.base.clone(),
            target: target_view(&st.source, &st.head, &branches),
            filter: st.project_filter().clone(),
            narrowing: st.narrowing(),
            completion: changes.remapped.completion,
            delta: files.iter().map(|f| f.delta).sum(),
            excluded: excluded_groups(&changes.diffed, &st),
            files,
            degraded: self.degraded_for(&st.source),
            interdiff: None,
            lens: facts.lens,
            commits: facts.commits,
            changeset_base: facts.base,
            changeset_files: facts.files,
            changeset_completion: facts.completion,
        })
    }
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// Stores how far the open review is when it differs from what is stored, so that the list of
    /// changesets shows it without remapping the marks of every review. Does nothing while the
    /// changes are not computed: the next remap measures.
    pub(super) async fn note_progress(&self, writes: &S::Writes, st: &mut State) -> Result<()> {
        let (Some(review), Some(changes)) = (&st.review, &st.changes) else {
            return Ok(());
        };
        let summary = ReviewSummary::of(&changes.remapped.completion);
        if review.summary == Some(summary) {
            return Ok(());
        }
        let id = review.id;
        writes.set_review_summary(id, summary).await?;
        if let Some(review) = &mut st.review {
            review.summary = Some(summary);
        }
        Ok(())
    }
}

/// What the changeset of `source`, built on the commit `tip`, is the target of; `branches` are the
/// repository's.
pub(super) fn target_view(source: &Source, tip: &CommitId, branches: &[BranchInfo]) -> TargetView {
    match source {
        Source::Branch { name } => TargetView {
            source: source.clone(),
            name: name.clone(),
            branch: Some(name.clone()),
            head: Some(tip.0.clone()),
        },
        Source::PullRequest { number, .. } => TargetView {
            source: source.clone(),
            name: format!("#{number}"),
            branch: None,
            head: Some(tip.0.clone()),
        },
        Source::WorkingTree => TargetView {
            source: source.clone(),
            name: "Working tree".into(),
            branch: branches.iter().find(|b| b.current).map(|b| b.name.clone()),
            head: None,
        },
    }
}

/// How many files of a group the manifest lists; the rest are counted.
const EXCLUDED_LISTED: usize = 500;

/// The files of `changes` that the review leaves out, by the reason, with the lines each changes.
pub(super) fn excluded_groups(changes: &Diffed, st: &State) -> Vec<ExcludedGroup> {
    let narrowed = st.filter.compile(&changes.diff.generated);
    let project = st.project.as_ref().map(|p| p.compile(&changes.diff.generated));
    // Why the review leaves a file out, given the rules that the filter in force and the project's
    // name for it: the project's own rule wins over the narrowing.
    let why = |named: Option<Exclusion>, project_named: Option<Exclusion>| {
        if project.is_none() {
            named.map(ExcludedBy::Rule)
        } else {
            project_named
                .map(ExcludedBy::Rule)
                .or(named.map(|_| ExcludedBy::Narrowing))
        }
    };
    let mut by_path: BTreeMap<&RepoPath, (ExcludedBy, LineDelta)> = BTreeMap::new();
    for d in &changes.diff.files {
        let project_named = project.as_ref().and_then(|p| p.exclusion(d.path()));
        if let Some(by) = why(narrowed.exclusion(d.path()), project_named) {
            by_path.insert(d.path(), (by, LineDelta::of(&d.hunks)));
        }
    }
    for n in &changes.diff.non_text {
        let project_named = project.as_ref().and_then(|p| p.exclusion_of(n));
        if let Some(by) = why(narrowed.exclusion_of(n), project_named) {
            by_path.entry(&n.path).or_insert((by, LineDelta::default()));
        }
    }
    let mut groups: BTreeMap<ExcludedBy, ExcludedGroup> = BTreeMap::new();
    for (path, (by, delta)) in by_path {
        let group = groups.entry(by.clone()).or_insert_with(|| ExcludedGroup {
            by,
            count: 0,
            delta: LineDelta::default(),
            files: Vec::new(),
        });
        group.count += 1;
        group.delta = [group.delta, delta].into_iter().sum();
        if group.files.len() < EXCLUDED_LISTED {
            group.files.push(ExcludedFile {
                path: path.clone(),
                delta,
            });
        }
    }
    groups.into_values().collect()
}

/// The files of `changes` the filter shows. `state` gives a file's review state and completion
/// from its text diff and its shown non-text changes.
pub(super) fn file_views(
    changes: &Diffed,
    filter: &ReviewFilter,
    state: impl Fn(Option<&FileDiff>, &[NonTextChange]) -> (ReviewView, Completion),
) -> Vec<FileView> {
    let compiled = filter.compile(&changes.diff.generated);
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
        let (review, completion) = state(diff, &items);
        files.push(FileView {
            path: path.clone(),
            old_path: old_path_of(path, diff, &items),
            status: status_of(diff, &items),
            base_lines: lines(diff, diff.and_then(|d| d.old_blob)),
            target_lines: lines(diff, diff.and_then(|d| d.new_blob)),
            delta: LineDelta::of(&hunks),
            hunks,
            review,
            completion,
            non_text: indices.iter().map(|&i| changes.items[i].clone()).collect(),
        });
    }
    files
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

pub(super) fn review_of(
    cov: &Coverage,
    diff: Option<&FileDiff>,
    items: &[NonTextChange],
    acked: &[NonTextChange],
) -> ReviewView {
    ReviewView {
        hunks: diff.map(|d| cov.hunk_states(d)).unwrap_or_default(),
        old_base_lines: diff.map(|d| cov.old_base_lines(d)).unwrap_or_default(),
        flagged: diff.map(|d| cov.hunk_flags(d)).unwrap_or_default(),
        acknowledged: items.iter().filter(|n| acked.contains(n)).map(|n| n.kind).collect(),
    }
}

/// The review state of one file and the changeset's completion.
pub(super) fn update_of(st: &State, path: &str) -> Result<ReviewUpdateView> {
    let changes = st.changes();
    update_from(st, &changes.diffed.diff, &changes.remapped, path)
}

/// The review state of the file at `path` of `whole`, read from `remapped`, and the completion of
/// `whole`.
pub(super) fn update_from(
    st: &State,
    whole: &SnapshotDiff,
    remapped: &Remapped,
    path: &str,
) -> Result<ReviewUpdateView> {
    let diff = find_diff(whole, path);
    let items = shown_items(whole, &st.filter, path);
    let file = diff
        .map(|d| d.path().clone())
        .or_else(|| items.first().map(|n| n.path.clone()))
        .ok_or_else(|| SessionError::NotFound(format!("file {path}")))?;
    let cov = remapped.coverage_of(diff.into_iter().flat_map(|d| d.old_path.iter().chain(&d.new_path)));
    let acked = acknowledged(st, &items);
    Ok(ReviewUpdateView {
        revision: st.revision,
        seq: st.seq,
        path: file,
        review: review_of(&cov, diff, &items, &acked),
        completion: file_completion(&cov, &st.filter.compile(&whole.generated), diff, &items, &acked),
        changeset: remapped.completion,
    })
}

/// The completion of one file: its text diff, if it has one, and its shown non-text items.
pub(super) fn file_completion(
    cov: &Coverage,
    filter: &CompiledFilter,
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
    cov.completion(&filter.compile(&diff.generated), &diff.files, &diff.non_text, acked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::RemoteKey;

    fn branch(name: &str, remote: bool, commit: &str, current: bool) -> BranchInfo {
        BranchInfo {
            name: name.into(),
            remote,
            commit: CommitId(commit.into()),
            current,
            default: false,
        }
    }

    #[test]
    fn a_branch_or_pull_request_target_carries_its_tips_full_id_and_the_working_tree_none() {
        let branches = [
            branch("main", false, "aaaaaaaaaaaaaaaa", true),
            branch("origin/feature", true, "bbbbbbbbbbbbbbbb", false),
        ];
        let tip = CommitId("cccccccccccccccc".into());
        let remote = Source::Branch {
            name: "origin/feature".into(),
        };
        assert_eq!(
            target_view(&remote, &tip, &branches).head.as_deref(),
            Some("cccccccccccccccc")
        );
        let pull_request = Source::PullRequest {
            remote: RemoteKey {
                host: "github.com".into(),
                path: "acme/app".into(),
            },
            number: 7,
        };
        assert_eq!(
            target_view(&pull_request, &tip, &branches).head.as_deref(),
            Some("cccccccccccccccc")
        );
        let tree = target_view(&Source::WorkingTree, &tip, &branches);
        assert_eq!(tree.head, None);
        assert_eq!(tree.branch.as_deref(), Some("main"));
    }
}
