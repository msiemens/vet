//! Base resolution (§6.5): merge bases, commit lookup, and the branches and
//! recent commits a base can be picked from.

use gix::bstr::ByteSlice;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;
use vet_app::pin::CommitId;
use vet_app::ports::{BaseCandidates, BranchInfo, CommitInfo, GitError};

use super::err;

fn commit_of<'r>(repo: &'r gix::Repository, spec: &str) -> Option<gix::Commit<'r>> {
    let id = repo.rev_parse_single(spec).ok()?;
    id.object().ok()?.peel_to_commit().ok()
}

pub(super) fn resolve_commit(repo: &gix::Repository, spec: &str) -> Result<CommitId, GitError> {
    commit_of(repo, spec)
        .map(|c| CommitId(c.id.to_string()))
        .ok_or_else(|| GitError(format!("unknown commit {spec}")))
}

pub(super) fn merge_base(repo: &gix::Repository, reference: &str) -> Result<CommitId, GitError> {
    let other =
        commit_of(repo, reference).ok_or_else(|| GitError(format!("unknown branch or revision {reference}")))?;
    let head = repo.head_id().map_err(err)?;
    let base = repo
        .merge_base(other.id, head)
        .map_err(|_| GitError(format!("no common ancestor with {reference}")))?;
    Ok(CommitId(base.to_string()))
}

pub(super) fn candidates(repo: &gix::Repository, max_commits: usize) -> Result<BaseCandidates, GitError> {
    let current = repo.head_name().map_err(err)?;
    let default = repo
        .try_find_reference("refs/remotes/origin/HEAD")
        .map_err(err)?
        .and_then(|r| {
            let target = r.target().try_name()?.as_bstr().to_str().ok()?.to_owned();
            target.strip_prefix("refs/remotes/origin/").map(str::to_owned)
        });

    let mut branches = Vec::new();
    for reference in repo.references().map_err(err)?.local_branches().map_err(err)? {
        let mut reference = reference.map_err(err)?;
        let name = reference.name().shorten().to_string();
        let commit = reference.peel_to_id().map_err(err)?;
        branches.push(BranchInfo {
            current: current.as_ref().is_some_and(|c| c.as_ref() == reference.name()),
            default: default.as_deref() == Some(name.as_str()),
            name,
            commit: CommitId(commit.to_string()),
        });
    }
    branches.sort_by(|a, b| a.name.cmp(&b.name));

    let mut commits = Vec::new();
    let head = repo.head_id().map_err(err)?;
    let walk = repo
        .rev_walk([head.detach()])
        .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
        .all()
        .map_err(err)?;
    for info in walk.take(max_commits) {
        let id = info.map_err(err)?.id;
        let commit = repo.find_commit(id).map_err(err)?;
        let author = commit.author().map_err(err)?;
        commits.push(CommitInfo {
            id: CommitId(id.to_string()),
            subject: commit.message().map_err(err)?.summary().to_string(),
            author: author.name.to_string(),
            time: author.time().map_err(err)?.seconds,
        });
    }
    Ok(BaseCandidates { branches, commits })
}
