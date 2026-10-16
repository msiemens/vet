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
    ExcludedBy, ExcludedFile, ExcludedFolder, ExcludedGroup, FileStatus, FileView, Lifecycle, ManifestView,
    ReviewUpdateView, ReviewView, TargetView, ViewKind, empty_reason,
};
use super::{LiveSession, Result, SessionError};
use crate::changeset::{CommitId, Source};
use crate::editor::line_count;
use crate::ports::{BranchInfo, GitRepo, LiveStore, ReviewWrites};
use crate::review::ReviewSummary;

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    pub async fn manifest(&self, view: View) -> Result<ManifestView> {
        let (st, shown) = self.viewing(view).await?;
        let manifest = match &shown {
            Shown::Interdiff(id) => self.interdiff_manifest(&st, id).await?,
            Shown::Commit { index } => self.commit_manifest(&st, *index).await?,
            Shown::Changeset => self.changeset_manifest(&st).await?,
        };
        self.headed(&st, manifest).await
    }

    /// The manifest of the changeset on view itself, whatever view some of its facts
    /// (`changeset_base` and friends) speak of beside it.
    async fn changeset_manifest(&self, st: &State) -> Result<ManifestView> {
        let branches = self.acquisition.git.branches().await?;
        let facts = self.lens_facts(st, &branches).await?;
        let changes = st.changes();
        let cov = changes.remapped.coverage();
        let acked = acknowledged(st, &changes.diffed.diff.non_text);
        let compiled = st.filter.compile(&changes.diffed.diff.generated);
        let files = file_views(&changes.diffed, &st.filter, |diff, items| {
            (
                review_of(&cov, diff, items, &acked),
                file_completion(&cov, &compiled, diff, items, &acked),
            )
        });
        let excluded = excluded_groups(&changes.diffed, st);
        // A merged pull request is one the platform says so of, as the shell's wording needs.
        let merged = matches!(
            self.discussion().await.map(|d| d.discussion.state),
            Some(crate::platform::PullRequestState::Merged)
        );
        let empty = empty_reason(
            !excluded.is_empty(),
            ViewKind::Changeset,
            facts.base.contained_in.as_ref().map(|c| (c.same_tip, merged)),
            matches!(st.source, Source::WorkingTree),
            match lifecycle_of(st) {
                Lifecycle::Open { version, .. } => Some(version),
                Lifecycle::Browsing => None,
            },
        );
        Ok(ManifestView {
            revision: st.revision,
            seq: st.seq,
            name: self.name.clone(),
            lifecycle: lifecycle_of(st),
            base: facts.base.clone(),
            target: target_view(&st.source, &st.head, &branches),
            filter: st.project_filter().clone(),
            narrowing: st.narrowing(),
            completion: changes.remapped.completion,
            delta: files.iter().map(|f| f.delta).sum(),
            excluded,
            files,
            degraded: self.degraded_for(&st.source),
            interdiff: None,
            lens: facts.lens,
            commits: facts.commits,
            changeset_base: facts.base,
            changeset_files: facts.files,
            changeset_completion: facts.completion,
            empty,
        })
    }

    /// Names the target of a pull request after the platform, with what it last said of it: its
    /// title and whether it is a draft. A view of a pull request, also a restored one, is headed
    /// without the platform being asked; a changeset of another kind says nothing of a platform.
    async fn headed(&self, st: &State, mut manifest: ManifestView) -> Result<ManifestView> {
        if let Source::PullRequest { .. } = &st.source
            && let Some(head) = self.pull_request_head(&st.source).await?
        {
            manifest.target.title = Some(head.title);
            manifest.target.draft = head.draft;
        }
        Ok(manifest)
    }
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// Stores how far the open review is when it differs from what is stored, so that the list of
    /// changesets shows it without remapping the marks of every review. Does nothing while the
    /// changes are not computed: the next remap measures. What cannot be stored is only logged:
    /// the summary is a cache of what the review shows, and the command it follows has been done.
    pub(super) async fn note_progress(&self, writes: &S::Writes, st: &mut State) {
        let (Some(review), Some(_)) = (&st.review, &st.changes) else {
            return;
        };
        let summary = ReviewSummary::of(&stored_completion(st));
        if review.summary == Some(summary) {
            return;
        }
        let id = review.id;
        match writes.set_review_summary(id, summary).await {
            Ok(()) => {
                if let Some(review) = &mut st.review {
                    review.summary = Some(summary);
                }
            }
            Err(e) => tracing::warn!(error = %e.0, "could not store how far the review is"),
        }
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
            title: None,
            draft: false,
        },
        Source::PullRequest { number, .. } => TargetView {
            source: source.clone(),
            name: format!("#{number}"),
            branch: None,
            head: Some(tip.0.clone()),
            title: None,
            draft: false,
        },
        Source::WorkingTree => TargetView {
            source: source.clone(),
            name: "Working tree".into(),
            branch: branches.iter().find(|b| b.current).map(|b| b.name.clone()),
            head: None,
            title: None,
            draft: false,
        },
    }
}

