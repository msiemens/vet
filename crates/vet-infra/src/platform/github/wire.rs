//! GitHub's GraphQL and REST documents, and what Vet takes from them.

use serde::Deserialize;
use vet_app::platform::{
    ChangedLines, CiState, ForcePush, PlatformError, PullRequest, PullRequestDetail, ReviewRequest, ReviewVerdict,
    ViewerReview,
};

/// The fields of a pull request that the queue shows. One request per page asks for these
/// for every pull request on it; each connection counts once per pull request against
/// the rate limit (5 of them: labels, assignees, review requests, review threads,
/// commits).
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
  author { login }
  labels(first: 20) { nodes { name } }
  assignees(first: 10) { nodes { login } }
  reviewRequests(first: 20) {
    nodes {
      requestedReviewer {
        __typename
        ... on User { login }
        ... on Mannequin { login }
        ... on Team { slug organization { login } }
      }
    }
  }
  viewerLatestReview { state commit { oid } }
  reviewThreads(first: 100) { nodes { isResolved } }
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

pub(super) fn detail_query() -> String {
    format!(
        "query($owner: String!, $name: String!, $number: Int!, $after: String) {{
  repository(owner: $owner, name: $name) {{
    pullRequest(number: $number) {{
      ...PullRequestFields
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

#[derive(Deserialize)]
pub(super) struct SearchData {
    pub search: Search,
}

#[derive(Deserialize)]
pub(super) struct Search {
    pub nodes: Vec<Option<PullRequestNode>>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

#[derive(Deserialize)]
pub(super) struct DetailData {
    pub repository: Option<Repository>,
}

#[derive(Deserialize)]
pub(super) struct Repository {
    #[serde(rename = "pullRequest")]
    pub pull_request: Option<DetailNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DetailNode {
    #[serde(flatten)]
    pub pull_request: PullRequestNode,
    pub base_ref_oid: String,
    pub timeline_items: Timeline,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Timeline {
    pub nodes: Vec<Option<ForcePushNode>>,
    pub page_info: PageInfo,
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
    slug: Option<String>,
    organization: Option<Login>,
}

#[derive(Deserialize)]
struct ReviewNode {
    state: String,
    commit: Option<Oid>,
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
    author: Option<Login>,
    labels: Nodes<Name>,
    assignees: Nodes<Login>,
    review_requests: Nodes<RequestNode>,
    viewer_latest_review: Option<ReviewNode>,
    review_threads: Nodes<ThreadNode>,
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

pub(super) fn review_request(node: RequestNode) -> Option<ReviewRequest> {
    let reviewer = node.requested_reviewer?;
    match reviewer.type_name.as_str() {
        "User" | "Mannequin" => reviewer.login.map(ReviewRequest::User),
        "Team" => Some(ReviewRequest::Team(format!(
            "{}/{}",
            reviewer.organization?.login, reviewer.slug?
        ))),
        _ => None,
    }
}

impl PullRequestNode {
    pub(super) fn into_pull_request(self) -> Result<PullRequest, PlatformError> {
        let updated_at = parse_timestamp(&self.updated_at).ok_or_else(|| unexpected("a time"))?;
        let ci = latest_ci(self.commits);
        let unresolved_threads = self.review_threads.present().filter(|t| !t.is_resolved).count();
        Ok(PullRequest {
            number: self.number,
            title: self.title,
            url: self.url,
            author: self.author.map_or_else(|| "ghost".to_owned(), |a| a.login),
            source_branch: self.head_ref_name,
            target_branch: self.base_ref_name,
            fork: self.is_cross_repository.then(|| {
                self.head_repository
                    .map_or_else(|| "a deleted repository".to_owned(), |r| r.name_with_owner)
            }),
            head: self.head_ref_oid,
            lines: Some(ChangedLines {
                added: self.additions,
                removed: self.deletions,
            }),
            comments: self.total_comments_count.unwrap_or(0),
            draft: self.is_draft,
            labels: self.labels.present().map(|l| l.name).collect(),
            assignees: self.assignees.present().map(|a| a.login).collect(),
            ci,
            unresolved_threads: u32::try_from(unresolved_threads).unwrap_or(u32::MAX),
            review_requests: self.review_requests.present().filter_map(review_request).collect(),
            viewer_review: self.viewer_latest_review.and_then(|r| {
                Some(ViewerReview {
                    verdict: review_verdict(&r.state)?,
                    commit: r.commit.map(|c| c.oid),
                })
            }),
            updated_at,
        })
    }
}

impl DetailNode {
    /// The detail, without the pushes: the force-pushes of every page of the timeline are
    /// needed for those.
    pub(super) fn into_detail(self) -> Result<(PullRequestDetail, Vec<ForcePush>), PlatformError> {
        let force_pushes = force_pushes(self.timeline_items.nodes)?;
        let pull_request = self.pull_request.into_pull_request()?;
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
            at: parse_timestamp(&node.created_at).ok_or_else(|| unexpected("a time"))?,
            before: before.oid,
            after: after.oid,
        });
    }
    Ok(found)
}

/// Unix seconds of a UTC time as GitHub writes it, `2024-05-01T12:00:00Z`.
pub(super) fn parse_timestamp(text: &str) -> Option<i64> {
    let (date, time) = text.strip_suffix('Z')?.split_once('T')?;
    let mut date = date.split('-').map(|n| n.parse::<i64>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let mut time = time.split(':').map(|n| n.parse::<i64>().ok());
    let (hour, minute, second) = (time.next()??, time.next()??, time.next()??);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    // Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_timestamps_are_unix_seconds() {
        assert_eq!(parse_timestamp("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_timestamp("2024-05-01T12:00:00Z"), Some(1_714_564_800));
        // A leap day, and the day after it.
        assert_eq!(parse_timestamp("2024-02-29T23:59:59Z"), Some(1_709_251_199));
        assert_eq!(parse_timestamp("2024-03-01T00:00:00Z"), Some(1_709_251_200));
        assert_eq!(parse_timestamp("1969-12-31T23:59:59Z"), Some(-1));
        for bad in [
            "",
            "2024-05-01",
            "2024-05-01T12:00:00+02:00",
            "2024-13-01T00:00:00Z",
            "x-y-zT0:0:0Z",
        ] {
            assert_eq!(parse_timestamp(bad), None, "{bad:?}");
        }
    }
}
