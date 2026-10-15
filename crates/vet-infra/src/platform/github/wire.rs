//! GitHub's GraphQL and REST documents, and what Vet takes from them.

use serde::Deserialize;
use vet_app::platform::{
    ChangesAsked, ChangesRequest, CiState, ForcePush, PlatformError, PullRequest, PullRequestDetail, ReviewDecision,
    ReviewRequest, ReviewVerdict, ViewerReview,
};
use vet_core::content::LineDelta;
use vet_core::num::sat_u32;

use crate::platform::time::parse_rfc3339;

/// The fields of a pull request that a queue row needs at once: what its status, checks and
/// review state say, and what the tabs and filters select by. One request per page asks for these
/// for every pull request on it, and a large repository's page has to be answered within GitHub's
/// ten seconds, so nothing that costs much per pull request is here: the review decision, the
/// merge state (GitHub works it out for each pull request asked), the review threads and who
/// asked for changes are read per pull request ([`detail_query`], and the discussion once it is
/// read). Each connection counts once per pull request against the rate limit (4 of them:
/// labels, assignees, review requests, commits).
const PULL_REQUEST_FIELDS: &str = "
fragment PullRequestFields on PullRequest {
  number
  title
  url
  isDraft
  updatedAt
  headRefName
  baseRefName
  headRefOid
  additions
  deletions
  totalCommentsCount
  isCrossRepository
  headRepository { nameWithOwner }
  author { login avatarUrl ... on User { name } }
  labels(first: 20) { nodes { name } }
  assignees(first: 10) { nodes { login } }
  reviewRequests(first: 20) {
    nodes {
      requestedReviewer {
        __typename
        ... on User { login name }
        ... on Mannequin { login }
        ... on Team { slug organization { login } }
      }
    }
  }
  viewerLatestReview { state commit { oid } }
  commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
}";

pub(super) fn search_query() -> String {
    format!(
        "query($q: String!, $first: Int!, $after: String) {{
  search(query: $q, type: ISSUE, first: $first, after: $after) {{
    nodes {{ ...PullRequestFields }}
    pageInfo {{ hasNextPage endCursor }}
  }}
}}
{PULL_REQUEST_FIELDS}"
    )
}

/// One pull request with everything the listing leaves out: it is asked for alone, so it can afford
/// the review threads and the reviewers' verdicts.
pub(super) fn detail_query() -> String {
    format!(
        "query($owner: String!, $name: String!, $number: Int!, $after: String) {{
  repository(owner: $owner, name: $name) {{
    pullRequest(number: $number) {{
      ...PullRequestFields
      reviewDecision
      mergeStateStatus
      latestOpinionatedReviews(first: 20, writersOnly: true) {{ nodes {{ state author {{ login ... on User {{ name }} }} }} }}
      reviewThreads(first: 100) {{ nodes {{ isResolved }} }}
      baseRefOid
      timelineItems(itemTypes: [HEAD_REF_FORCE_PUSHED_EVENT], first: 100, after: $after) {{
        nodes {{
          ... on HeadRefForcePushedEvent {{
            createdAt
            beforeCommit {{ oid }}
            afterCommit {{ oid }}
          }}
        }}
        pageInfo {{ hasNextPage endCursor }}
      }}
    }}
  }}
}}
{PULL_REQUEST_FIELDS}"
    )
}

#[derive(Deserialize)]
pub(super) struct PageInfo {
    #[serde(rename = "hasNextPage")]
    pub has_next_page: bool,
    #[serde(rename = "endCursor")]
    pub end_cursor: Option<String>,
}

