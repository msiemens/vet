//! A platform that holds one pull request in memory and answers like GitHub does, for the tests
//! of what is built on [`Platform`]. The test plays the other people: `say`, `edit`, `delete_thread`,
//! `delete_comment` and `resolve` change what the platform has, as a colleague in the browser would.

use std::sync::{Arc, Mutex};

use super::checks::{CheckLog, CommitChecks};
use super::discussion::{
    CommentKind, CommentRef, ConversationItem, ConversationKind, DiffSide, Discussion, NewThread, PlatformComment,
    PlatformThreadData, PostedComment, PostedThread, PullRequestState, Submission, Submitted,
};
use super::model::{
    PlatformError, PlatformTraits, Posting, PullRequestDetail, PullRequestListing, PullRequestQuery, Viewer,
};
use super::port::Platform;
use super::remote::RemoteKey;

pub(crate) struct Hosted {
    pub discussion: Discussion,
    next: u64,
    /// What was asked of the platform, in order.
    pub calls: Vec<String>,
    /// Calls whose description starts with the text fail with the error.
    pub failing: Vec<(String, PlatformError)>,
    pub submissions: Vec<Submission>,
    /// The ids of the verdicts the viewer voted, in order.
    pub votes: Vec<String>,
    pub traits: PlatformTraits,
    /// Edit times, so a platform edit is later than any before it.
    clock: i64,
    /// A submission is taken, and its answer never reaches Vet.
    answers_lost: bool,
}

#[derive(Clone)]
pub(crate) struct FakeHosting(pub Arc<Mutex<Hosted>>);

impl Default for FakeHosting {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Hosted {
            discussion: Discussion {
                title: "Add idempotency keys".to_owned(),
                url: "https://github.com/acme/app/pull/7".to_owned(),
                draft: false,
                source_branch: "keys".to_owned(),
                target_branch: "main".to_owned(),
                state: PullRequestState::Open,
                author: "lena".to_owned(),
                author_name: "lena".to_owned(),
                author_avatar: None,
                reviewers: Vec::new(),
                viewer_verdict: None,
                labels: Vec::new(),
                ci: None,
                conversation: Vec::new(),
                threads: Vec::new(),
                complete: true,
            },
            next: 100,
            calls: Vec::new(),
            failing: Vec::new(),
            submissions: Vec::new(),
            votes: Vec::new(),
            traits: PlatformTraits::github(),
            clock: 1_000,
            answers_lost: false,
        })))
    }
}

fn conversation_item(comment: &PlatformComment) -> ConversationItem {
    ConversationItem {
        id: comment.id.clone(),
        author: comment.author.clone(),
        avatar: None,
        at: comment.created_at,
        kind: ConversationKind::Comment { mine: comment.mine },
        body: comment.body.clone(),
    }
}

impl Hosted {
    fn id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    fn tick(&mut self) -> i64 {
        self.clock += 1;
        self.clock
    }

    fn comment(&mut self, author: &str, mine: bool, body: &str, commit: Option<&str>) -> PlatformComment {
        let at = self.tick();
        PlatformComment {
            id: self.id("C"),
            author: author.to_owned(),
            avatar: None,
            body: body.to_owned(),
            created_at: at,
            edited_at: at,
            commit: commit.map(str::to_owned),
            mine,
        }
    }

    fn find(&mut self, id: &str) -> Option<&mut PlatformComment> {
        self.discussion
            .threads
            .iter_mut()
            .flat_map(|t| t.comments.iter_mut())
            .find(|c| c.id == id)
    }

