//! What people say on an Azure DevOps pull request: its threads and reviewers, and posting,
//! editing, removing, resolving and voting.
//!
//! A thread belongs to a pull request and a comment to a thread, and none of the three numbers is
//! unique without the others, while the port names a thread or a comment by one text. So the ids
//! Vet keeps are `<pull request>/<thread>` and `<pull request>/<thread>/<comment>`.
//!
//! Drafts are posted one comment after the other: Azure DevOps has no pending review. A thread's
//! lines are those of the iteration it was made on (`iterationContext`, or where Azure DevOps
//! tracked it to), which is the commit its anchor is built on.

use std::collections::HashMap;

use azure_devops_rust_api::git::models::{
    Comment, CommentIterationContext, CommentPosition, CommentThread, CommentThreadContext, GitPullRequest,
    GitPullRequestCommentThread, GitPullRequestCommentThreadContext, GitPullRequestIteration, IdentityRef,
    IdentityRefWithVote, comment::CommentType, comment_thread::Status as ThreadStatus,
    git_pull_request::Status as PullRequestStatus,
};
use futures::future::try_join3;
use vet_app::platform::{
    CommentRef, CredentialProvider, DiffSide, Discussion, NewThread, PlatformComment, PlatformError,
    PlatformThreadData, PostedComment, PostedThread, PullRequestState, Reviewer, ReviewerState, Submission, Submitted,
};

use super::wire::{self, Me};
use super::{AzureDevOpsClient, Call, Repository};

/// The most pages of a pull request's changes read to find a file's tracking id.
const MAX_CHANGE_PAGES: usize = 20;

/// The votes and the verdicts that name them, in the platform's order.
const VOTES: [(&str, i64); 4] = [
    ("approved", 10),
    ("approved-with-suggestions", 5),
    ("wait-for-author", -5),
    ("rejected", -10),
];

pub(super) fn vote_of(verdict: &str) -> Option<i64> {
    VOTES.iter().find(|(id, _)| *id == verdict).map(|(_, vote)| *vote)
}

fn verdict_of(vote: i64) -> Option<&'static str> {
    VOTES.iter().find(|(_, v)| *v == vote).map(|(id, _)| *id)
}

pub(super) fn thread_id(pull_request: i32, thread: i32) -> String {
    format!("{pull_request}/{thread}")
}

fn comment_id(pull_request: i32, thread: i32, comment: i64) -> String {
    format!("{pull_request}/{thread}/{comment}")
}

/// The pull request and the thread an id of [`thread_id`] names.
pub(super) fn parse_thread(id: &str) -> Option<(i32, i32)> {
    let (pull_request, thread) = id.split_once('/')?;
    Some((pull_request.parse().ok()?, thread.parse().ok()?))
}

/// The pull request, thread and comment an id of [`comment_id`] names.
fn parse_comment(id: &str) -> Option<(i32, i32, i32)> {
    let (thread, comment) = id.rsplit_once('/')?;
    let (pull_request, thread) = parse_thread(thread)?;
    Some((pull_request, thread, comment.parse().ok()?))
}

pub(super) fn pull_request_id(remote: &vet_app::platform::RemoteKey, number: u64) -> Result<i32, PlatformError> {
    i32::try_from(number).map_err(|_| PlatformError::Unexpected(format!("{remote}: pull request {number}")))
}

/// Resolved is what the pull request page shows as such: fixed, won't fix, closed and by design.
fn resolved(status: Option<&ThreadStatus>) -> bool {
    matches!(
        status,
        Some(ThreadStatus::Fixed | ThreadStatus::WontFix | ThreadStatus::Closed | ThreadStatus::ByDesign)
    )
}

fn pull_request_state(status: &PullRequestStatus) -> PullRequestState {
    match status {
        PullRequestStatus::Abandoned => PullRequestState::Closed,
        PullRequestStatus::Completed => PullRequestState::Merged,
        _ => PullRequestState::Open,
    }
}

