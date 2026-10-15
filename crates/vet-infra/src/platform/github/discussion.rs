//! What people say on a pull request, through GitHub's GraphQL API: reading the review threads
//! and the conversation, and posting, editing, removing and resolving.
//!
//! A review is posted through a pending review that exists only for the length of one call: it
//! is created, filled and submitted, and deleted if anything on the way fails, so nothing
//! half-posted stays on GitHub. A comment on the pull request as a whole is not part of a
//! review; it is posted after it.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use vet_app::platform::{
    CommentKind, CommentRef, ConversationItem, ConversationKind, CredentialProvider, DiffSide, Discussion,
    NewConversationComment, NewThread, PlatformComment, PlatformError, PlatformThreadData, PostedComment, PostedThread,
    PullRequestState, RemoteKey, ReviewRequest, ReviewVerdict, Reviewer, ReviewerState, Submission, Submitted,
};

use super::wire::{
    Author, BackwardsPage, CommitNode, Name, Nodes, Oid, Page, RepositoryData, RequestNode, decision, is_blocked,
    latest_ci, requested, review_verdict, unexpected,
};
use super::{Clients, GitHubClient, repository};
use crate::platform::time::parse_rfc3339;
use vet_core::review::Verdict;

/// Threads asked for per page.
const THREAD_PAGE: u32 = 50;

/// The most threads read from one pull request; past it the listing is partial, so nothing is
/// taken to be gone.
const MAX_THREADS: usize = 2000;

/// Conversation comments and reviews asked for per page; the newest come first and the pages
/// before them are read until the oldest.
const CONVERSATION_PAGE: u32 = 50;

/// The most comments, and the most reviews, read from one pull request; past it the listing is
/// partial, like one that stops at [`MAX_THREADS`].
const MAX_CONVERSATION: usize = 2000;

const REVIEW: &str = "id state body submittedAt viewerDidAuthor author { login avatarUrl ... on User { name } }";

const REVIEW_COMMENT: &str = "id body createdAt lastEditedAt state viewerDidAuthor author { login avatarUrl ... on User { name } } originalCommit { oid }";

const ISSUE_COMMENT: &str =
    "id body createdAt lastEditedAt viewerDidAuthor author { login avatarUrl ... on User { name } }";

fn thread_fields() -> String {
    format!(
        "fragment ThreadFields on PullRequestReviewThread {{
  id isResolved isOutdated path diffSide line startLine originalLine originalStartLine subjectType
  resolvedBy {{ login ... on User {{ name }} }}
  comments(first: 100) {{ nodes {{ {REVIEW_COMMENT} }} pageInfo {{ hasNextPage endCursor }} }}
}}"
    )
}

fn discussion_query() -> String {
    format!(
        "query($owner: String!, $name: String!, $number: Int!, $after: String) {{
  repository(owner: $owner, name: $name) {{
    pullRequest(number: $number) {{
      title url body isDraft state headRefName baseRefName reviewDecision mergeStateStatus
      author {{ login avatarUrl ... on User {{ name }} }}
      labels(first: 20) {{ nodes {{ name }} }}
      reviewRequests(first: 30) {{
        nodes {{
          requestedReviewer {{
            __typename
            ... on User {{ login name }}
            ... on Mannequin {{ login }}
            ... on Team {{ slug organization {{ login }} }}
          }}
        }}
      }}
      latestReviews(first: 30) {{ nodes {{ state author {{ login ... on User {{ name }} }} }} }}
      commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ state }} }} }} }}
      comments(last: {CONVERSATION_PAGE}) {{ nodes {{ {ISSUE_COMMENT} }} pageInfo {{ hasPreviousPage startCursor }} }}
      reviews(last: {CONVERSATION_PAGE}) {{ nodes {{ {REVIEW} }} pageInfo {{ hasPreviousPage startCursor }} }}
      reviewThreads(first: {THREAD_PAGE}, after: $after) {{
        nodes {{ ...ThreadFields }}
        pageInfo {{ hasNextPage endCursor }}
      }}
    }}
  }}
}}
{}",
        thread_fields()
    )
}