    /// Posts the drafts in turn, as a platform that takes one comment at a time does: each is a
    /// call `post <text>`, the first that fails ends it, and a comment on the whole pull request
    /// is a thread of its own.
    fn post_one_by_one(&mut self, submission: &Submission) -> Submitted {
        let mut posted = Submitted::default();
        let thread_of = |hosted: &mut Self, key: u64, new: Option<&NewThread>, body: &str| {
            hosted.check(format!("post {body}"))?;
            let first = hosted.comment("ada", true, body, Some(&submission.commit));
            let thread = hosted.id("T");
            hosted.discussion.threads.push(PlatformThreadData {
                id: thread.clone(),
                path: new.map(|n| n.path.clone()).unwrap_or_default(),
                side: new.map_or(DiffSide::Right, |n| n.side),
                lines: new.and_then(|n| n.lines),
                outdated: false,
                resolved: false,
                resolved_by: None,
                comments: vec![first.clone()],
            });
            Ok::<_, PlatformError>(PostedThread {
                key,
                thread,
                comment: first,
                replies: Vec::new(),
            })
        };
        let reply = |hosted: &mut Self, thread: &str, key: u64, body: &str| {
            hosted.check(format!("post {body}"))?;
            let comment = hosted.comment("ada", true, body, Some(&submission.commit));
            let found = hosted.discussion.threads.iter_mut().find(|t| t.id == thread);
            found
                .ok_or_else(|| PlatformError::NotFound(format!("no thread {thread}")))?
                .comments
                .push(comment.clone());
            Ok::<_, PlatformError>(PostedComment { key, comment })
        };
        let result = (|| {
            for new in &submission.threads {
                let made = thread_of(self, new.key, Some(new), &new.body)?;
                let thread = made.thread.clone();
                posted.threads.push(made);
                for r in &new.replies {
                    let comment = reply(self, &thread, r.key, &r.body)?;
                    posted.threads.last_mut().expect("pushed").replies.push(comment);
                }
            }
            for r in &submission.replies {
                posted.replies.push(reply(self, &r.thread, r.key, &r.body)?);
            }
            for c in &submission.comments {
                posted.threads.push(thread_of(self, c.key, None, &c.body)?);
            }
            Ok::<_, PlatformError>(())
        })();
        posted.failed = result.err();
        posted
    }

    fn check(&mut self, call: String) -> Result<(), PlatformError> {
        let failure = self
            .failing
            .iter()
            .find(|(prefix, _)| call.starts_with(prefix.as_str()));
        let failure = failure.map(|(_, e)| e.clone());
        self.calls.push(call);
        failure.map_or(Ok(()), Err)
    }
}

impl FakeHosting {
    /// A platform that takes comments one at a time and keeps the vote apart, as Azure DevOps does.
    pub fn per_comment() -> Self {
        let hosting = Self::default();
        hosting.0.lock().unwrap().traits = PlatformTraits::azure_devops();
        hosting
    }

    pub fn votes(&self) -> Vec<String> {
        self.0.lock().unwrap().votes.clone()
    }

    /// A thread of someone else's on lines `first..=last` of `path` at `commit`. Answers with
    /// its id.
    pub fn thread(&self, path: &str, lines: (u32, u32), commit: &str, author: &str, body: &str) -> String {
        self.thread_by(path, lines, commit, author, false, body)
    }

    /// Like [`FakeHosting::thread`], by the viewer, who is `ada`.
    pub fn my_thread(&self, path: &str, lines: (u32, u32), commit: &str, body: &str) -> String {
        self.thread_by(path, lines, commit, "ada", true, body)
    }

    fn thread_by(&self, path: &str, lines: (u32, u32), commit: &str, author: &str, mine: bool, body: &str) -> String {
        let mut hosted = self.0.lock().unwrap();
        let comment = hosted.comment(author, mine, body, Some(commit));
        let id = hosted.id("T");
        hosted.discussion.threads.push(PlatformThreadData {
            id: id.clone(),
            path: path.to_owned(),
            side: DiffSide::Right,
            lines: Some(lines),
            outdated: false,
            resolved: false,
            resolved_by: None,
            comments: vec![comment],
        });
        id
    }

    /// Someone replies in a thread. Answers with the comment's id.
    pub fn say(&self, thread: &str, author: &str, body: &str) -> String {
        let mut hosted = self.0.lock().unwrap();
        let commit = hosted
            .discussion
            .threads
            .iter()
            .find(|t| t.id == thread)
            .and_then(|t| t.comments[0].commit.clone());
        let comment = hosted.comment(author, false, body, commit.as_deref());
        let id = comment.id.clone();
        let found = hosted.discussion.threads.iter_mut().find(|t| t.id == thread);
        found.expect("a thread").comments.push(comment);
        id
    }

    /// A comment on the whole pull request, in its conversation. Answers with its id.
    pub fn say_on_the_pull_request(&self, author: &str, mine: bool, body: &str) -> String {
        let mut hosted = self.0.lock().unwrap();
        let comment = hosted.comment(author, mine, body, None);
        hosted.discussion.conversation.push(conversation_item(&comment));
        comment.id
    }

    /// The texts of the conversation of the pull request, in order.
    pub fn conversation(&self) -> Vec<String> {
        let hosted = self.0.lock().unwrap();
        hosted.discussion.conversation.iter().map(|i| i.body.clone()).collect()
    }