/// A vote is approval from 5 on, waiting for the author or rejection below 0.
fn reviewer_state(vote: i64) -> ReviewerState {
    match vote {
        5.. => ReviewerState::Approved,
        0 => ReviewerState::Waiting,
        _ => ReviewerState::ChangesRequested,
    }
}

/// The reviewers who have not declined, people by their name and groups by theirs.
fn reviewers(pr: &GitPullRequest, me: &Me) -> Vec<Reviewer> {
    pr.reviewers
        .iter()
        .filter(|r| r.has_declined != Some(true))
        .map(|r| {
            let identity = &r.identity_ref;
            let login = if identity.is_container == Some(true) {
                identity
                    .graph_subject_base
                    .display_name
                    .clone()
                    .unwrap_or_else(|| identity.id.clone())
            } else {
                me.name(identity)
            };
            Reviewer {
                login,
                name: Me::display(identity),
                state: reviewer_state(r.vote.unwrap_or(0)),
            }
        })
        .collect()
}

/// The verdict the viewer voted, if they have a vote.
fn viewer_verdict(pr: &GitPullRequest, me: &Me) -> Option<String> {
    let vote = pr.reviewers.iter().find(|r| me.is(&r.identity_ref))?.vote?;
    verdict_of(vote).map(str::to_owned)
}

/// The commit of each iteration.
pub(super) fn iteration_commits(iterations: &[GitPullRequestIteration]) -> HashMap<i64, String> {
    iterations
        .iter()
        .filter_map(|i| Some((i64::from(i.id?), i.source_ref_commit.as_ref()?.commit_id.clone()?)))
        .collect()
}

fn comment(pull_request: i32, thread: i32, c: &Comment, me: &Me, commit: Option<&String>) -> Option<PlatformComment> {
    if c.is_deleted == Some(true) || matches!(c.comment_type, Some(CommentType::System)) {
        return None;
    }
    let created = c
        .published_date
        .or(c.last_updated_date)
        .map_or(0, |d| d.unix_timestamp());
    Some(PlatformComment {
        id: comment_id(pull_request, thread, c.id?),
        author: c.author.as_ref().map_or_else(|| "unknown".to_owned(), Me::display),
        // The pictures need the sign-in that the web page has; the initials stand in.
        avatar: None,
        body: c.content.clone().unwrap_or_default(),
        created_at: created,
        edited_at: c.last_content_updated_date.map_or(created, |d| d.unix_timestamp()),
        commit: commit.cloned(),
        mine: c.author.as_ref().is_some_and(|a| me.is(a)),
    })
}

/// A line of a thread's position; none for a position that names no line.
fn line_of(position: &CommentPosition) -> Option<u32> {
    u32::try_from(position.line?).ok().filter(|l| *l > 0)
}

/// The side and 1-based inclusive lines of a thread's position; none for the whole file.
fn place(context: &CommentThreadContext) -> (DiffSide, Option<(u32, u32)>) {
    let range = |start: &CommentPosition, end: &Option<CommentPosition>| {
        let first = line_of(start)?;
        let last = end.as_ref().and_then(line_of).unwrap_or(first);
        Some((first.min(last), first.max(last)))
    };
    if let Some(start) = &context.right_file_start {
        (DiffSide::Right, range(start, &context.right_file_end))
    } else if let Some(start) = &context.left_file_start {
        (DiffSide::Left, range(start, &context.left_file_end))
    } else {
        (DiffSide::Right, None)
    }
}

