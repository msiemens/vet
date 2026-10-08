//! What Vet takes from Azure DevOps' pull requests, threads, statuses and iterations.

use std::collections::HashMap;

use azure_devops_rust_api::git::models::{
    GitPullRequest, GitPullRequestCommentThread, GitPullRequestIteration, GitPullRequestStatus, IdentityRef,
    comment::CommentType, comment_thread::Status as ThreadStatus, git_pull_request_iteration::Reason,
    git_status::State as StatusState,
};
use vet_app::platform::{CiState, Push, ReviewRequest, ReviewVerdict, ViewerReview};

/// The signed-in user, as the reviewers of a pull request name them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Me {
    /// How the rest of Vet names the user: the address, else the display name.
    pub login: String,
    /// The identity's id, which the pull requests carry.
    pub id: Option<String>,
    pub email: Option<String>,
    /// The groups the user is a member of, by display name.
    pub teams: Vec<String>,
}

impl Me {
    fn is(&self, identity: &IdentityRef) -> bool {
        let same = |a: &Option<String>, b: &str| a.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(b));
        same(&self.id, &identity.id) || identity.unique_name.as_deref().is_some_and(|n| same(&self.email, n))
    }

    /// The name the rest of Vet knows `identity` by.
    fn name(&self, identity: &IdentityRef) -> String {
        if self.is(identity) {
            return self.login.clone();
        }
        identity
            .unique_name
            .clone()
            .or_else(|| identity.graph_subject_base.display_name.clone())
            .unwrap_or_else(|| identity.id.clone())
    }
}

/// `refs/heads/main` is the branch `main`.
fn branch(reference: &str) -> String {
    reference.strip_prefix("refs/heads/").unwrap_or(reference).to_owned()
}

/// ADO's votes: 10 approved, 5 approved with suggestions, 0 none, -5 waiting for the
/// author, -10 rejected.
fn verdict(vote: i64) -> Option<ReviewVerdict> {
    match vote {
        5.. => Some(ReviewVerdict::Approved),
        0 => None,
        _ => Some(ReviewVerdict::ChangesRequested),
    }
}

/// A pull request as the list shows it, without what takes a request of its own: checks
/// and threads. `None` for one without a source commit. `web` is the address the
/// number is appended to.
pub(super) fn pull_request(pr: &GitPullRequest, me: &Me, web: &str) -> Option<vet_app::platform::PullRequest> {
    let head = pr.last_merge_source_commit.as_ref()?.commit_id.clone()?;
    let mut review_requests = Vec::new();
    let mut viewer_review = None;
    for reviewer in &pr.reviewers {
        let identity = &reviewer.identity_ref;
        let vote = reviewer.vote.unwrap_or(0);
        if identity.is_container == Some(true) {
            if vote == 0 && reviewer.has_declined != Some(true) {
                let name = identity.graph_subject_base.display_name.clone();
                review_requests.extend(name.map(ReviewRequest::Team));
            }
        } else if me.is(identity) {
            // A vote carries no commit; it is taken to be on the head, which is what
            // the platform shows next to it.
            viewer_review = verdict(vote).map(|verdict| ViewerReview {
                verdict,
                commit: Some(head.clone()),
            });
            if vote == 0 && reviewer.has_declined != Some(true) {
                review_requests.push(ReviewRequest::User(me.login.clone()));
            }
        } else if vote == 0 && reviewer.has_declined != Some(true) {
            review_requests.push(ReviewRequest::User(me.name(identity)));
        }
    }
    Some(vet_app::platform::PullRequest {
        number: u64::try_from(pr.pull_request_id).ok()?,
        title: pr.title.clone().unwrap_or_default(),
        url: format!("{web}{}", pr.pull_request_id),
        author: me.name(&pr.created_by),
        source_branch: branch(&pr.source_ref_name),
        target_branch: branch(&pr.target_ref_name),
        fork: pr.fork_source.as_ref().map(|source| {
            source
                .repository
                .as_ref()
                .map_or_else(|| "a fork".to_owned(), |r| r.name.clone())
        }),
        head,
        // Neither the list nor the pull request says how many lines it changes.
        lines: None,
        // Counted from the threads, which take a request of their own.
        comments: 0,
        draft: pr.is_draft,
        labels: pr.labels.iter().filter_map(|l| l.name.clone()).collect(),
        // Azure DevOps has no assignees.
        assignees: Vec::new(),
        ci: None,
        unresolved_threads: 0,
        review_requests,
        viewer_review,
        // The list says when a pull request was created, not when it last changed.
        updated_at: pr.creation_date.unix_timestamp(),
    })
}

