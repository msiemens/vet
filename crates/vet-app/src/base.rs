//! What a base is called, how far its branch has moved on, and the default (§6.5).

use crate::changeset::{BaseChoice, CommitId};
use crate::ports::{BranchInfo, GitError, History};
use crate::pull_request::PullRequestRefs;

/// First-parent steps searched for a branch-relative name.
const MAX_RELATIVE: usize = 1000;
/// Commits counted for "behind".
const MAX_BEHIND: usize = 10_000;

/// The current branch's tip, or `head` itself when none is checked out. Against the checked-out
/// branch the merge base with `HEAD` is its tip.
pub fn default_choice(branches: &[BranchInfo], head: &CommitId) -> (BaseChoice, CommitId) {
    match branches.iter().find(|b| b.current) {
        Some(b) => (
            BaseChoice::MergeBase {
                reference: b.name.clone(),
            },
            b.commit.clone(),
        ),
        None => (BaseChoice::Commit { id: head.clone() }, head.clone()),
    }
}

/// What a branch is compared against unless the viewer chose otherwise: where it left the
/// repository's default branch, or `tip` itself (an empty changeset) when there is no such branch.
pub async fn default_for_branch<G: History>(git: &G, tip: &CommitId) -> Result<(BaseChoice, CommitId), String> {
    match git.default_reference().await.map_err(|e| e.0)? {
        Some(reference) => resolve(git, BaseChoice::MergeBase { reference }, tip).await,
        None => Ok((BaseChoice::Commit { id: tip.clone() }, tip.clone())),
    }
}

/// What a pull request is compared against unless the viewer chose otherwise: where its head left
/// the target branch as it was fetched.
pub async fn default_for_pull_request<G: History>(
    git: &G,
    refs: &PullRequestRefs,
    tip: &CommitId,
) -> Result<(BaseChoice, CommitId), String> {
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
) -> Result<(BaseChoice, CommitId), String> {
    match choice {
        BaseChoice::MergeBase { reference } => {
            let commit = git.merge_base(&reference, tip).await.map_err(|e| e.0)?;
            Ok((BaseChoice::MergeBase { reference }, commit))
        }
        BaseChoice::Commit { id } => {
            let commit = git.resolve_commit(&id.0).await.map_err(|e| e.0)?;
            Ok((BaseChoice::Commit { id: commit.clone() }, commit))
        }
    }
}

/// The name a person knows `commit` by: a branch whose tip it is, else
/// `branch~N` on a branch's first-parent history, else its short hash. Branches
/// are tried in this order: `prefer`, the current one, the default one, the rest.
pub async fn name_of<G: History>(
    git: &G,
    branches: &[BranchInfo],
    prefer: Option<&str>,
    commit: &CommitId,
) -> Result<String, GitError> {
    let mut order: Vec<&BranchInfo> = branches.iter().collect();
    order.sort_by_key(|b| (Some(b.name.as_str()) != prefer, !b.current, !b.default));
    if let Some(tip) = order.iter().find(|b| b.commit == *commit) {
        return Ok(tip.name.clone());
    }
    for b in order {
        if let Some(steps) = git.first_parent_distance(&b.commit, commit, MAX_RELATIVE).await? {
            return Ok(format!("{}~{steps}", b.name));
        }
    }
    Ok(commit.short().to_owned())
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

/// How many commits the chosen branch has gained since `base`, on its own or on
/// the remote as of the last fetch; `None` unless the choice is a merge base
/// against a branch and it has moved.
pub async fn behind<G: History>(
    git: &G,
    branches: &[BranchInfo],
    choice: &BaseChoice,
    base: &CommitId,
) -> Result<Option<u32>, GitError> {
    let BaseChoice::MergeBase { reference } = choice else {
        return Ok(None);
    };
    let Some(branch) = branches.iter().find(|b| b.name == *reference) else {
        return Ok(None);
    };
    let mut count = git.commits_behind(&branch.commit, base, MAX_BEHIND).await?;
    if let Some(upstream) = &branch.upstream {
        count = count.max(git.commits_behind(upstream, base, MAX_BEHIND).await?);
    }
    Ok((count > 0).then_some(count as u32))
}