/// The thread as Vet has it; none for one deleted or holding nothing a person wrote.
fn thread_data(
    pull_request: i32,
    thread: &GitPullRequestCommentThread,
    me: &Me,
    commits: &HashMap<i64, String>,
) -> Option<PlatformThreadData> {
    let base = &thread.comment_thread;
    if base.is_deleted == Some(true) {
        return None;
    }
    let id = base.id?;
    let iteration = thread.pull_request_thread_context.as_ref().and_then(|c| {
        let tracked = c
            .tracking_criteria
            .as_ref()
            .and_then(|t| t.second_comparing_iteration)
            .filter(|i| *i > 0)
            .map(i64::from);
        tracked.or_else(|| c.iteration_context.as_ref()?.second_comparing_iteration)
    });
    let commit = iteration.and_then(|i| commits.get(&i));
    let comments: Vec<PlatformComment> = base
        .comments
        .iter()
        .filter_map(|c| comment(pull_request, id, c, me, commit))
        .collect();
    if comments.is_empty() {
        return None;
    }
    let (path, side, lines) = match &base.thread_context {
        Some(context) => {
            let (side, lines) = place(context);
            let path = context.file_path.as_deref().unwrap_or_default();
            (path.trim_start_matches('/').to_owned(), side, lines)
        }
        None => (String::new(), DiffSide::Right, None),
    };
    Some(PlatformThreadData {
        id: thread_id(pull_request, id),
        path,
        side,
        lines,
        outdated: false,
        resolved: resolved(base.status.as_ref()),
        // A thread carries its status but not who set it.
        resolved_by: None,
        comments,
    })
}

/// A comment to post. Its parent is the first comment of the thread it replies in.
fn new_comment(body: &str, parent: i64) -> Comment {
    Comment {
        content: Some(body.to_owned()),
        comment_type: Some(CommentType::Text),
        parent_comment_id: Some(parent),
        ..Comment::default()
    }
}

fn position(line: u32) -> CommentPosition {
    CommentPosition {
        line: i32::try_from(line).ok(),
        offset: Some(1),
    }
}

/// Where a new thread goes: the iteration it is made on, and the tracking ids of the files that
/// iteration changes, which let Azure DevOps follow a thread across iterations.
struct Place {
    iteration: Option<i32>,
    tracking: HashMap<String, i32>,
}

fn new_thread_body(thread: &NewThread, place: &Place) -> GitPullRequestCommentThread {
    let context = |path: &str, lines: Option<(u32, u32)>| {
        let mut context = CommentThreadContext {
            file_path: Some(format!("/{path}")),
            ..CommentThreadContext::default()
        };
        if let Some((first, last)) = lines {
            let (start, end) = (Some(position(first)), Some(position(last)));
            match thread.side {
                DiffSide::Right => (context.right_file_start, context.right_file_end) = (start, end),
                DiffSide::Left => (context.left_file_start, context.left_file_end) = (start, end),
            }
        }
        context
    };
    GitPullRequestCommentThread {
        comment_thread: CommentThread {
            comments: vec![new_comment(&thread.body, 0)],
            status: Some(ThreadStatus::Active),
            thread_context: Some(context(&thread.path, thread.lines)),
            ..CommentThread::default()
        },
        pull_request_thread_context: place.iteration.map(|iteration| GitPullRequestCommentThreadContext {
            change_tracking_id: place.tracking.get(&thread.path).copied(),
            iteration_context: Some(CommentIterationContext {
                first_comparing_iteration: Some(i64::from(iteration)),
                second_comparing_iteration: Some(i64::from(iteration)),
            }),
            tracking_criteria: None,
        }),
    }
}

/// A thread on the pull request as a whole.
fn new_whole_thread_body(body: &str) -> GitPullRequestCommentThread {
    GitPullRequestCommentThread {
        comment_thread: CommentThread {
            comments: vec![new_comment(body, 0)],
            status: Some(ThreadStatus::Active),
            ..CommentThread::default()
        },
        pull_request_thread_context: None,
    }
}