/// How many files of a group the manifest lists; the rest are counted.
const EXCLUDED_LISTED: usize = 500;

/// How many folders of a group the manifest lists.
const EXCLUDED_FOLDERS: usize = 200;

/// The files of `changes` that the review leaves out, grouped by every rule that does so (a file two
/// rules leave out is in one group with both, so a group's rules are what to lift to bring it back),
/// the largest group first.
pub(super) fn excluded_groups(changes: &Diffed, st: &State) -> Vec<ExcludedGroup> {
    let narrowed = st.filter.compile(&changes.diff.generated);
    let project = st.project.as_ref().map(|p| p.compile(&changes.diff.generated));
    // What leaves a file out, given the rules of the filter in force and of the project's: the
    // project's own rules that the narrowing still applies, else the narrowing.
    let why = |named: Vec<Exclusion>, project_named: Vec<Exclusion>| match project {
        None => named.into_iter().map(ExcludedBy::Rule).collect(),
        Some(_) => {
            let kept: Vec<_> = project_named
                .into_iter()
                .filter(|rule| named.contains(rule))
                .map(ExcludedBy::Rule)
                .collect();
            if kept.is_empty() {
                vec![ExcludedBy::Narrowing]
            } else {
                kept
            }
        }
    };
    let mut by_path: BTreeMap<&RepoPath, (Vec<ExcludedBy>, LineDelta)> = BTreeMap::new();
    for d in &changes.diff.files {
        if narrowed.passes(d.path()) {
            continue;
        }
        let project_named = project.as_ref().map_or_else(Vec::new, |p| p.exclusions(d.path()));
        let by = why(narrowed.exclusions(d.path()), project_named);
        by_path.insert(d.path(), (by, LineDelta::of(&d.hunks)));
    }
    for n in &changes.diff.non_text {
        if narrowed.passes(&n.path) && narrowed.shows(n) {
            continue;
        }
        let project_named = project.as_ref().map_or_else(Vec::new, |p| p.exclusions_of(n));
        let by = why(narrowed.exclusions_of(n), project_named);
        if !by.is_empty() {
            by_path.entry(&n.path).or_insert((by, LineDelta::default()));
        }
    }
    // The folders at the depth of the narrowing, where it is the narrowing that leaves files out.
    let depth = (st
        .narrowing()
        .and_then(|n| n.roots.first().map(|r| r.0.split(|&b| b == b'/').count())))
    .unwrap_or(1);
    let mut groups: BTreeMap<Vec<ExcludedBy>, (ExcludedGroup, BTreeMap<String, ExcludedFolder>)> = BTreeMap::new();
    for (path, (by, delta)) in by_path {
        let (group, folders) = groups.entry(by.clone()).or_insert_with(|| {
            (
                ExcludedGroup {
                    by,
                    count: 0,
                    delta: LineDelta::default(),
                    files: Vec::new(),
                    folders: Vec::new(),
                },
                BTreeMap::new(),
            )
        });
        group.count += 1;
        group.delta = [group.delta, delta].into_iter().sum();
        if group.files.len() < EXCLUDED_LISTED {
            group.files.push(ExcludedFile {
                path: path.clone(),
                delta,
            });
        }
        let text = path.to_string_lossy();
        let segments = text.split('/').count();
        let (key, dir) = if segments == 1 {
            (text.clone(), false)
        } else {
            (
                text.split('/')
                    .take(depth.min(segments - 1))
                    .collect::<Vec<_>>()
                    .join("/"),
                true,
            )
        };
        let folder = folders.entry(key.clone()).or_insert_with(|| ExcludedFolder {
            path: key,
            dir,
            files: 0,
            delta: LineDelta::default(),
        });
        folder.files += 1;
        folder.delta = [folder.delta, delta].into_iter().sum();
    }
    let mut groups: Vec<ExcludedGroup> = groups
        .into_values()
        .map(|(mut group, folders)| {
            let mut folders: Vec<ExcludedFolder> = folders.into_values().collect();
            folders.sort_by(|a, b| b.files.cmp(&a.files).then_with(|| a.path.cmp(&b.path)));
            folders.truncate(EXCLUDED_FOLDERS);
            group.folders = folders;
            group
        })
        .collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.count));
    groups
}

/// How far the changeset is under the project's filter, which is what is stored of it: a
/// narrowing of the review does not change that.
pub(super) fn stored_completion(st: &State) -> Completion {
    let changes = st.changes();
    if st.project.is_none() {
        return changes.remapped.completion;
    }
    let diff = &changes.diffed.diff;
    changeset_completion(
        &changes.remapped.coverage(),
        st.project_filter(),
        diff,
        &acknowledged(st, &diff.non_text),
    )
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