/// A connection: one page of nodes, and where the next one begins.
#[derive(Deserialize)]
pub(super) struct Page<T> {
    pub nodes: Vec<Option<T>>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

#[derive(Deserialize)]
pub(super) struct BackwardsInfo {
    #[serde(rename = "hasPreviousPage")]
    pub has_previous_page: bool,
    #[serde(rename = "startCursor")]
    pub start_cursor: Option<String>,
}

/// A connection read from its end (`last`): one page of nodes, and where the page before it
/// ends.
#[derive(Deserialize)]
pub(super) struct BackwardsPage<T> {
    pub nodes: Vec<Option<T>>,
    #[serde(rename = "pageInfo")]
    pub page_info: BackwardsInfo,
}

#[derive(Deserialize)]
pub(super) struct SearchData {
    pub search: Page<PullRequestNode>,
}

/// The answer to a query for one pull request of a repository.
#[derive(Deserialize)]
pub(super) struct RepositoryData<T> {
    pub repository: Option<RepositoryNode<T>>,
}

#[derive(Deserialize)]
pub(super) struct RepositoryNode<T> {
    #[serde(rename = "pullRequest")]
    pub pull_request: Option<T>,
}

impl<T> RepositoryData<T> {
    pub(super) fn pull_request(self) -> Option<T> {
        self.repository.and_then(|r| r.pull_request)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DetailNode {
    #[serde(flatten)]
    pub pull_request: PullRequestNode,
    /// Null when no review is required or none was given.
    review_decision: Option<String>,
    merge_state_status: Option<String>,
    latest_opinionated_reviews: Nodes<OpinionNode>,
    review_threads: Nodes<ThreadNode>,
    pub base_ref_oid: String,
    pub timeline_items: Page<ForcePushNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ForcePushNode {
    pub created_at: String,
    pub before_commit: Option<Oid>,
    pub after_commit: Option<Oid>,
}

#[derive(Deserialize)]
pub(super) struct Oid {
    pub oid: String,
}

#[derive(Deserialize)]
pub(super) struct Nodes<T> {
    nodes: Vec<Option<T>>,
}

impl<T> Nodes<T> {
    pub(super) fn present(self) -> impl Iterator<Item = T> {
        self.nodes.into_iter().flatten()
    }
}

#[derive(Deserialize)]
pub(super) struct Login {
    pub login: String,
}

/// An account with its picture, and the profile name that only a user has (a bot or a mannequin
/// has none).
#[derive(Deserialize)]
pub(super) struct Author {
    pub login: String,
    /// Null or empty for a user who has not set a name.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "avatarUrl", default)]
    pub avatar_url: Option<String>,
}

impl Author {
    pub(super) fn display(&self) -> String {
        display_name(self.name.as_deref(), &self.login)
    }
}

/// What a person is called when shown: the profile name, else the login.
pub(super) fn display_name(name: Option<&str>, login: &str) -> String {
    name.filter(|n| !n.trim().is_empty()).unwrap_or(login).to_owned()
}

#[derive(Deserialize)]
pub(super) struct Name {
    pub name: String,
}

#[derive(Deserialize)]
struct HeadRepository {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
}

#[derive(Deserialize)]
pub(super) struct RequestNode {
    #[serde(rename = "requestedReviewer")]
    requested_reviewer: Option<Reviewer>,
}

#[derive(Deserialize)]
struct Reviewer {
    #[serde(rename = "__typename")]
    type_name: String,
    login: Option<String>,
    /// Only a user has one.
    name: Option<String>,
    slug: Option<String>,
    organization: Option<Login>,
}

#[derive(Deserialize)]
struct ReviewNode {
    state: String,
    commit: Option<Oid>,
}

/// A reviewer's latest review that approves or requests changes.
#[derive(Deserialize)]
struct OpinionNode {
    state: String,
    /// Null for an account that was deleted.
    author: Option<Author>,
}

#[derive(Deserialize)]
struct ThreadNode {
    #[serde(rename = "isResolved")]
    is_resolved: bool,
}

#[derive(Deserialize)]
pub(super) struct CommitNode {
    commit: CommitRollup,
}

/// The rolled-up checks of the last of `commits`, if it has any.
pub(super) fn latest_ci(commits: Nodes<CommitNode>) -> Option<CiState> {
    commits
        .present()
        .last()
        .and_then(|c| c.commit.status_check_rollup)
        .and_then(|r| ci_state(&r.state))
}

#[derive(Deserialize)]
struct CommitRollup {
    #[serde(rename = "statusCheckRollup")]
    status_check_rollup: Option<Rollup>,
}

#[derive(Deserialize)]
struct Rollup {
    state: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PullRequestNode {
    number: u64,
    title: String,
    url: String,
    is_draft: bool,
    updated_at: String,
    head_ref_name: String,
    base_ref_name: String,
    head_ref_oid: String,
    additions: u32,
    deletions: u32,
    total_comments_count: Option<u32>,
    is_cross_repository: bool,
    /// Null when the fork was deleted.
    head_repository: Option<HeadRepository>,
    /// Null for an account that was deleted.
    author: Option<Author>,
    labels: Nodes<Name>,
    assignees: Nodes<Login>,
    review_requests: Nodes<RequestNode>,
    viewer_latest_review: Option<ReviewNode>,
    commits: Nodes<CommitNode>,
}

pub(super) fn unexpected(what: &str) -> PlatformError {
    PlatformError::Unexpected(format!("GitHub sent {what} that Vet cannot read"))
}

/// The rolled-up state of GitHub's check suites and commit statuses.
pub(super) fn ci_state(state: &str) -> Option<CiState> {
    match state {
        "SUCCESS" => Some(CiState::Success),
        "FAILURE" | "ERROR" => Some(CiState::Failure),
        "PENDING" | "EXPECTED" => Some(CiState::Pending),
        _ => None,
    }
}

pub(super) fn review_verdict(state: &str) -> Option<ReviewVerdict> {
    match state {
        "APPROVED" => Some(ReviewVerdict::Approved),
        "CHANGES_REQUESTED" => Some(ReviewVerdict::ChangesRequested),
        "COMMENTED" => Some(ReviewVerdict::Commented),
        "DISMISSED" => Some(ReviewVerdict::Dismissed),
        // A draft review has not been given.
        _ => None,
    }
}

/// What GitHub rolled the reviews up to; a pull request nobody has to review or that has no
/// decisive review yet is undecided.
pub(super) fn decision(review_decision: Option<&str>) -> ReviewDecision {
    match review_decision {
        Some("APPROVED") => ReviewDecision::Approved,
        Some("CHANGES_REQUESTED") => ReviewDecision::ChangesRequested,
        _ => ReviewDecision::Undecided,
    }
}

/// Whether GitHub holds the merge back: a required review, check or protection is not met.
pub(super) fn is_blocked(merge_state_status: Option<&str>) -> bool {
    merge_state_status == Some("BLOCKED")
}

pub(super) fn review_request(node: RequestNode) -> Option<ReviewRequest> {
    requested(node).map(|(request, _)| request)
}

/// The review request with what its reviewer is called when shown.
pub(super) fn requested(node: RequestNode) -> Option<(ReviewRequest, String)> {
    let reviewer = node.requested_reviewer?;
    match reviewer.type_name.as_str() {
        "User" | "Mannequin" => {
            let login = reviewer.login?;
            let shown = display_name(reviewer.name.as_deref(), &login);
            Some((ReviewRequest::User(login), shown))
        }
        "Team" => {
            let team = format!("{}/{}", reviewer.organization?.login, reviewer.slug?);
            Some((ReviewRequest::Team(team.clone()), team))
        }
        _ => None,
    }
}

impl PullRequestNode {
    /// The pull request as the listing has it: without its review decision, merge state,
    /// unresolved threads and who asked for changes, which only [`DetailNode`] reads.
    pub(super) fn into_pull_request(self) -> Result<PullRequest, PlatformError> {
        let updated_at = parse_rfc3339(&self.updated_at).ok_or_else(|| unexpected("a time"))?;
        let ci = latest_ci(self.commits);
        Ok(PullRequest {
            number: self.number,
            title: self.title,
            url: self.url,
            author: self
                .author
                .as_ref()
                .map_or_else(|| "ghost".to_owned(), |a| a.login.clone()),
            author_name: self.author.as_ref().map_or_else(|| "ghost".to_owned(), Author::display),
            author_avatar: self.author.and_then(|a| a.avatar_url),
            source_branch: self.head_ref_name,
            target_branch: self.base_ref_name,
            fork: self.is_cross_repository.then(|| {
                self.head_repository
                    .map_or_else(|| "a deleted repository".to_owned(), |r| r.name_with_owner)
            }),
            head: self.head_ref_oid,
            lines: Some(LineDelta {
                added: self.additions,
                removed: self.deletions,
            }),
            comments: self.total_comments_count.unwrap_or(0),
            draft: self.is_draft,
            labels: self.labels.present().map(|l| l.name).collect(),
            assignees: self.assignees.present().map(|a| a.login).collect(),
            ci,
            unresolved_threads: 0,
            review_requests: self.review_requests.present().filter_map(review_request).collect(),
            viewer_review: self.viewer_latest_review.and_then(|r| {
                Some(ViewerReview {
                    verdict: review_verdict(&r.state)?,
                    commit: r.commit.map(|c| c.oid),
                })
            }),
            decision: ReviewDecision::Undecided,
            changes_requested: Vec::new(),
            blocked: false,
            updated_at,
        })
    }
}

impl DetailNode {
    /// The detail, without the pushes: the force-pushes of every page of the timeline are
    /// needed for those.
    pub(super) fn into_detail(self) -> Result<(PullRequestDetail, Vec<ForcePush>), PlatformError> {
        let force_pushes = force_pushes(self.timeline_items.nodes)?;
        let mut pull_request = self.pull_request.into_pull_request()?;
        pull_request.decision = decision(self.review_decision.as_deref());
        pull_request.blocked = is_blocked(self.merge_state_status.as_deref());
        pull_request.unresolved_threads = sat_u32(self.review_threads.present().filter(|t| !t.is_resolved).count());
        pull_request.changes_requested = self
            .latest_opinionated_reviews
            .present()
            .filter(|r| r.state == "CHANGES_REQUESTED")
            .map(|r| ChangesRequest {
                reviewer: r.author.as_ref().map_or_else(|| "ghost".to_owned(), Author::display),
                how: ChangesAsked::Requested,
            })
            .collect();
        let detail = PullRequestDetail {
            pull_request,
            base: self.base_ref_oid,
            pushes: Vec::new(),
        };
        Ok((detail, force_pushes))
    }
}

pub(super) fn force_pushes(nodes: Vec<Option<ForcePushNode>>) -> Result<Vec<ForcePush>, PlatformError> {
    let mut found = Vec::new();
    for node in nodes.into_iter().flatten() {
        // The commits of a force-push can be gone from the repository.
        let (Some(before), Some(after)) = (node.before_commit, node.after_commit) else {
            continue;
        };
        found.push(ForcePush {
            at: parse_rfc3339(&node.created_at).ok_or_else(|| unexpected("a time"))?,
            before: before.oid,
            after: after.oid,
        });
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_rolls_the_reviews_up_to_approved_or_changes_requested_and_nothing_else_decides() {
        assert_eq!(decision(Some("APPROVED")), ReviewDecision::Approved);
        assert_eq!(decision(Some("CHANGES_REQUESTED")), ReviewDecision::ChangesRequested);
        for undecided in [Some("REVIEW_REQUIRED"), Some("SOMETHING_NEW"), None] {
            assert_eq!(decision(undecided), ReviewDecision::Undecided, "{undecided:?}");
        }
    }

    #[test]
    fn only_a_blocked_merge_state_is_blocked() {
        assert!(is_blocked(Some("BLOCKED")));
        for state in [
            Some("CLEAN"),
            Some("BEHIND"),
            Some("DIRTY"),
            Some("UNSTABLE"),
            Some("DRAFT"),
            None,
        ] {
            assert!(!is_blocked(state), "{state:?}");
        }
    }
}