impl<C: CredentialProvider + Sync> AzureDevOpsClient<C> {
    pub(super) async fn read_discussion(
        &self,
        remote: &vet_app::platform::RemoteKey,
        number: u64,
    ) -> Result<Discussion, PlatformError> {
        let call = self.call(remote).await?;
        let id = pull_request_id(remote, number)?;
        let host = &remote.account();
        let Repository {
            organization,
            project,
            name,
        } = call.repository;
        let (pr, threads, iterations) = try_join3(
            async {
                let found = call
                    .git
                    .pull_requests_client()
                    .get_pull_request(organization, name, id, project)
                    .await;
                self.settle(host, found).map_err(|e| match e {
                    PlatformError::NotFound(_) => PlatformError::NotFound(format!(
                        "{organization}/{project}/{name}#{number} was not found on {host}, or the token cannot see it"
                    )),
                    other => other,
                })
            },
            async {
                let list = call
                    .git
                    .pull_request_threads_client()
                    .list(organization, name, id, project)
                    .await;
                self.settle(host, list).map(|l| l.value)
            },
            async {
                let list = call
                    .git
                    .pull_request_iterations_client()
                    .list(organization, name, id, project)
                    .await;
                self.settle(host, list).map(|l| l.value)
            },
        )
        .await?;

        let commits = iteration_commits(&iterations);
        let head = pr.last_merge_source_commit.as_ref().and_then(|c| c.commit_id.clone());
        let ci = match &head {
            Some(head) => {
                let checks = self.pull_request_checks(&call, &pr, &iterations, head).await;
                Self::or_nothing(checks.map(|c| c.state()))?
            }
            None => None,
        };
        Ok(Discussion {
            title: pr.title.clone().unwrap_or_default(),
            url: format!("{}{id}", super::web_address(remote)),
            draft: pr.is_draft,
            source_branch: wire::branch(&pr.source_ref_name),
            target_branch: wire::branch(&pr.target_ref_name),
            state: pull_request_state(&pr.status),
            author: call.me.name(&pr.created_by),
            author_name: Me::display(&pr.created_by),
            // The pictures need the sign-in that the web page has; the initials stand in.
            author_avatar: None,
            reviewers: reviewers(&pr, &call.me),
            viewer_verdict: viewer_verdict(&pr, &call.me),
            labels: pr.labels.iter().filter_map(|l| l.name.clone()).collect(),
            ci,
            conversation: Vec::new(),
            threads: threads
                .iter()
                .filter_map(|t| thread_data(id, t, &call.me, &commits))
                .collect(),
            complete: true,
        })
    }

    pub(super) async fn read_comment(
        &self,
        remote: &vet_app::platform::RemoteKey,
        comment_ref: &CommentRef,
    ) -> Result<PlatformComment, PlatformError> {
        let (pull_request, thread, comment_number) = parse_comment(&comment_ref.id).ok_or_else(|| {
            PlatformError::NotFound(format!("{} has no comment {}", remote.account(), comment_ref.id))
        })?;
        let call = self.call(remote).await?;
        let Repository {
            organization,
            project,
            name,
        } = call.repository;
        let found = call
            .git
            .pull_request_thread_comments_client()
            .get(organization, name, pull_request, thread, comment_number, project)
            .await;
        let found = self.settle(&remote.account(), found)?;
        self::comment(pull_request, thread, &found, &call.me, None)
            .ok_or_else(|| PlatformError::NotFound(format!("{} has no comment {}", remote.account(), comment_ref.id)))
    }

    pub(super) async fn write_comment(
        &self,
        remote: &vet_app::platform::RemoteKey,
        comment_ref: &CommentRef,
        body: &str,
    ) -> Result<PlatformComment, PlatformError> {
        let (pull_request, thread, comment_number) = parse_comment(&comment_ref.id).ok_or_else(|| {
            PlatformError::NotFound(format!("{} has no comment {}", remote.account(), comment_ref.id))
        })?;
        let call = self.call(remote).await?;
        let Repository {
            organization,
            project,
            name,
        } = call.repository;
        let edited = Comment {
            content: Some(body.to_owned()),
            ..Comment::default()
        };
        let updated = call
            .git
            .pull_request_thread_comments_client()
            .update(
                organization,
                edited,
                name,
                pull_request,
                thread,
                comment_number,
                project,
            )
            .await;
        let updated = self.settle(&remote.account(), updated)?;
        self::comment(pull_request, thread, &updated, &call.me, None).ok_or_else(|| {
            PlatformError::Unexpected(format!(
                "{} answered an edit with something but a comment",
                remote.account()
            ))
        })
    }

