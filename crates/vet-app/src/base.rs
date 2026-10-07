//! What a base is called, how far its branch has moved on, and the default (§6.5).

use crate::changeset::{BaseChoice, CommitId};
use crate::ports::{BranchInfo, GitError, History};

/// First-parent steps searched for a branch-relative name.
const MAX_RELATIVE: usize = 1000;
/// Commits counted for "behind".
const MAX_BEHIND: usize = 10_000;

/// The current branch's tip, or `head` itself when none is checked out.
pub fn default_choice(branches: &[BranchInfo], head: &CommitId) -> (BaseChoice, CommitId) {
    match branches.iter().find(|b| b.current) {
        Some(b) => (BaseChoice::Branch { name: b.name.clone() }, b.commit.clone()),
        None => (BaseChoice::Commit { id: head.clone() }, head.clone()),
    }
}

/// The commit `choice` names now, with the choice as stored: a commit is spelled out in full.
/// The error is fit to show.
pub async fn resolve<G: History>(git: &G, choice: BaseChoice) -> Result<(BaseChoice, CommitId), String> {
    match choice {
        BaseChoice::Branch { name } => {
            let commit = git
                .branch_tip(&name)
                .await
                .map_err(|_| format!("unknown branch {name}"))?;
            Ok((BaseChoice::Branch { name }, commit))
        }
        BaseChoice::MergeBase { reference } => {
            let commit = git.merge_base(&reference).await.map_err(|e| e.0)?;
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

/// How many commits the chosen branch has gained since `base`; `None` unless
/// the choice is a branch and it has moved.
pub async fn behind<G: History>(
    git: &G,
    branches: &[BranchInfo],
    choice: &BaseChoice,
    base: &CommitId,
) -> Result<Option<u32>, GitError> {
    let BaseChoice::Branch { name } = choice else {
        return Ok(None);
    };
    let Some(branch) = branches.iter().find(|b| b.name == *name) else {
        return Ok(None);
    };
    let count = git.commits_behind(&branch.commit, base, MAX_BEHIND).await?;
    Ok((count > 0).then_some(count as u32))
}