/// The rolled-up state of a pull request's statuses: the newest of each context counts,
/// as far as it is on the newest iteration that has statuses.
pub(super) fn ci_state(statuses: &[GitPullRequestStatus]) -> Option<CiState> {
    let newest_iteration = statuses.iter().filter_map(|s| s.iteration_id).max();
    let mut newest: HashMap<(Option<&str>, Option<&str>), &GitPullRequestStatus> = HashMap::new();
    for status in statuses
        .iter()
        .filter(|s| s.iteration_id.is_none() || s.iteration_id == newest_iteration)
    {
        let context = status.git_status.context.as_ref();
        let key = (
            context.and_then(|c| c.genre.as_deref()),
            context.and_then(|c| c.name.as_deref()),
        );
        let id = |s: &GitPullRequestStatus| s.git_status.id;
        if newest.get(&key).is_none_or(|seen| id(seen) <= id(status)) {
            newest.insert(key, status);
        }
    }
    let states: Vec<&StatusState> = newest.values().filter_map(|s| s.git_status.state.as_ref()).collect();
    let any = |wanted: &[StatusState]| states.iter().any(|s| wanted.contains(s));
    if any(&[StatusState::Failed, StatusState::Error]) {
        Some(CiState::Failure)
    } else if any(&[StatusState::Pending, StatusState::NotSet]) {
        Some(CiState::Pending)
    } else if any(&[StatusState::Succeeded]) {
        Some(CiState::Success)
    } else {
        None
    }
}

/// The threads that still ask for something: open, not deleted, and with a comment that
/// a person wrote.
pub(super) fn unresolved_threads(threads: &[GitPullRequestCommentThread]) -> u32 {
    let open = threads
        .iter()
        .map(|t| &t.comment_thread)
        .filter(|t| t.is_deleted != Some(true))
        .filter(|t| matches!(t.status, Some(ThreadStatus::Active | ThreadStatus::Pending)))
        .filter(|t| {
            t.comments
                .iter()
                .any(|c| c.is_deleted != Some(true) && !matches!(c.comment_type, Some(CommentType::System)))
        })
        .count();
    u32::try_from(open).unwrap_or(u32::MAX)
}

/// The comments people wrote on the pull request, in threads not deleted.
pub(super) fn comment_count(threads: &[GitPullRequestCommentThread]) -> u32 {
    let count = threads
        .iter()
        .map(|t| &t.comment_thread)
        .filter(|t| t.is_deleted != Some(true))
        .flat_map(|t| &t.comments)
        .filter(|c| c.is_deleted != Some(true) && !matches!(c.comment_type, Some(CommentType::System)))
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// The states of the source branch the iterations record, oldest first, ending in
/// `head`; and the merge base of the latest.
pub(super) fn pushes(iterations: &[GitPullRequestIteration], head: &str) -> (Option<String>, Vec<Push>) {
    let mut ordered: Vec<&GitPullRequestIteration> = iterations.iter().collect();
    ordered.sort_by_key(|i| i.id);
    let mut pushes: Vec<Push> = Vec::new();
    for iteration in &ordered {
        let Some(commit) = iteration
            .source_ref_commit
            .as_ref()
            .and_then(|c| c.commit_id.as_deref())
        else {
            continue;
        };
        let forced = iteration.reason == Some(Reason::ForcePush);
        match pushes.last_mut() {
            // A new iteration on the same commit (the target moved) adds no state.
            Some(last) if last.head == commit => last.forced |= forced,
            _ => pushes.push(Push {
                head: commit.to_owned(),
                at: iteration.created_date.map(|d| d.unix_timestamp()),
                forced,
            }),
        }
    }
    if pushes.last().is_none_or(|last| last.head != head) {
        pushes.push(Push {
            head: head.to_owned(),
            at: None,
            forced: false,
        });
    }
    let base = ordered
        .iter()
        .rev()
        .find_map(|i| i.common_ref_commit.as_ref()?.commit_id.clone());
    (base, pushes)
}