    pub(super) async fn remove_comment(
        &self,
        remote: &vet_app::platform::RemoteKey,
        comment_ref: &CommentRef,
    ) -> Result<(), PlatformError> {
        let (pull_request, thread, comment_number) = parse_comment(&comment_ref.id).ok_or_else(|| {
            PlatformError::NotFound(format!("{} has no comment {}", remote.account(), comment_ref.id))
        })?;
        let call = self.call(remote).await?;
        let Repository {
            organization,
            project,
            name,
        } = call.repository;
        let removed = call
            .git
            .pull_request_thread_comments_client()
            .delete(organization, name, pull_request, thread, comment_number, project)
            .await;
        self.settle(&remote.account(), removed)
    }

    pub(super) async fn resolve_thread(
        &self,
        remote: &vet_app::platform::RemoteKey,
        thread_ref: &str,
        resolve: bool,
    ) -> Result<(), PlatformError> {
        let (pull_request, thread) = parse_thread(thread_ref)
            .ok_or_else(|| PlatformError::NotFound(format!("{} has no thread {thread_ref}", remote.account())))?;
        let call = self.call(remote).await?;
        let Repository {
            organization,
            project,
            name,
        } = call.repository;
        let update = GitPullRequestCommentThread {
            comment_thread: CommentThread {
                // What the pull request page's Resolve button sets.
                status: Some(if resolve {
                    ThreadStatus::Fixed
                } else {
                    ThreadStatus::Active
                }),
                ..CommentThread::default()
            },
            pull_request_thread_context: None,
        };
        let updated = call
            .git
            .pull_request_threads_client()
            .update(organization, update, name, pull_request, thread, project)
            .await;
        self.settle(&remote.account(), updated).map(|_| ())
    }

    pub(super) async fn set_vote(
        &self,
        remote: &vet_app::platform::RemoteKey,
        number: u64,
        verdict: &str,
    ) -> Result<(), PlatformError> {
        let vote = vote_of(verdict)
            .ok_or_else(|| PlatformError::Unexpected(format!("{} has no verdict {verdict}", remote.account())))?;
        let id = pull_request_id(remote, number)?;
        let call = self.call(remote).await?;
        let me =
            call.me.id.clone().ok_or_else(|| {
                PlatformError::Unexpected(format!("{} did not say who the user is", remote.account()))
            })?;
        let Repository {
            organization,
            project,
            name,
        } = call.repository;
        let body = IdentityRefWithVote {
            vote: Some(vote),
            ..IdentityRefWithVote::new(IdentityRef::new(me.clone()))
        };
        let voted = call
            .git
            .pull_request_reviewers_client()
            .create_pull_request_reviewer(organization, body, name, id, me, project)
            .await;
        self.settle(&remote.account(), voted).map(|_| ())
    }

    /// Posts the drafts of `submission` one after the other. Posting stops at the first the platform
    /// refuses, which is in the answer with what was posted before it.
    pub(super) async fn post_in_turn(
        &self,
        remote: &vet_app::platform::RemoteKey,
        number: u64,
        submission: &Submission,
    ) -> Result<Submitted, PlatformError> {
        let id = pull_request_id(remote, number)?;
        let call = self.call(remote).await?;
        let place = self.place_for(&call, id, &submission.commit).await?;
        let mut posted = Submitted::default();
        if let Err(e) = self.post_all(&call, id, submission, &place, &mut posted).await {
            posted.failed = Some(e);
        }
        Ok(posted)
    }

    /// The iteration of `commit`, else the latest, and the tracking ids of its files.
    async fn place_for(&self, call: &Call<'_>, pull_request: i32, commit: &str) -> Result<Place, PlatformError> {
        let Repository {
            organization,
            project,
            name,
        } = call.repository;
        let host = &call.remote.account();
        let list = call
            .git
            .pull_request_iterations_client()
            .list(organization, name, pull_request, project)
            .await;
        let iterations = self.settle(host, list)?.value;
        let of_commit = iterations
            .iter()
            .filter(|i| i.source_ref_commit.as_ref().and_then(|c| c.commit_id.as_deref()) == Some(commit))
            .filter_map(|i| i.id)
            .max();
        let iteration = of_commit.or_else(|| iterations.iter().filter_map(|i| i.id).max());
        let mut tracking = HashMap::new();
        if let Some(iteration) = iteration {
            let mut skip = 0;
            for _ in 0..MAX_CHANGE_PAGES {
                let page = call
                    .git
                    .pull_request_iteration_changes_client()
                    .get(organization, name, pull_request, iteration, project)
                    .skip(skip)
                    .await;
                // Without the ids the thread is still posted, it only is not followed across pushes.
                let Ok(page) = page else { break };
                for entry in &page.change_entries {
                    let path = entry
                        .git_change
                        .change
                        .item
                        .as_ref()
                        .and_then(|i| i.get("path")?.as_str());
                    if let (Some(path), Some(id)) = (path, entry.change_tracking_id) {
                        tracking.insert(path.trim_start_matches('/').to_owned(), id);
                    }
                }
                match page.next_skip {
                    Some(next) if next > 0 => skip = next,
                    _ => break,
                }
            }
        }
        Ok(Place { iteration, tracking })
    }