fn threads_query() -> String {
    format!(
        "query($owner: String!, $name: String!, $number: Int!, $after: String) {{
  repository(owner: $owner, name: $name) {{
    pullRequest(number: $number) {{
      reviewThreads(first: {THREAD_PAGE}, after: $after) {{
        nodes {{ ...ThreadFields }}
        pageInfo {{ hasNextPage endCursor }}
      }}
    }}
  }}
}}
{}",
        thread_fields()
    )
}

/// The page of `connection` (`comments` or `reviews`) of a pull request that ends before `$before`.
fn earlier_query(connection: &str, fields: &str) -> String {
    format!(
        "query($owner: String!, $name: String!, $number: Int!, $before: String) {{
  repository(owner: $owner, name: $name) {{
    pullRequest(number: $number) {{
      {connection}(last: {CONVERSATION_PAGE}, before: $before) {{ nodes {{ {fields} }} pageInfo {{ hasPreviousPage startCursor }} }}
    }}
  }}
}}"
    )
}

fn comments_query() -> String {
    format!(
        "query($id: ID!, $after: String) {{
  node(id: $id) {{
    ... on PullRequestReviewThread {{
      comments(first: 100, after: $after) {{ nodes {{ {REVIEW_COMMENT} }} pageInfo {{ hasNextPage endCursor }} }}
    }}
  }}
}}"
    )
}

fn comment_query() -> String {
    format!(
        "query($id: ID!) {{
  node(id: $id) {{
    ... on PullRequestReviewComment {{ {REVIEW_COMMENT} }}
    ... on IssueComment {{ {ISSUE_COMMENT} }}
  }}
}}"
    )
}

const PULL_REQUEST_ID: &str = "query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) { pullRequest(number: $number) { id } }
}";

fn mutation(call: &str, selection: &str) -> String {
    format!(
        "mutation($input: {}Input!) {{ {call}(input: $input) {{ {selection} }} }}",
        capitalized(call)
    )
}

