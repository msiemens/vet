//! What each changeset is compared against (§6.5). A base is fixed by the kind of changeset and
//! never chosen: the working tree's is the `HEAD` it was opened at, a branch's is where it left
//! the repository's default branch, a pull request's where its head left the fetched target.

use crate::changeset::{BaseChoice, CommitId};
use crate::ports::{GitError, History};
use crate::pull_request::PullRequestRefs;

/// What the working tree is compared against: `pinned`, the `HEAD` it was opened at, wherever
/// `HEAD` moves later.
pub fn for_working_tree(pinned: &CommitId) -> (BaseChoice, CommitId) {
    (BaseChoice::Commit { id: pinned.clone() }, pinned.clone())
}

/// What a branch is compared against: where it left the repository's default branch, or `tip`
/// itself (an empty changeset) when there is no such branch.
pub async fn for_branch<G: History>(git: &G, tip: &CommitId) -> Result<(BaseChoice, CommitId), GitError> {
    match git.default_reference().await? {
        Some(reference) => resolve(git, BaseChoice::MergeBase { reference }, tip).await,
        None => Ok((BaseChoice::Commit { id: tip.clone() }, tip.clone())),
    }
}

/// What a pull request is compared against: where its head left the target branch as it was
/// fetched.
pub async fn for_pull_request<G: History>(
    git: &G,
    refs: &PullRequestRefs,
    tip: &CommitId,
) -> Result<(BaseChoice, CommitId), GitError> {
    resolve(
        git,
        BaseChoice::MergeBase {
            reference: refs.target.clone(),
        },
        tip,
    )
    .await
}

/// The commit `choice` names now, with the choice as stored: a commit is spelled out in full.
/// A merge base is taken against `tip`, the commit the target is built on. The error is fit to show.
pub async fn resolve<G: History>(
    git: &G,
    choice: BaseChoice,
    tip: &CommitId,
) -> Result<(BaseChoice, CommitId), GitError> {
    match choice {
        BaseChoice::MergeBase { reference } => {
            let commit = git.merge_base(&reference, tip).await?;
            Ok((BaseChoice::MergeBase { reference }, commit))
        }
        BaseChoice::Commit { id } => {
            let commit = git
                .resolve_commit(&id.0)
                .await?
                .ok_or_else(|| GitError::UnknownRevision(format!("unknown commit {}", id.0)))?;
            Ok((BaseChoice::Commit { id: commit.clone() }, commit))
        }
    }
}

/// The reference a changeset whose tip the base reached is already contained in: set when
/// the choice is a merge base that is `tip` itself, and the reference is not the changeset's own
/// branch `own`. Such a changeset has nothing to review.
pub fn contained_in(choice: &BaseChoice, base: &CommitId, tip: &CommitId, own: Option<&str>) -> Option<String> {
    match choice {
        BaseChoice::MergeBase { reference } if base == tip && own != Some(reference.as_str()) => {
            Some(reference.clone())
        }
        _ => None,
    }
}

/// A reference as people name the branch: `refs/heads/main` is `main`, `refs/remotes/origin/main`
/// is `origin/main`.
pub fn short_reference(reference: &str) -> String {
    let name = reference
        .strip_prefix("refs/heads/")
        .or_else(|| reference.strip_prefix("refs/remotes/"))
        .unwrap_or(reference);
    name.to_owned()
}

/// What a reference is called as a branch: a local one by its name, a remote-tracking one with its
/// remote, and `origin/HEAD` as the `default_branch` it points at.
pub fn reference_name(reference: &str, default_branch: Option<&str>) -> String {
    match (reference.strip_prefix("refs/remotes/"), default_branch) {
        (Some(remote), Some(branch)) if remote.ends_with("/HEAD") => {
            format!("{}/{branch}", remote.trim_end_matches("/HEAD"))
        }
        _ => short_reference(reference),
    }
}

/// The commit that brought a tip into a branch, and where the branch was before.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Merging {
    pub commit: CommitId,
    /// The branch just before the commit: its first parent.
    pub before: Option<CommitId>,
}

/// The commit that brought `tip` into the first-parent history of `into`: the oldest commit of it
/// that has `tip` among its ancestors. `None` when `into` merely advanced past `tip` (the commit
/// after `tip` is its child, a fast-forward) or when `into` does not contain `tip`.
pub async fn merging_commit<G: History>(git: &G, tip: &CommitId, into: &CommitId) -> Result<Option<Merging>, GitError> {
    // Along the history, `tip` is reachable from a commit exactly when it is not older than the merge.
    let history = git.first_parent_range(tip, into).await?;
    let (mut low, mut high) = (0, history.len());
    while low < high {
        let mid = (low + high) / 2;
        let reaches = git.commits_behind(tip, &history[mid], 1).await? == 0;
        if reaches {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    let Some(merge) = history.get(low) else {
        return Ok(None);
    };
    let detail = git.commit_detail(merge).await?;
    Ok((detail.parents.first() != Some(tip)).then(|| Merging {
        commit: merge.clone(),
        before: detail.parents.first().cloned(),
    }))
}

/// Whether `HEAD` no longer builds on `base`: `base` is neither `HEAD` nor one of its ancestors.
/// A `HEAD` that cannot be compared with it has not been seen to leave.
pub async fn head_left<G: History>(git: &G, base: &CommitId) -> bool {
    let Ok(head) = git.head().await else {
        return false;
    };
    // The commits only `base` reaches: none when `head` has them all.
    git.commits_behind(base, &head, 1)
        .await
        .is_ok_and(|only_in_base| only_in_base > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_is_named_as_the_branch_it_stands_for() {
        assert_eq!(reference_name("refs/heads/main", Some("main")), "main");
        assert_eq!(reference_name("refs/remotes/origin/main", Some("main")), "origin/main");
        assert_eq!(
            reference_name("refs/remotes/origin/HEAD", Some("trunk")),
            "origin/trunk"
        );
        assert_eq!(reference_name("refs/remotes/origin/HEAD", None), "origin/HEAD");
    }
}