    async fn post_all(
        &self,
        call: &Call<'_>,
        pull_request: i32,
        submission: &Submission,
        place: &Place,
        posted: &mut Submitted,
    ) -> Result<(), PlatformError> {
        for new in &submission.threads {
            let made = self
                .create_thread(
                    call,
                    pull_request,
                    new_thread_body(new, place),
                    new.key,
                    &submission.commit,
                )
                .await?;
            let thread = made.thread.clone();
            posted.threads.push(made);
            for reply in &new.replies {
                let comment = self
                    .reply(call, &thread, reply.key, &reply.body, &submission.commit)
                    .await?;
                posted.threads.last_mut().expect("pushed").replies.push(comment);
            }
        }
        for reply in &submission.replies {
            let comment = self
                .reply(call, &reply.thread, reply.key, &reply.body, &submission.commit)
                .await?;
            posted.replies.push(comment);
        }
        for new in &submission.comments {
            let made = self
                .create_thread(
                    call,
                    pull_request,
                    new_whole_thread_body(&new.body),
                    new.key,
                    &submission.commit,
                )
                .await?;
            posted.threads.push(made);
        }
        Ok(())
    }

    async fn create_thread(
        &self,
        call: &Call<'_>,
        pull_request: i32,
        body: GitPullRequestCommentThread,
        key: u64,
        commit: &str,
    ) -> Result<PostedThread, PlatformError> {
        let Repository {
            organization,
            project,
            name,
        } = call.repository;
        let created = call
            .git
            .pull_request_threads_client()
            .create(organization, body, name, pull_request, project)
            .await;
        let created = self.settle(&call.remote.account(), created)?;
        let unexpected = || {
            PlatformError::Unexpected(format!(
                "{} answered a new thread with something but a thread",
                call.remote.account()
            ))
        };
        let id = created.comment_thread.id.ok_or_else(unexpected)?;
        let first = created
            .comment_thread
            .comments
            .first()
            .and_then(|c| comment(pull_request, id, c, &call.me, Some(&commit.to_owned())))
            .ok_or_else(unexpected)?;
        Ok(PostedThread {
            key,
            thread: thread_id(pull_request, id),
            comment: first,
            replies: Vec::new(),
        })
    }

    async fn reply(
        &self,
        call: &Call<'_>,
        thread_ref: &str,
        key: u64,
        body: &str,
        commit: &str,
    ) -> Result<PostedComment, PlatformError> {
        let (pull_request, thread) = parse_thread(thread_ref)
            .ok_or_else(|| PlatformError::NotFound(format!("{} has no thread {thread_ref}", call.remote.account())))?;
        let Repository {
            organization,
            project,
            name,
        } = call.repository;
        let added = call
            .git
            .pull_request_thread_comments_client()
            .create(organization, new_comment(body, 1), name, pull_request, thread, project)
            .await;
        let added = self.settle(&call.remote.account(), added)?;
        let comment = comment(pull_request, thread, &added, &call.me, Some(&commit.to_owned())).ok_or_else(|| {
            PlatformError::Unexpected(format!(
                "{} answered a new comment with something but a comment",
                call.remote.account()
            ))
        })?;
        Ok(PostedComment { key, comment })
    }
}
