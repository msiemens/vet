//! A platform that holds one pull request in memory and answers like GitHub does, for the tests
//! of what is built on [`Platform`]. The test plays the other people: `say`, `edit`, `delete_thread`
//! and `resolve` change what the platform has, as a colleague in the browser would.

use std::sync::{Arc, Mutex};

use super::discussion::{
    CommentRef, DiffSide, Discussion, PlatformComment, PlatformThreadData, PostedComment, PostedThread,
    PullRequestState, Submission, Submitted,
};
use super::model::{PlatformError, PlatformTraits, PullRequest, PullRequestDetail, PullRequestQuery, Viewer};
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
    /// Edit times, so a platform edit is later than any before it.
    clock: i64,
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
                author_avatar: None,
                reviewers: Vec::new(),
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
            clock: 1_000,
        })))
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
        self.0.lock().unwrap().failing.clear();
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
        PlatformTraits::github()
    }

    async fn viewer(&self, _: &RemoteKey) -> Result<Viewer, PlatformError> {
        Ok(Viewer {
            login: "ada".to_owned(),
            teams: Vec::new(),
        })
    }

    async fn list_pull_requests(&self, _: &RemoteKey, _: &PullRequestQuery) -> Result<Vec<PullRequest>, PlatformError> {
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
        // The first comment of a thread is the thread.
        hosted.discussion.threads.retain(|t| t.comments[0].id != comment.id);
        for t in &mut hosted.discussion.threads {
            t.comments.retain(|c| c.id != comment.id);
        }
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

    async fn submit_review(&self, _: &RemoteKey, _: u64, submission: &Submission) -> Result<Submitted, PlatformError> {
        let mut hosted = self.0.lock().unwrap();
        hosted.check(format!("submit {}", submission.verdict))?;
        hosted.submissions.push(submission.clone());
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
            let comment = hosted.comment("ada", true, &new.body, None);
            posted.comments.push(PostedComment { key: new.key, comment });
        }
        Ok(posted)
    }
}