    /// Someone edits a comment in the browser.
    pub fn edit(&self, comment: &str, body: &str) {
        let mut hosted = self.0.lock().unwrap();
        let at = hosted.tick();
        let found = hosted.find(comment).expect("a comment");
        found.body = body.to_owned();
        found.edited_at = at;
    }

    pub fn delete_thread(&self, thread: &str) {
        self.0.lock().unwrap().discussion.threads.retain(|t| t.id != thread);
    }

    /// Someone removes a comment in the browser, which leaves the thread with the others.
    pub fn delete_comment(&self, comment: &str) {
        let mut hosted = self.0.lock().unwrap();
        for t in &mut hosted.discussion.threads {
            t.comments.retain(|c| c.id != comment);
        }
    }

    pub fn resolve(&self, thread: &str, by: Option<&str>) {
        let mut hosted = self.0.lock().unwrap();
        let found = hosted.discussion.threads.iter_mut().find(|t| t.id == thread);
        let found = found.expect("a thread");
        found.resolved = by.is_some();
        found.resolved_by = by.map(str::to_owned);
    }

    /// Calls that start with `prefix` ("" is every call) fail from now on.
    pub fn fail(&self, prefix: &str, error: PlatformError) {
        self.0.lock().unwrap().failing.push((prefix.to_owned(), error));
    }

    pub fn recover(&self) {
        let mut hosted = self.0.lock().unwrap();
        hosted.failing.clear();
        hosted.answers_lost = false;
    }

    /// From now on a submission is posted, and the connection drops before Vet hears so.
    pub fn lose_answers(&self) {
        self.0.lock().unwrap().answers_lost = true;
    }

    pub fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap().calls.clone()
    }

    pub fn body_of(&self, comment: &str) -> Option<String> {
        self.0.lock().unwrap().find(comment).map(|c| c.body.clone())
    }

    pub fn thread_ids(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .discussion
            .threads
            .iter()
            .map(|t| t.id.clone())
            .collect()
    }
}

impl Platform for FakeHosting {
    fn traits(&self) -> PlatformTraits {
        self.0.lock().unwrap().traits.clone()
    }

    async fn viewer(&self, _: &RemoteKey) -> Result<Viewer, PlatformError> {
        Ok(Viewer {
            login: "ada".to_owned(),
            teams: Vec::new(),
        })
    }

    async fn list_pull_requests(
        &self,
        _: &RemoteKey,
        _: &PullRequestQuery,
    ) -> Result<PullRequestListing, PlatformError> {
        unreachable!("the tests of threads list nothing")
    }

    async fn pull_request(&self, _: &RemoteKey, _: u64) -> Result<PullRequestDetail, PlatformError> {
        unreachable!("the tests of threads open nothing")
    }

    async fn discussion(&self, _: &RemoteKey, _: u64) -> Result<Discussion, PlatformError> {
        let mut hosted = self.0.lock().unwrap();
        hosted.check("discussion".to_owned())?;
        Ok(hosted.discussion.clone())
    }

    async fn comment(&self, _: &RemoteKey, comment: &CommentRef) -> Result<PlatformComment, PlatformError> {
        let mut hosted = self.0.lock().unwrap();
        hosted.check(format!("comment {}", comment.id))?;
        hosted
            .find(&comment.id)
            .map(|c| c.clone())
            .ok_or_else(|| PlatformError::NotFound(format!("no comment {}", comment.id)))
    }

    async fn edit_comment(
        &self,
        _: &RemoteKey,
        comment: &CommentRef,
        body: &str,
    ) -> Result<PlatformComment, PlatformError> {
        let mut hosted = self.0.lock().unwrap();
        hosted.check(format!("edit {} {body}", comment.id))?;
        let at = hosted.tick();
        let said = hosted.discussion.conversation.iter_mut().find(|i| i.id == comment.id);
        if let (CommentKind::Conversation, Some(item)) = (comment.kind, said) {
            item.body = body.to_owned();
            return Ok(PlatformComment {
                id: item.id.clone(),
                author: item.author.clone(),
                avatar: None,
                body: item.body.clone(),
                created_at: item.at,
                edited_at: at,
                commit: None,
                mine: true,
            });
        }
        let found = hosted
            .find(&comment.id)
            .ok_or_else(|| PlatformError::NotFound(format!("no comment {}", comment.id)))?;
        found.body = body.to_owned();
        found.edited_at = at;
        Ok(found.clone())
    }