fn capitalized(call: &str) -> String {
    let mut chars = call.chars();
    chars
        .next()
        .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentNode {
    id: String,
    body: String,
    created_at: String,
    last_edited_at: Option<String>,
    /// Absent for an issue comment.
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    viewer_did_author: bool,
    /// Null for an account that was deleted.
    author: Option<Author>,
    /// Absent for an issue comment.
    #[serde(default)]
    original_commit: Option<Oid>,
}

impl CommentNode {
    fn into_comment(self) -> Result<PlatformComment, PlatformError> {
        let created_at = parse_rfc3339(&self.created_at).ok_or_else(|| unexpected("a time"))?;
        let edited_at = match self.last_edited_at {
            Some(at) => parse_rfc3339(&at).ok_or_else(|| unexpected("a time"))?,
            None => created_at,
        };
        let (author, avatar) = match self.author {
            Some(a) => (a.display(), a.avatar_url),
            None => ("ghost".to_owned(), None),
        };
        Ok(PlatformComment {
            id: self.id,
            author,
            avatar,
            body: self.body,
            created_at,
            edited_at,
            commit: self.original_commit.map(|c| c.oid),
            mine: self.viewer_did_author,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadNode {
    id: String,
    is_resolved: bool,
    is_outdated: bool,
    path: String,
    diff_side: String,
    original_line: Option<u32>,
    original_start_line: Option<u32>,
    line: Option<u32>,
    start_line: Option<u32>,
    subject_type: Option<String>,
    resolved_by: Option<Author>,
    comments: Page<CommentNode>,
}

#[derive(Deserialize)]
struct ThreadsNode {
    #[serde(rename = "reviewThreads")]
    review_threads: Page<ThreadNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscussionNode {
    title: String,
    url: String,
    body: String,
    is_draft: bool,
    state: String,
    head_ref_name: String,
    base_ref_name: String,
    review_decision: Option<String>,
    merge_state_status: Option<String>,
    author: Option<Author>,
    labels: Nodes<Name>,
    review_requests: Nodes<RequestNode>,
    latest_reviews: Nodes<LatestReview>,
    commits: Nodes<CommitNode>,
    comments: BackwardsPage<CommentNode>,
    reviews: BackwardsPage<ReviewNode>,
    review_threads: Page<ThreadNode>,
}

#[derive(Deserialize)]
struct LatestReview {
    state: String,
    author: Option<Author>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewNode {
    id: String,
    state: String,
    body: String,
    submitted_at: Option<String>,
    viewer_did_author: bool,
    author: Option<Author>,
}

#[derive(Deserialize)]
struct NodeData<T> {
    node: Option<T>,
}

#[derive(Deserialize)]
struct MoreComments {
    comments: Page<CommentNode>,
}

impl ThreadNode {
    /// The thread, with `more` comments after those of its first page; `None` if it holds
    /// nothing a person posted (the comments of a pending review are not).
    fn into_thread(self, more: Vec<CommentNode>) -> Result<Option<PlatformThreadData>, PlatformError> {
        let comments: Vec<PlatformComment> = self
            .comments
            .nodes
            .into_iter()
            .flatten()
            .chain(more)
            .filter(|c| c.state.as_deref() != Some("PENDING"))
            .map(CommentNode::into_comment)
            .collect::<Result<_, _>>()?;
        if comments.is_empty() {
            return Ok(None);
        }
        let file = self.subject_type.as_deref() == Some("FILE");
        // The lines are those of the file as it was when the first comment was made, which is
        // where its anchor is; where the code went since is for Vet to work out.
        let end = self.original_line.or(self.line);
        let start = self.original_start_line.or(self.start_line).or(end);
        let lines = match (file, start, end) {
            (false, Some(start), Some(end)) => Some((start.min(end), end.max(start))),
            _ => None,
        };
        Ok(Some(PlatformThreadData {
            id: self.id,
            path: self.path,
            side: if self.diff_side == "LEFT" {
                DiffSide::Left
            } else {
                DiffSide::Right
            },
            lines,
            outdated: self.is_outdated,
            resolved: self.is_resolved,
            resolved_by: self.resolved_by.as_ref().map(Author::display),
            comments,
        }))
    }
}

fn pull_request_state(state: &str) -> PullRequestState {
    match state {
        "MERGED" => PullRequestState::Merged,
        "CLOSED" => PullRequestState::Closed,
        _ => PullRequestState::Open,
    }
}

fn reviewers(latest: Nodes<LatestReview>, requests: Nodes<RequestNode>) -> Vec<Reviewer> {
    let mut found: Vec<Reviewer> = latest
        .present()
        .filter_map(|r| {
            let state = match r.state.as_str() {
                "APPROVED" => ReviewerState::Approved,
                "CHANGES_REQUESTED" => ReviewerState::ChangesRequested,
                "COMMENTED" => ReviewerState::Commented,
                "DISMISSED" => ReviewerState::Waiting,
                _ => return None,
            };
            let author = r.author?;
            Some(Reviewer {
                name: author.display(),
                login: author.login,
                state,
            })
        })
        .collect();
    for (request, name) in requests.present().filter_map(requested) {
        let (ReviewRequest::User(login) | ReviewRequest::Team(login)) = request;
        match found.iter_mut().find(|r| r.login == login) {
            // Asked again after a review: they are being waited for.
            Some(known) => known.state = ReviewerState::Waiting,
            None => found.push(Reviewer {
                login,
                name,
                state: ReviewerState::Waiting,
            }),
        }
    }
    found
}

fn conversation(comments: Vec<CommentNode>, reviews: Vec<ReviewNode>) -> Result<Vec<ConversationItem>, PlatformError> {
    let mut items = Vec::new();
    for comment in comments {
        let comment = comment.into_comment()?;
        items.push(ConversationItem {
            id: comment.id,
            author: comment.author,
            avatar: comment.avatar,
            at: comment.created_at,
            kind: ConversationKind::Comment { mine: comment.mine },
            body: comment.body,
        });
    }
    for review in reviews {
        // A review that was not submitted is its author's and no one else's business.
        let Some(at) = review.submitted_at.as_deref().and_then(parse_rfc3339) else {
            continue;
        };
        let verdict = review_verdict(&review.state);
        let opinion = matches!(verdict, Some(ReviewVerdict::Approved | ReviewVerdict::ChangesRequested));
        // A review that only holds line comments says nothing in the conversation.
        if review.body.trim().is_empty() && !opinion {
            continue;
        }
        let (author, avatar) = match review.author {
            Some(a) => (a.display(), a.avatar_url),
            None => ("ghost".to_owned(), None),
        };
        items.push(ConversationItem {
            id: review.id,
            author,
            avatar,
            at,
            kind: ConversationKind::Review {
                verdict,
                mine: review.viewer_did_author,
            },
            body: review.body,
        });
    }
    items.sort_by_key(|i| i.at);
    Ok(items)
}

fn dig<T: DeserializeOwned>(value: &Value, pointer: &str) -> Result<T, PlatformError> {
    value
        .pointer(pointer)
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .ok_or_else(|| unexpected("an answer"))
}

fn comment_at(value: &Value, pointer: &str) -> Result<PlatformComment, PlatformError> {
    dig::<CommentNode>(value, pointer)?.into_comment()
}

fn thread_input(thread: &NewThread, review: &str) -> Value {
    let side = |s: DiffSide| if s == DiffSide::Left { "LEFT" } else { "RIGHT" };
    let mut input = json!({
        "pullRequestReviewId": review,
        "path": thread.path,
        "body": thread.body,
    });
    match thread.lines {
        None => input["subjectType"] = json!("FILE"),
        Some((first, last)) => {
            input["subjectType"] = json!("LINE");
            input["line"] = json!(last);
            input["side"] = json!(side(thread.side));
            if first != last {
                input["startLine"] = json!(first);
                input["startSide"] = json!(side(thread.side));
            }
        }
    }
    input
}

impl<C: CredentialProvider + Sync> GitHubClient<C> {
    async fn mutate(&self, host: &str, clients: &Clients, query: &str, input: Value) -> Result<Value, PlatformError> {
        self.graphql(host, clients, query, json!({ "input": input })).await
    }

    /// The comments of a thread that did not fit its first page.
    async fn rest_of_comments(
        &self,
        host: &str,
        clients: &Clients,
        thread: &str,
        first: &Page<CommentNode>,
    ) -> Result<Vec<CommentNode>, PlatformError> {
        let mut after = first
            .page_info
            .end_cursor
            .clone()
            .filter(|_| first.page_info.has_next_page);
        let mut more = Vec::new();
        while let Some(cursor) = after {
            let data: NodeData<MoreComments> = self
                .graphql(
                    host,
                    clients,
                    &comments_query(),
                    json!({ "id": thread, "after": cursor }),
                )
                .await?;
            let page = data.node.ok_or_else(|| unexpected("a thread"))?.comments;
            after = page.page_info.end_cursor.filter(|_| page.page_info.has_next_page);
            more.extend(page.nodes.into_iter().flatten());
        }
        Ok(more)
    }

    /// The nodes of `connection` of the pull request: those of `newest`, its last page, and of
    /// the pages before it, as far as [`MAX_CONVERSATION`] allows; and whether that is all of them.
    async fn all_backwards<T: DeserializeOwned>(
        &self,
        host: &str,
        clients: &Clients,
        pull_request: (&str, &str, u64),
        connection: &str,
        fields: &str,
        newest: BackwardsPage<T>,
    ) -> Result<(Vec<T>, bool), PlatformError> {
        let (owner, name, number) = pull_request;
        let mut before = newest
            .page_info
            .start_cursor
            .filter(|_| newest.page_info.has_previous_page);
        let mut found: Vec<T> = newest.nodes.into_iter().flatten().collect();
        while let Some(cursor) = before {
            if found.len() >= MAX_CONVERSATION {
                return Ok((found, false));
            }
            let data: Value = self
                .graphql(
                    host,
                    clients,
                    &earlier_query(connection, fields),
                    json!({ "owner": owner, "name": name, "number": number, "before": cursor }),
                )
                .await?;
            let page: BackwardsPage<T> = dig(&data, &format!("/repository/pullRequest/{connection}"))?;
            before = page.page_info.start_cursor.filter(|_| page.page_info.has_previous_page);
            found.extend(page.nodes.into_iter().flatten());
        }
        Ok((found, true))
    }

    /// Every thread of `page` and of the pages after it, with all their comments, and whether
    /// that is every thread of the pull request.
    async fn all_threads(
        &self,
        host: &str,
        clients: &Clients,
        mut page: Page<ThreadNode>,
        pull_request: (&str, &str, u64),
    ) -> Result<(Vec<PlatformThreadData>, bool), PlatformError> {
        let (owner, name, number) = pull_request;
        let mut threads = Vec::new();
        loop {
            let (nodes, info) = (page.nodes, page.page_info);
            for node in nodes.into_iter().flatten() {
                let more = self.rest_of_comments(host, clients, &node.id, &node.comments).await?;
                threads.extend(node.into_thread(more)?);
            }
            let Some(cursor) = info.end_cursor.filter(|_| info.has_next_page) else {
                return Ok((threads, true));
            };
            if threads.len() >= MAX_THREADS {
                return Ok((threads, false));
            }
            let data: RepositoryData<ThreadsNode> = self
                .graphql(
                    host,
                    clients,
                    &threads_query(),
                    json!({ "owner": owner, "name": name, "number": number, "after": cursor }),
                )
                .await?;
            page = data
                .pull_request()
                .ok_or_else(|| unexpected("a pull request"))?
                .review_threads;
        }
    }

    pub(super) async fn read_discussion(&self, remote: &RemoteKey, number: u64) -> Result<Discussion, PlatformError> {
        let (owner, name) = repository(remote)?;
        let host = remote.host.as_str();
        let clients = self.clients(host).await?;
        let data: RepositoryData<DiscussionNode> = self
            .graphql(
                host,
                &clients,
                &discussion_query(),
                json!({ "owner": owner, "name": name, "number": number, "after": null }),
            )
            .await?;
        let node = data.pull_request().ok_or_else(|| {
            PlatformError::NotFound(format!(
                "{owner}/{name}#{number} was not found on {host}, or the token cannot see it"
            ))
        })?;
        let (threads, threads_complete) = self
            .all_threads(host, &clients, node.review_threads, (owner, name, number))
            .await?;
        let (comments, comments_complete) = self
            .all_backwards(
                host,
                &clients,
                (owner, name, number),
                "comments",
                ISSUE_COMMENT,
                node.comments,
            )
            .await?;
        let (reviews, reviews_complete) = self
            .all_backwards(host, &clients, (owner, name, number), "reviews", REVIEW, node.reviews)
            .await?;
        let (author, author_name, author_avatar) = match node.author {
            Some(a) => (a.login.clone(), a.display(), a.avatar_url),
            None => ("ghost".to_owned(), "ghost".to_owned(), None),
        };
        Ok(Discussion {
            title: node.title,
            url: node.url,
            description: node.body,
            draft: node.is_draft,
            source_branch: node.head_ref_name,
            target_branch: node.base_ref_name,
            state: pull_request_state(&node.state),
            author,
            author_name,
            author_avatar,
            reviewers: reviewers(node.latest_reviews, node.review_requests),
            viewer_verdict: None,
            labels: node.labels.present().map(|l| l.name).collect(),
            ci: latest_ci(node.commits),
            decision: decision(node.review_decision.as_deref()),
            blocked: is_blocked(node.merge_state_status.as_deref()),
            conversation: conversation(comments, reviews)?,
            threads,
            complete: threads_complete && comments_complete && reviews_complete,
        })
    }

    pub(super) async fn read_comment(
        &self,
        host: &str,
        comment: &CommentRef,
    ) -> Result<PlatformComment, PlatformError> {
        let clients = self.clients(host).await?;
        let data: NodeData<Value> = self
            .graphql(host, &clients, &comment_query(), json!({ "id": comment.id }))
            .await?;
        data.node
            .and_then(|v| serde_json::from_value::<CommentNode>(v).ok())
            .ok_or_else(|| PlatformError::NotFound(format!("{host} has no comment {}", comment.id)))?
            .into_comment()
    }

    pub(super) async fn write_comment(
        &self,
        host: &str,
        comment: &CommentRef,
        body: &str,
    ) -> Result<PlatformComment, PlatformError> {
        let clients = self.clients(host).await?;
        let (call, id_field, result, selection) = match comment.kind {
            CommentKind::Review => (
                "updatePullRequestReviewComment",
                "pullRequestReviewCommentId",
                "/updatePullRequestReviewComment/pullRequestReviewComment",
                format!("pullRequestReviewComment {{ {REVIEW_COMMENT} }}"),
            ),
            CommentKind::Conversation => (
                "updateIssueComment",
                "id",
                "/updateIssueComment/issueComment",
                format!("issueComment {{ {ISSUE_COMMENT} }}"),
            ),
        };
        let answer = self
            .mutate(
                host,
                &clients,
                &mutation(call, &selection),
                json!({ id_field: comment.id, "body": body }),
            )
            .await?;
        comment_at(&answer, result)
    }

    pub(super) async fn remove_comment(&self, host: &str, comment: &CommentRef) -> Result<(), PlatformError> {
        let clients = self.clients(host).await?;
        let call = match comment.kind {
            CommentKind::Review => "deletePullRequestReviewComment",
            CommentKind::Conversation => "deleteIssueComment",
        };
        self.mutate(
            host,
            &clients,
            &mutation(call, "clientMutationId"),
            json!({ "id": comment.id }),
        )
        .await
        .map(|_| ())
    }

    pub(super) async fn resolve_thread(&self, host: &str, thread: &str, resolved: bool) -> Result<(), PlatformError> {
        let clients = self.clients(host).await?;
        let call = if resolved {
            "resolveReviewThread"
        } else {
            "unresolveReviewThread"
        };
        self.mutate(
            host,
            &clients,
            &mutation(call, "thread { id isResolved }"),
            json!({ "threadId": thread }),
        )
        .await
        .map(|_| ())
    }

    pub(super) async fn post_review(
        &self,
        remote: &RemoteKey,
        number: u64,
        submission: &Submission,
    ) -> Result<Submitted, PlatformError> {
        let (owner, name) = repository(remote)?;
        let host = remote.host.as_str();
        let clients = self.clients(host).await?;
        let found: Value = self
            .graphql(
                host,
                &clients,
                PULL_REQUEST_ID,
                json!({ "owner": owner, "name": name, "number": number }),
            )
            .await?;
        let pull_request: String = dig(&found, "/repository/pullRequest/id").map_err(|_| {
            PlatformError::NotFound(format!(
                "{owner}/{name}#{number} was not found on {host}, or the token cannot see it"
            ))
        })?;

        let mut posted = if needs_review(submission) {
            self.post_pending_review(host, &clients, &pull_request, submission)
                .await?
        } else {
            // A review without lines, verdict or summary would be an empty "Commented" review, or refused.
            Submitted::default()
        };
        for new in &submission.comments {
            match self.post_conversation_comment(host, &clients, &pull_request, new).await {
                Ok(comment) => posted.comments.push(comment),
                Err(e) => {
                    posted.failed = Some(e);
                    break;
                }
            }
        }
        Ok(posted)
    }

    /// Everything of the review goes into a pending review that is submitted at the end, or
    /// deleted so that none of it stays on GitHub.
    async fn post_pending_review(
        &self,
        host: &str,
        clients: &Clients,
        pull_request: &str,
        submission: &Submission,
    ) -> Result<Submitted, PlatformError> {
        let started = self
            .mutate(
                host,
                clients,
                &mutation("addPullRequestReview", "pullRequestReview { id }"),
                json!({ "pullRequestId": pull_request, "commitOID": submission.commit }),
            )
            .await?;
        let review: String = dig(&started, "/addPullRequestReview/pullRequestReview/id")?;
        let filled = self.fill_review(host, clients, &review, submission).await;
        let posted = match filled {
            Ok(posted) => posted,
            Err(e) => {
                self.discard_review(host, clients, &review).await;
                return Err(e);
            }
        };
        let event = event_of(&submission.verdict)?;
        let mut body = json!({ "pullRequestReviewId": review, "event": event });
        if let Some(text) = &submission.body {
            body["body"] = json!(text);
        }
        let submitted = self
            .mutate(
                host,
                clients,
                &mutation("submitPullRequestReview", "pullRequestReview { id }"),
                body,
            )
            .await;
        if let Err(e) = submitted {
            self.discard_review(host, clients, &review).await;
            return Err(e);
        }
        Ok(posted)
    }

    async fn fill_review(
        &self,
        host: &str,
        clients: &Clients,
        review: &str,
        submission: &Submission,
    ) -> Result<Submitted, PlatformError> {
        let mut posted = Submitted::default();
        for new in &submission.threads {
            let added = self
                .mutate(
                    host,
                    clients,
                    &mutation(
                        "addPullRequestReviewThread",
                        &format!("thread {{ id comments(first: 1) {{ nodes {{ {REVIEW_COMMENT} }} }} }}"),
                    ),
                    thread_input(new, review),
                )
                .await?;
            let thread: String = dig(&added, "/addPullRequestReviewThread/thread/id")?;
            let comment = comment_at(&added, "/addPullRequestReviewThread/thread/comments/nodes/0")?;
            let mut replies = Vec::new();
            for reply in &new.replies {
                replies.push(PostedComment {
                    key: reply.key,
                    comment: self.post_reply(host, clients, review, &thread, &reply.body).await?,
                });
            }
            posted.threads.push(PostedThread {
                key: new.key,
                thread,
                comment,
                replies,
            });
        }
        for reply in &submission.replies {
            posted.replies.push(PostedComment {
                key: reply.key,
                comment: self
                    .post_reply(host, clients, review, &reply.thread, &reply.body)
                    .await?,
            });
        }
        Ok(posted)
    }

    async fn post_reply(
        &self,
        host: &str,
        clients: &Clients,
        review: &str,
        thread: &str,
        body: &str,
    ) -> Result<PlatformComment, PlatformError> {
        let added = self
            .mutate(
                host,
                clients,
                &mutation(
                    "addPullRequestReviewThreadReply",
                    &format!("comment {{ {REVIEW_COMMENT} }}"),
                ),
                json!({ "pullRequestReviewId": review, "pullRequestReviewThreadId": thread, "body": body }),
            )
            .await?;
        comment_at(&added, "/addPullRequestReviewThreadReply/comment")
    }

    async fn post_conversation_comment(
        &self,
        host: &str,
        clients: &Clients,
        pull_request: &str,
        new: &NewConversationComment,
    ) -> Result<PostedComment, PlatformError> {
        let added = self
            .mutate(
                host,
                clients,
                &mutation("addComment", &format!("commentEdge {{ node {{ {ISSUE_COMMENT} }} }}")),
                json!({ "subjectId": pull_request, "body": new.body }),
            )
            .await?;
        Ok(PostedComment {
            key: new.key,
            comment: comment_at(&added, "/addComment/commentEdge/node")?,
        })
    }

    /// Deletes a pending review. A failure leaves it behind, which the viewer sees on GitHub as a
    /// review in progress; the error that led here is what is reported.
    async fn discard_review(&self, host: &str, clients: &Clients, review: &str) {
        let _ = self
            .mutate(
                host,
                clients,
                &mutation("deletePullRequestReview", "clientMutationId"),
                json!({ "pullRequestReviewId": review }),
            )
            .await;
    }
}

/// How GitHub names the verdict a submission closes its review with, in the submission's event.
/// The session sends one of GitHub's verdicts; another one is refused, for sending it as another
/// would tell the platform what the viewer did not.
pub(super) fn event_of(verdict: &Verdict) -> Result<&'static str, PlatformError> {
    match verdict {
        Verdict::Comment => Ok("COMMENT"),
        Verdict::Approve => Ok("APPROVE"),
        Verdict::RequestChanges => Ok("REQUEST_CHANGES"),
        other => Err(PlatformError::Unexpected(format!("GitHub has no verdict {other}"))),
    }
}

/// Whether a submission is a review: comments on the lines or in threads, a verdict other than a
/// plain comment, or a summary. Comments on the pull request as a whole are not.
fn needs_review(submission: &Submission) -> bool {
    !submission.threads.is_empty()
        || !submission.replies.is_empty()
        || submission.body.is_some()
        || submission.verdict != Verdict::Comment
}