    async fn delete_comment(&self, _: &RemoteKey, comment: &CommentRef) -> Result<(), PlatformError> {
        let mut hosted = self.0.lock().unwrap();
        hosted.check(format!("delete {}", comment.id))?;
        hosted.discussion.conversation.retain(|i| i.id != comment.id);
        if hosted.traits.first_comment_ends_thread {
            hosted.discussion.threads.retain(|t| t.comments[0].id != comment.id);
        }
        for t in &mut hosted.discussion.threads {
            t.comments.retain(|c| c.id != comment.id);
        }
        hosted.discussion.threads.retain(|t| !t.comments.is_empty());
        Ok(())
    }

    async fn set_thread_resolved(&self, _: &RemoteKey, thread: &str, resolved: bool) -> Result<(), PlatformError> {
        let mut hosted = self.0.lock().unwrap();
        hosted.check(format!("resolve {thread} {resolved}"))?;
        let found = hosted.discussion.threads.iter_mut().find(|t| t.id == thread);
        let found = found.ok_or_else(|| PlatformError::NotFound(format!("no thread {thread}")))?;
        found.resolved = resolved;
        found.resolved_by = resolved.then(|| "ada".to_owned());
        Ok(())
    }

    async fn checks(&self, _: &RemoteKey, _: &str) -> Result<CommitChecks, PlatformError> {
        unreachable!("the tests of threads ask for no checks")
    }

    async fn check_log(&self, _: &RemoteKey, _: &str) -> Result<CheckLog, PlatformError> {
        unreachable!("the tests of threads ask for no checks")
    }

    async fn vote(&self, _: &RemoteKey, _: u64, verdict: &str) -> Result<(), PlatformError> {
        let mut hosted = self.0.lock().unwrap();
        hosted.check(format!("vote {verdict}"))?;
        hosted.votes.push(verdict.to_owned());
        hosted.discussion.viewer_verdict = Some(verdict.to_owned());
        Ok(())
    }

    async fn submit_review(&self, _: &RemoteKey, _: u64, submission: &Submission) -> Result<Submitted, PlatformError> {
        let mut hosted = self.0.lock().unwrap();
        let posted = hosted.take(submission)?;
        if hosted.answers_lost {
            return Err(PlatformError::Unreachable("the connection dropped".to_owned()));
        }
        Ok(posted)
    }
}

impl Hosted {
    fn take(&mut self, submission: &Submission) -> Result<Submitted, PlatformError> {
        let hosted = self;
        hosted.check(format!("submit {}", submission.verdict))?;
        hosted.submissions.push(submission.clone());
        if hosted.traits.posting == Posting::PerComment {
            return Ok(hosted.post_one_by_one(submission));
        }
        let mut posted = Submitted::default();
        for new in &submission.threads {
            let first = hosted.comment("ada", true, &new.body, Some(&submission.commit));
            let thread = hosted.id("T");
            let mut comments = vec![first.clone()];
            let mut replies = Vec::new();
            for reply in &new.replies {
                let comment = hosted.comment("ada", true, &reply.body, Some(&submission.commit));
                comments.push(comment.clone());
                replies.push(PostedComment {
                    key: reply.key,
                    comment,
                });
            }
            hosted.discussion.threads.push(PlatformThreadData {
                id: thread.clone(),
                path: new.path.clone(),
                side: new.side,
                lines: new.lines,
                outdated: false,
                resolved: false,
                resolved_by: None,
                comments,
            });
            posted.threads.push(PostedThread {
                key: new.key,
                thread,
                comment: first,
                replies,
            });
        }
        for reply in &submission.replies {
            let commit = hosted
                .discussion
                .threads
                .iter()
                .find(|t| t.id == reply.thread)
                .ok_or_else(|| PlatformError::NotFound(format!("no thread {}", reply.thread)))?
                .comments[0]
                .commit
                .clone();
            let comment = hosted.comment("ada", true, &reply.body, commit.as_deref());
            let found = hosted.discussion.threads.iter_mut().find(|t| t.id == reply.thread);
            found.expect("checked").comments.push(comment.clone());
            posted.replies.push(PostedComment {
                key: reply.key,
                comment,
            });
        }
        for new in &submission.comments {
            if let Err(error) = hosted.check("flat comment".to_owned()) {
                posted.failed = Some(error);
                break;
            }
            let comment = hosted.comment("ada", true, &new.body, None);
            hosted.discussion.conversation.push(conversation_item(&comment));
            posted.comments.push(PostedComment { key: new.key, comment });
        }
        Ok(posted)
    }
}
