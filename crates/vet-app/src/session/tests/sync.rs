//! Threads of a pull request exchanged with its platform, against a platform kept in memory.

use vet_core::review::{EditConflict, Outgoing};

use super::*;
use crate::platform::{DiffSide, PlatformError};
use crate::session::VoteSent;

/// Pull request 7, opened: its head `pr1` has `a.txt` edited at lines 3 and 20 (0-based).
async fn on_a_pull_request() -> (Fixture, FakeHosting) {
    let base = numbered(30, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fetch_pull_request_7(
        &fx.git,
        &[("a.txt", &base)],
        "pr1",
        &[("a.txt", &numbered(30, &[3, 20]))],
    );
    fx.session.open_changeset(pull_request_7()).await.unwrap();
    (fx, FakeHosting::default())
}

/// Like [`on_a_pull_request`], on Azure DevOps.
async fn on_an_azure_devops_pull_request() -> (Fixture, FakeHosting) {
    let remote = RemoteKey {
        host: "dev.azure.com".into(),
        path: "acme/shop/app".into(),
    };
    let base = numbered(30, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    let refs = PullRequestRefs::of(&remote, 7);
    fx.git
        .add_commit(&refs.head, "pr1", &[("a.txt", &numbered(30, &[3, 20]))]);
    fx.git
        .add_commit(&format!("merge-base {}", refs.target), "m1", &[("a.txt", &base)]);
    let source = Source::PullRequest { remote, number: 7 };
    fx.session.open_changeset(source).await.unwrap();
    (fx, FakeHosting::per_comment())
}

impl Fixture {
    async fn all(&self) -> Vec<ThreadView> {
        self.session.threads(View::Changeset).await.unwrap().threads
    }

    async fn sync(&self, hosting: &FakeHosting) -> SyncReport {
        self.session.sync_threads(hosting).await.unwrap()
    }
}

fn bodies(thread: &ThreadView) -> Vec<&str> {
    thread.messages.iter().map(|m| m.body.as_str()).collect()
}

fn remote_id(thread: &ThreadView) -> Option<&str> {
    thread.platform.as_ref()?.remote_id.as_deref()
}

/// The texts of the conversation of the pull request on view, as Vet knows it.
async fn conversation(fx: &Fixture) -> Vec<String> {
    let view = fx.session.discussion().await.expect("synced");
    view.discussion.conversation.iter().map(|i| i.body.clone()).collect()
}

#[tokio::test]
async fn the_platforms_threads_are_pulled_in_where_they_were_made_with_their_ids_and_authors() {
    let (fx, hosting) = on_a_pull_request().await;
    let id = hosting.thread("a.txt", (21, 21), "pr1", "marco", "Why 20?");
    hosting.say(&id, "lena", "Because.");

    let report = fx.sync(&hosting).await;

    assert!(report.changed && report.problem.is_none());
    let [thread] = <[ThreadView; 1]>::try_from(fx.all().await).expect("one thread");
    assert_eq!(
        (thread.state.clone(), thread.location.clone()),
        (Some(EXACT), at("a.txt", 20, 21))
    );
    assert_eq!(bodies(&thread), ["Why 20?", "Because."]);
    assert_eq!(
        thread
            .messages
            .iter()
            .map(|m| (m.author.as_str(), m.mine))
            .collect::<Vec<_>>(),
        [("marco", false), ("lena", false)]
    );
    assert_eq!(remote_id(&thread), Some(id.as_str()));
    assert!(
        thread.messages.iter().all(|m| m.remote.is_some()),
        "nothing of it is a draft"
    );

    assert!(!fx.sync(&hosting).await.changed, "a second sync has nothing to change");
    assert_eq!(fx.all().await.len(), 1);
    assert!(
        matches!(fx.session.lifecycle().await, Lifecycle::Browsing),
        "reading what others said is no review action"
    );
}

#[tokio::test]
async fn a_comment_on_older_code_is_anchored_there_and_follows_the_code_to_where_it_went() {
    let base = numbered(30, &[]);
    let first = numbered(30, &[20]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fetch_pull_request_7(&fx.git, &[("a.txt", &base)], "pr1", &[("a.txt", &first)]);
    fx.git.add_commit("an earlier push", "pr1", &[("a.txt", &first)]);
    fx.session.open_changeset(pull_request_7()).await.unwrap();
    // The author pushed again, adding two lines at the top.
    fetch_pull_request_7(
        &fx.git,
        &[("a.txt", &base)],
        "pr2",
        &[("a.txt", &format!("new 1\nnew 2\n{first}"))],
    );
    fx.session.observe().await.unwrap().expect("pushed to");
    let hosting = FakeHosting::default();
    hosting.thread("a.txt", (21, 21), "pr1", "marco", "Why 20?");

    fx.sync(&hosting).await;

    let [thread] = <[ThreadView; 1]>::try_from(fx.all().await).expect("one thread");
    assert_eq!((thread.state, thread.location), (Some(EXACT), at("a.txt", 22, 23)));
    let CommentTarget::Range(recorded) = thread.target else {
        panic!("not a range")
    };
    assert_eq!(
        recorded.range,
        LineRange { start: 20, end: 21 },
        "recorded where it was made"
    );
}

#[tokio::test]
async fn a_comment_whose_code_cannot_be_found_is_attached_to_its_file() {
    let (fx, hosting) = on_a_pull_request().await;
    hosting.thread("a.txt", (500, 500), "pr1", "marco", "Past the end.");
    hosting.thread("gone.txt", (1, 1), "pr1", "marco", "No such file.");

    fx.sync(&hosting).await;

    let kinds: Vec<_> = fx
        .all()
        .await
        .into_iter()
        .map(|t| match t.target {
            CommentTarget::File { path: p, .. } => p.to_string_lossy(),
            CommentTarget::Changeset => "pull request".to_owned(),
            CommentTarget::Range(_) => "range".to_owned(),
        })
        .collect();
    assert_eq!(kinds, ["a.txt", "pull request"]);
}

#[tokio::test]
async fn a_status_the_viewer_changed_since_the_last_sync_wins_and_otherwise_the_platforms_is_taken() {
    let (fx, hosting) = on_a_pull_request().await;
    let id = hosting.thread("a.txt", (4, 4), "pr1", "marco", "Why 3?");
    fx.sync(&hosting).await;

    hosting.resolve(&id, Some("lena"));
    fx.sync(&hosting).await;
    let thread = fx.all().await.remove(0);
    assert_eq!(thread.status, ThreadStatus::Resolved);
    assert_eq!(thread.platform.as_ref().unwrap().resolved_by.as_deref(), Some("lena"));

    // Reopened here while the platform still has it resolved: the viewer's choice is sent.
    fx.session
        .set_thread_status(thread.id, ThreadStatus::Open)
        .await
        .unwrap();
    assert_eq!(fx.sync(&hosting).await.problem, None);
    assert!(hosting.calls().contains(&format!("resolve {id} false")));
    let thread = fx.all().await.remove(0);
    assert_eq!(thread.status, ThreadStatus::Open);
    assert!(!thread.platform.as_ref().unwrap().status_pending);

    // Nothing was changed here since: what the platform says goes.
    hosting.resolve(&id, Some("lena"));
    fx.sync(&hosting).await;
    assert_eq!(fx.all().await[0].status, ThreadStatus::Resolved);
    hosting.resolve(&id, None);
    fx.sync(&hosting).await;
    assert_eq!(fx.all().await[0].status, ThreadStatus::Open);
}

#[tokio::test]
async fn a_status_that_cannot_be_sent_stays_the_viewers_and_is_reported() {
    let (fx, hosting) = on_a_pull_request().await;
    let id = hosting.thread("a.txt", (4, 4), "pr1", "marco", "Why 3?");
    fx.sync(&hosting).await;
    let thread = fx.all().await.remove(0);
    fx.session
        .set_thread_status(thread.id, ThreadStatus::Resolved)
        .await
        .unwrap();
    hosting.fail("resolve", PlatformError::Forbidden("no".to_owned()));

    let report = fx.sync(&hosting).await;

    assert_eq!(report.problem, Some(PlatformError::Forbidden("no".to_owned())));
    let thread = fx.all().await.remove(0);
    assert_eq!(thread.status, ThreadStatus::Resolved);
    assert!(thread.platform.as_ref().unwrap().status_pending, "still to be sent");

    hosting.recover();
    fx.sync(&hosting).await;
    assert!(hosting.calls().contains(&format!("resolve {id} true")));
    assert!(!fx.all().await[0].platform.as_ref().unwrap().status_pending);
}

#[tokio::test]
async fn an_edit_of_a_posted_comment_is_sent_at_once() {
    let (fx, hosting) = on_a_pull_request().await;
    hosting.my_thread("a.txt", (21, 21), "pr1", "Keys expire after 24 h.");
    fx.sync(&hosting).await;
    let message = fx.all().await[0].messages[0].clone();
    let comment = message.remote.as_ref().unwrap().id.clone();

    let view = fx
        .session
        .edit_message(&hosting, message.id, "Keys expire after 48 h.".into(), 500)
        .await
        .unwrap();

    assert_eq!(hosting.body_of(&comment).as_deref(), Some("Keys expire after 48 h."));
    let edited = &view.messages[0];
    assert_eq!(edited.body, "Keys expire after 48 h.");
    assert_eq!(edited.edited_at, Some(500));
    let remote = edited.remote.as_ref().unwrap();
    assert_eq!(
        (remote.synced.as_str(), remote.conflict.clone()),
        ("Keys expire after 48 h.", None)
    );
    assert!(!fx.sync(&hosting).await.changed, "nothing left to send");
}

#[tokio::test]
async fn an_edit_that_could_not_be_sent_is_sent_by_the_next_sync() {
    let (fx, hosting) = on_a_pull_request().await;
    hosting.my_thread("a.txt", (21, 21), "pr1", "old");
    fx.sync(&hosting).await;
    let message = fx.all().await[0].messages[0].clone();
    let comment = message.remote.as_ref().unwrap().id.clone();
    hosting.fail("edit", PlatformError::Unreachable("offline".to_owned()));

    let error = fx
        .session
        .edit_message(&hosting, message.id, "new".into(), 500)
        .await
        .unwrap_err();

    assert!(
        matches!(error, SessionError::Platform(PlatformError::Unreachable(_))),
        "{error}"
    );
    let kept = fx.all().await[0].messages[0].clone();
    assert_eq!(
        (kept.body.as_str(), kept.remote.unwrap().synced.as_str()),
        ("new", "old")
    );
    assert_eq!(hosting.body_of(&comment).as_deref(), Some("old"));

    hosting.recover();
    fx.sync(&hosting).await;
    assert_eq!(hosting.body_of(&comment).as_deref(), Some("new"));
    let sent = fx.all().await[0].messages[0].clone();
    assert_eq!(sent.remote.unwrap().synced, "new");
}

#[tokio::test]
async fn a_comment_edited_in_both_places_keeps_both_texts_until_one_is_chosen() {
    let (fx, hosting) = on_a_pull_request().await;
    hosting.my_thread("a.txt", (21, 21), "pr1", "24 h");
    fx.sync(&hosting).await;
    let message = fx.all().await[0].messages[0].clone();
    let comment = message.remote.as_ref().unwrap().id.clone();

    hosting.edit(&comment, "24 h and the weekend backfill");
    let view = fx
        .session
        .edit_message(&hosting, message.id, "48 h".into(), 500)
        .await
        .unwrap();

    let held = &view.messages[0];
    assert_eq!(held.body, "48 h");
    assert!(
        matches!(&held.remote.as_ref().unwrap().conflict, Some(EditConflict { remote_body, .. }) if remote_body == "24 h and the weekend backfill")
    );
    assert_eq!(
        hosting.body_of(&comment).as_deref(),
        Some("24 h and the weekend backfill"),
        "nothing was sent"
    );
    let again = fx.session.edit_message(&hosting, message.id, "72 h".into(), 501).await;
    assert!(
        matches!(again, Err(SessionError::Invalid(_))),
        "one text must be kept first"
    );

    // Keeping the platform's text drops the viewer's.
    let view = fx.session.keep_edit(&hosting, message.id, Keep::Remote).await.unwrap();
    assert_eq!(view.messages[0].body, "24 h and the weekend backfill");
    assert_eq!(view.messages[0].remote.as_ref().unwrap().conflict, None);
    assert!(!fx.sync(&hosting).await.changed);
}

#[tokio::test]
async fn keeping_the_viewers_text_sends_it() {
    let (fx, hosting) = on_a_pull_request().await;
    hosting.my_thread("a.txt", (21, 21), "pr1", "24 h");
    fx.sync(&hosting).await;
    let message = fx.all().await[0].messages[0].clone();
    let comment = message.remote.as_ref().unwrap().id.clone();
    hosting.edit(&comment, "theirs");
    fx.session
        .edit_message(&hosting, message.id, "mine".into(), 500)
        .await
        .unwrap();

    let view = fx.session.keep_edit(&hosting, message.id, Keep::Vet).await.unwrap();

    assert_eq!(hosting.body_of(&comment).as_deref(), Some("mine"));
    assert_eq!(view.messages[0].remote.as_ref().unwrap().conflict, None);
    assert_eq!(view.messages[0].body, "mine");
}

#[tokio::test]
async fn a_sync_finds_an_edit_made_in_both_places_while_apart() {
    let (fx, hosting) = on_a_pull_request().await;
    hosting.my_thread("a.txt", (21, 21), "pr1", "24 h");
    fx.sync(&hosting).await;
    let message = fx.all().await[0].messages[0].clone();
    let comment = message.remote.as_ref().unwrap().id.clone();
    hosting.fail("edit", PlatformError::Unreachable("offline".to_owned()));
    fx.session
        .edit_message(&hosting, message.id, "48 h".into(), 500)
        .await
        .unwrap_err();
    hosting.recover();
    hosting.edit(&comment, "36 h");

    fx.sync(&hosting).await;

    let held = fx.all().await[0].messages[0].clone();
    assert_eq!(held.body, "48 h");
    assert_eq!(
        held.remote.unwrap().conflict.map(|c| c.remote_body),
        Some("36 h".to_owned())
    );
    assert_eq!(hosting.body_of(&comment).as_deref(), Some("36 h"));
}

#[tokio::test]
async fn an_edit_made_only_on_the_platform_is_taken_over() {
    let (fx, hosting) = on_a_pull_request().await;
    hosting.thread("a.txt", (21, 21), "pr1", "marco", "typo");
    fx.sync(&hosting).await;
    let comment = fx.all().await[0].messages[0].remote.as_ref().unwrap().id.clone();
    hosting.edit(&comment, "fixed");

    fx.sync(&hosting).await;

    assert_eq!(bodies(&fx.all().await[0]), ["fixed"]);
}

#[tokio::test]
async fn a_thread_deleted_on_the_platform_is_removed_and_a_draft_in_it_is_reported_with_its_text() {
    let (fx, hosting) = on_a_pull_request().await;
    let kept = hosting.thread("a.txt", (4, 4), "pr1", "marco", "stays");
    let doomed = hosting.thread("a.txt", (21, 21), "pr1", "marco", "goes");
    let quiet = hosting.thread("a.txt", (10, 10), "pr1", "marco", "goes too");
    fx.sync(&hosting).await;
    let on_doomed = fx
        .all()
        .await
        .into_iter()
        .find(|t| remote_id(t) == Some(doomed.as_str()))
        .unwrap();
    fx.session
        .reply(on_doomed.id, "Then use 48 h.".into(), 5)
        .await
        .unwrap();
    hosting.delete_thread(&doomed);
    hosting.delete_thread(&quiet);

    let report = fx.sync(&hosting).await;

    assert_eq!(
        report.notices,
        [SyncNotice::ThreadDeleted {
            place: "a.txt:21".to_owned(),
            dropped: vec!["Then use 48 h.".to_owned()]
        }],
        "only the thread with a draft in it is announced"
    );
    let left = fx.all().await;
    assert_eq!(
        left.iter().map(|t| remote_id(t)).collect::<Vec<_>>(),
        [Some(kept.as_str())]
    );
}

#[tokio::test]
async fn a_partial_listing_removes_no_thread() {
    let (fx, hosting) = on_a_pull_request().await;
    hosting.thread("a.txt", (4, 4), "pr1", "marco", "here");
    fx.sync(&hosting).await;
    {
        let mut hosted = hosting.0.lock().unwrap();
        hosted.discussion.threads.clear();
        hosted.discussion.complete = false;
    }

    fx.sync(&hosting).await;

    assert_eq!(fx.all().await.len(), 1);
}

#[tokio::test]
async fn a_sync_that_cannot_ask_the_platform_changes_nothing_and_says_why() {
    let (fx, hosting) = on_a_pull_request().await;
    hosting.thread("a.txt", (4, 4), "pr1", "marco", "here");
    fx.sync(&hosting).await;
    hosting.fail("", PlatformError::SignedOut("github.com".to_owned()));
    let local = fx.thread(on("a.txt", 20, 21)).await;
    fx.session.reply(local, "a draft".into(), 5).await.unwrap();

    let error = fx.session.sync_threads(&hosting).await.unwrap_err();

    assert!(
        matches!(error, SessionError::Platform(PlatformError::SignedOut(_))),
        "{error}"
    );
    assert_eq!(fx.all().await.len(), 2, "drafts and what was pulled stay");
}

#[tokio::test]
async fn drafts_stay_local_until_the_review_is_submitted_as_one_with_its_verdict() {
    let (fx, hosting) = on_a_pull_request().await;
    let theirs = hosting.thread("a.txt", (4, 4), "pr1", "marco", "Why 3?");
    fx.sync(&hosting).await;
    let on_theirs = fx.all().await[0].id;
    let new_thread = fx.thread(on("a.txt", 20, 21)).await;
    fx.session.reply(new_thread, "and why not 21?".into(), 6).await.unwrap();
    fx.session.reply(on_theirs, "Agreed.".into(), 7).await.unwrap();
    let overall = fx.thread(NewThread::Changeset).await;
    assert!(
        hosting.calls().iter().all(|c| !c.starts_with("submit")),
        "nothing is posted by writing"
    );
    assert!(
        fx.all()
            .await
            .iter()
            .any(|t| t.messages.iter().any(|m| m.remote.is_none()))
    );

    let report = fx
        .session
        .submit_review(&hosting, "REQUEST_CHANGES", Some("Needs a TTL.".into()))
        .await
        .unwrap();

    assert_eq!((report.posted, report.skipped, report.problem), (4, 0, None));
    let submissions = hosting.0.lock().unwrap().submissions.clone();
    let [submission] = <[_; 1]>::try_from(submissions).expect("one review");
    assert_eq!(
        (
            submission.verdict.as_str(),
            submission.body.as_deref(),
            submission.commit.as_str()
        ),
        ("REQUEST_CHANGES", Some("Needs a TTL."), "pr1")
    );
    let [posted_thread] = <[_; 1]>::try_from(submission.threads.clone()).expect("one new thread");
    assert_eq!(
        (
            posted_thread.path.as_str(),
            posted_thread.side,
            posted_thread.lines,
            posted_thread.body.as_str()
        ),
        ("a.txt", DiffSide::Right, Some((21, 21)), "why?")
    );
    assert_eq!(
        posted_thread
            .replies
            .iter()
            .map(|r| r.body.as_str())
            .collect::<Vec<_>>(),
        ["and why not 21?"]
    );
    assert_eq!(
        submission
            .replies
            .iter()
            .map(|r| (r.thread.as_str(), r.body.as_str()))
            .collect::<Vec<_>>(),
        [(theirs.as_str(), "Agreed.")]
    );
    assert_eq!(submission.comments.len(), 1);

    // Everything is the platform's now, and a sync finds nothing to add or change. The comment on
    // the whole pull request is in its conversation, and no thread of Vet's.
    let all = fx.all().await;
    assert!(all.iter().all(|t| t.messages.iter().all(|m| m.remote.is_some())));
    assert!(all.iter().all(|t| t.id != overall));
    assert!(
        all.iter()
            .filter(|t| t.id == new_thread)
            .all(|t| remote_id(t).is_some())
    );
    assert_eq!(conversation(&fx).await, ["why?"]);
    let again = fx.sync(&hosting).await;
    assert!(!again.changed, "no thread is added twice: {again:?}");
    assert_eq!(fx.all().await.len(), 2);
    assert_eq!(conversation(&fx).await, ["why?"]);
}

#[tokio::test]
async fn a_draft_on_code_that_is_gone_and_one_in_a_thread_with_an_edit_conflict_wait() {
    let (fx, hosting) = on_a_pull_request().await;
    hosting.my_thread("a.txt", (4, 4), "pr1", "24 h");
    fx.sync(&hosting).await;
    let mine = fx.all().await[0].clone();
    let comment = mine.messages[0].remote.as_ref().unwrap().id.clone();
    hosting.edit(&comment, "theirs");
    fx.session
        .edit_message(&hosting, mine.messages[0].id, "mine".into(), 5)
        .await
        .unwrap();
    fx.session.reply(mine.id, "a reply that waits".into(), 6).await.unwrap();
    // A thread on a line that the next push removes.
    fx.thread(on("a.txt", 20, 21)).await;
    let base = numbered(30, &[]);
    fetch_pull_request_7(&fx.git, &[("a.txt", &base)], "pr2", &[("a.txt", &numbered(10, &[3]))]);
    fx.session.observe().await.unwrap().expect("pushed to");

    let report = fx
        .session
        .submit_review(&hosting, "COMMENT", Some("Only this.".into()))
        .await
        .unwrap();

    assert_eq!((report.posted, report.skipped), (0, 2));
    let hosted = hosting.0.lock().unwrap();
    assert!(hosted.submissions[0].threads.is_empty() && hosted.submissions[0].replies.is_empty());
}

#[tokio::test]
async fn an_empty_comment_review_is_refused_and_a_verdict_the_platform_lacks_too() {
    let (fx, hosting) = on_a_pull_request().await;

    let empty = fx.session.submit_review(&hosting, "COMMENT", None).await;
    let unknown = fx.session.submit_review(&hosting, "REJECT", None).await;

    assert!(matches!(empty, Err(SessionError::Invalid(_))));
    assert!(matches!(unknown, Err(SessionError::Invalid(_))));
    assert!(hosting.calls().is_empty());
    // A verdict needs no comments.
    assert_eq!(
        fx.session
            .submit_review(&hosting, "APPROVE", None)
            .await
            .unwrap()
            .posted,
        0
    );
}

/// The bodies of the viewer's comments that the platform does not have, with how far each is.
async fn outgoing(fx: &Fixture) -> Vec<(String, Outgoing)> {
    fx.all()
        .await
        .iter()
        .flat_map(|t| t.messages.iter().filter(|m| m.remote.is_none()))
        .map(|m| (m.body.clone(), m.outgoing.clone()))
        .collect()
}

/// The same, as the store has them.
fn stored_outgoing(fx: &Fixture) -> Vec<(String, Outgoing)> {
    let mem = fx.store.0.lock().unwrap();
    mem.threads
        .iter()
        .flat_map(|t| t.thread.messages.iter().filter(|m| m.remote.is_none()))
        .map(|m| (m.body.clone(), m.outgoing.clone()))
        .collect()
}

fn failed(error: &str) -> Outgoing {
    Outgoing::Failed {
        error: error.to_owned(),
    }
}

#[tokio::test]
async fn a_review_the_platform_refuses_posts_nothing_and_fails_each_draft_with_the_reason_until_posted_again() {
    let (fx, hosting) = on_a_pull_request().await;
    fx.thread(on("a.txt", 20, 21)).await;
    let reply = fx.thread(on("a.txt", 3, 4)).await;
    fx.session.reply(reply, "and this".into(), 6).await.unwrap();
    hosting.fail("submit", PlatformError::Rejected("token expired".to_owned()));

    let report = fx.session.submit_review(&hosting, "APPROVE", None).await.unwrap();

    assert_eq!(
        (report.posted, report.failed, report.problem),
        (0, 3, Some(PlatformError::Rejected("token expired".to_owned())))
    );
    let expected = [
        ("why?".to_owned(), failed("token expired")),
        ("why?".to_owned(), failed("token expired")),
        ("and this".to_owned(), failed("token expired")),
    ];
    assert_eq!(outgoing(&fx).await, expected);
    assert_eq!(stored_outgoing(&fx), expected, "kept across a restart");
    assert!(hosting.thread_ids().is_empty());

    // A failed draft is posted with the next review.
    hosting.recover();
    let report = fx.session.submit_review(&hosting, "APPROVE", None).await.unwrap();
    assert_eq!((report.posted, report.failed, report.problem), (3, 0, None));
    assert!(outgoing(&fx).await.is_empty());
    assert_eq!(hosting.thread_ids().len(), 2);
}

#[tokio::test]
async fn a_failed_draft_is_discarded_alone_and_the_rest_post_on_retry() {
    let (fx, hosting) = on_a_pull_request().await;
    fx.thread(on("a.txt", 20, 21)).await;
    let reply = fx.thread(on("a.txt", 3, 4)).await;
    fx.session.reply(reply, "and this".into(), 6).await.unwrap();
    hosting.fail("submit", PlatformError::Rejected("line not in the diff".to_owned()));
    fx.session.submit_review(&hosting, "COMMENT", None).await.unwrap();
    let refused = fx.all().await.iter().find(|t| t.id == reply).unwrap().messages[1].clone();
    assert_eq!(refused.outgoing, failed("line not in the diff"));

    fx.session.remove_message(&hosting, refused.id).await.unwrap();
    assert_eq!(bodies(&fx.view(reply).await), ["why?"]);

    hosting.recover();
    let report = fx.session.submit_review(&hosting, "COMMENT", None).await.unwrap();
    assert_eq!((report.posted, report.failed), (2, 0));
    assert!(outgoing(&fx).await.is_empty());
}

#[tokio::test]
async fn a_draft_being_posted_cannot_be_removed() {
    let (fx, hosting) = on_a_pull_request().await;
    let thread = fx.thread(on("a.txt", 20, 21)).await;
    hosting.lose_answers();
    fx.session.submit_review(&hosting, "COMMENT", None).await.unwrap();
    let posting = fx.view(thread).await.messages[0].clone();
    assert!(matches!(posting.outgoing, Outgoing::Posting { .. }));

    let refused = fx.session.remove_message(&hosting, posting.id).await;

    assert!(matches!(refused, Err(SessionError::Invalid(_))), "{refused:?}");
    assert_eq!(bodies(&fx.view(thread).await), ["why?"]);
}

#[tokio::test]
async fn drafts_are_marked_posting_before_the_platform_is_asked_and_one_it_took_is_found_there_after_a_restart() {
    let (fx, hosting) = on_a_pull_request().await;
    let theirs = hosting.thread("a.txt", (4, 4), "pr1", "marco", "Why 3?");
    fx.sync(&hosting).await;
    let on_theirs = fx.all().await[0].id;
    fx.session.reply(on_theirs, "Agreed.".into(), 7).await.unwrap();
    let new_thread = fx.thread(on("a.txt", 20, 21)).await;
    // Written after the thread was opened, whenever the clock says that was.
    fx.session.reply(new_thread, "and 21".into(), 10_000).await.unwrap();
    fx.thread(NewThread::Changeset).await;
    // The platform posts the review, and Vet stops before it hears so.
    hosting.lose_answers();

    let report = fx.session.submit_review(&hosting, "COMMENT", None).await.unwrap();

    assert!(matches!(report.problem, Some(PlatformError::Unreachable(_))));
    assert_eq!((report.posted, report.failed), (0, 0));
    assert!(
        stored_outgoing(&fx)
            .iter()
            .all(|(_, o)| matches!(o, Outgoing::Posting { .. })),
        "{:?}",
        stored_outgoing(&fx)
    );
    let refused = fx.session.submit_review(&hosting, "COMMENT", None).await;
    assert!(
        matches!(refused, Err(SessionError::Invalid(_))),
        "what may be posted is not posted again: {refused:?}"
    );

    hosting.recover();
    let session = fx.reopen(&[("a.txt", &numbered(30, &[]))]).await;
    session.open_changeset(pull_request_7()).await.unwrap();
    let report = session.sync_threads(&hosting).await.unwrap();

    assert!(report.changed);
    let threads = session.threads(View::Changeset).await.unwrap().threads;
    assert!(
        threads
            .iter()
            .all(|t| t.messages.iter().all(|m| m.remote.is_some()) && remote_id(t).is_some()),
        "{threads:?}"
    );
    assert_eq!(
        threads.len(),
        2,
        "the comment on the pull request is in its conversation"
    );
    let new = threads.iter().find(|t| t.id == new_thread).expect("the new thread");
    assert_eq!(bodies(new), ["why?", "and 21"]);
    let mut on_platform = hosting.thread_ids();
    on_platform.sort();
    assert_eq!(on_platform.len(), 2);
    assert!(on_platform.contains(&theirs));
    assert_eq!(hosting.conversation(), ["why?"]);
    assert!(
        !session.sync_threads(&hosting).await.unwrap().changed,
        "nothing is added twice"
    );
    let again = session.submit_review(&hosting, "COMMENT", None).await;
    assert!(
        matches!(again, Err(SessionError::Invalid(_))),
        "nothing is left to post"
    );
}

#[tokio::test]
async fn a_draft_left_posting_that_the_platform_never_got_is_a_draft_again_after_a_restart() {
    let (fx, hosting) = on_an_azure_devops_pull_request().await;
    fx.thread(on("a.txt", 20, 21)).await;
    fx.thread(NewThread::Changeset).await;
    hosting.fail("submit", PlatformError::Unreachable("offline".to_owned()));

    fx.session.post_drafts(&hosting).await.unwrap();
    assert!(
        stored_outgoing(&fx)
            .iter()
            .all(|(_, o)| matches!(o, Outgoing::Posting { .. }))
    );

    hosting.recover();
    let session = fx.reopen(&[("a.txt", &numbered(30, &[]))]).await;
    session
        .open_changeset(Source::PullRequest {
            remote: RemoteKey {
                host: "dev.azure.com".into(),
                path: "acme/shop/app".into(),
            },
            number: 7,
        })
        .await
        .unwrap();
    assert!(session.sync_threads(&hosting).await.unwrap().changed);
    assert_eq!(
        stored_outgoing(&fx),
        [
            ("why?".to_owned(), Outgoing::Draft),
            ("why?".to_owned(), Outgoing::Draft)
        ]
    );
    assert!(hosting.thread_ids().is_empty());

    let report = session.post_drafts(&hosting).await.unwrap();
    assert_eq!((report.posted, report.problem), (2, None));
    assert_eq!(hosting.thread_ids().len(), 2);
}

#[tokio::test]
async fn a_comment_on_the_whole_pull_request_the_platform_refuses_is_reported_with_its_text_and_stays_a_draft() {
    let (fx, hosting) = on_a_pull_request().await;
    fx.thread(on("a.txt", 20, 21)).await;
    let overall = fx.thread(NewThread::Changeset).await;
    hosting.fail("flat comment", PlatformError::Forbidden("no access".to_owned()));

    let report = fx.session.submit_review(&hosting, "COMMENT", None).await.unwrap();

    assert!(matches!(report.problem, Some(PlatformError::Forbidden(_))));
    assert_eq!(report.not_posted, ["why?"]);
    assert_eq!(report.posted, 1, "the review itself went through");
    let all = fx.all().await;
    let kept = all.iter().find(|t| t.id == overall).expect("the thread");
    assert!(kept.messages.iter().all(|m| m.remote.is_none()));
}

#[tokio::test]
async fn a_flat_comment_on_the_whole_pull_request_takes_no_reply_and_a_draft_in_one_is_never_a_reply_of_the_review() {
    let (fx, hosting) = on_a_pull_request().await;
    let overall = fx.thread(NewThread::Changeset).await;

    let refused = fx.session.reply(overall, "One more thing.".into(), 108).await;
    assert!(matches!(refused, Err(SessionError::Invalid(_))), "{refused:?}");
    assert_eq!(bodies(&fx.view(overall).await), ["why?"]);

    // A draft such a comment holds from before replies were refused there.
    fx.store
        .add_message(overall, Message::written("ada".into(), "One more thing.".into(), 108))
        .await
        .unwrap();
    hosting.thread("a.txt", (4, 4), "pr1", "marco", "Why 3?");
    fx.sync(&hosting).await;
    fx.thread(on("a.txt", 20, 21)).await;

    let report = fx.session.submit_review(&hosting, "COMMENT", None).await.unwrap();

    assert_eq!((report.posted, report.problem), (3, None), "the review went through");
    let submission = hosting.0.lock().unwrap().submissions[0].clone();
    assert!(submission.replies.is_empty(), "{submission:?}");
    assert_eq!(submission.threads.len(), 1);
    assert_eq!(
        submission.comments.iter().map(|c| c.body.as_str()).collect::<Vec<_>>(),
        ["why?", "One more thing."]
    );
    assert!(fx.all().await.iter().all(|t| t.id != overall));
    assert_eq!(conversation(&fx).await, ["why?", "One more thing."]);
}

#[tokio::test]
async fn own_comments_in_the_conversation_are_edited_and_deleted_on_the_platform_and_no_one_elses() {
    let (fx, hosting) = on_a_pull_request().await;
    let mine = hosting.say_on_the_pull_request("ada", true, "Looks good.");
    let theirs = hosting.say_on_the_pull_request("marco", false, "Thanks!");
    fx.sync(&hosting).await;

    fx.session
        .edit_conversation_comment(&hosting, &mine, "Looks good to me.".into())
        .await
        .unwrap();

    assert_eq!(hosting.conversation(), ["Looks good to me.", "Thanks!"]);
    assert_eq!(conversation(&fx).await, ["Looks good to me.", "Thanks!"]);
    let edited = fx
        .session
        .edit_conversation_comment(&hosting, &theirs, "Mine now.".into())
        .await;
    let removed = fx.session.remove_conversation_comment(&hosting, &theirs).await;
    for refused in [edited, removed] {
        assert!(matches!(refused, Err(SessionError::Invalid(_))), "{refused:?}");
    }

    fx.session.remove_conversation_comment(&hosting, &mine).await.unwrap();

    assert_eq!(hosting.conversation(), ["Thanks!"]);
    assert_eq!(conversation(&fx).await, ["Thanks!"]);
}

#[tokio::test]
async fn a_comment_on_the_whole_pull_request_is_a_thread_that_resolves_where_the_platform_has_it_so() {
    let (fx, hosting) = on_an_azure_devops_pull_request().await;
    let overall = fx.thread(NewThread::Changeset).await;
    fx.session.post_drafts(&hosting).await.unwrap();

    fx.session
        .set_thread_status(overall, ThreadStatus::Resolved)
        .await
        .unwrap();
    fx.session.reply(overall, "Done.".into(), 200).await.unwrap();
    fx.session.post_drafts(&hosting).await.unwrap();
    fx.sync(&hosting).await;

    let id = remote_id(&fx.view(overall).await).unwrap().to_owned();
    assert!(hosting.calls().contains(&format!("resolve {id} true")));
    let thread = fx.view(overall).await;
    assert_eq!(thread.status, ThreadStatus::Resolved);
    assert_eq!(bodies(&thread), ["why?", "Done."]);

    // On GitHub the same comment is a flat one, with no status to set.
    let (fx, _) = on_a_pull_request().await;
    let overall = fx.thread(NewThread::Changeset).await;
    let refused = fx.session.set_thread_status(overall, ThreadStatus::Resolved).await;
    assert!(matches!(refused, Err(SessionError::Invalid(_))), "{refused:?}");
}

#[tokio::test]
async fn own_comments_are_removed_locally_when_drafts_and_on_the_platform_when_posted() {
    let (fx, hosting) = on_a_pull_request().await;
    let mine = hosting.my_thread("a.txt", (4, 4), "pr1", "first");
    hosting.say(&mine, "marco", "an answer");
    let theirs = hosting.thread("a.txt", (21, 21), "pr1", "marco", "not mine");
    fx.sync(&hosting).await;
    let all = fx.all().await;
    let my_thread = all
        .iter()
        .find(|t| remote_id(t) == Some(mine.as_str()))
        .unwrap()
        .clone();
    let their_thread = all
        .iter()
        .find(|t| remote_id(t) == Some(theirs.as_str()))
        .unwrap()
        .clone();

    // A draft goes without the platform hearing of it.
    let draft = fx
        .session
        .reply(my_thread.id, "draft".into(), 9)
        .await
        .unwrap()
        .messages[2]
        .clone();
    fx.session.remove_message(&hosting, draft.id).await.unwrap();
    assert_eq!(bodies(&fx.view(my_thread.id).await), ["first", "an answer"]);
    assert!(hosting.calls().iter().all(|c| !c.starts_with("delete")));

    // What someone else wrote cannot be removed.
    let refused = fx.session.remove_message(&hosting, their_thread.messages[0].id).await;
    assert!(matches!(refused, Err(SessionError::Invalid(_))));

    // The first comment of a posted thread removes the thread, there and here.
    fx.session
        .remove_message(&hosting, my_thread.messages[0].id)
        .await
        .unwrap();
    assert!(hosting.calls().iter().any(|c| c.starts_with("delete")));
    assert_eq!(hosting.thread_ids(), std::slice::from_ref(&theirs));
    assert_eq!(
        fx.all().await.iter().map(|t| remote_id(t)).collect::<Vec<_>>(),
        [Some(theirs.as_str())]
    );
}

#[tokio::test]
async fn a_posted_reply_is_removed_alone_and_deleting_a_thread_the_viewer_started_removes_it_there() {
    let (fx, hosting) = on_a_pull_request().await;
    hosting.my_thread("a.txt", (4, 4), "pr1", "first");
    fx.sync(&hosting).await;
    let thread = fx.all().await[0].id;
    fx.session.reply(thread, "second".into(), 9).await.unwrap();
    fx.session.submit_review(&hosting, "COMMENT", None).await.unwrap();
    let second = fx.all().await[0].messages[1].clone();

    fx.session.remove_message(&hosting, second.id).await.unwrap();
    assert_eq!(bodies(&fx.all().await[0]), ["first"]);

    fx.session.delete_thread(&hosting, thread).await.unwrap();
    assert!(hosting.thread_ids().is_empty() && fx.all().await.is_empty());
}

#[tokio::test]
async fn a_thread_the_viewer_did_not_start_cannot_be_deleted_from_the_platform() {
    let (fx, hosting) = on_a_pull_request().await;
    hosting.thread("a.txt", (4, 4), "pr1", "marco", "his");
    fx.sync(&hosting).await;
    let thread = fx.all().await[0].id;

    let refused = fx.session.delete_thread(&hosting, thread).await;

    assert!(matches!(refused, Err(SessionError::Invalid(_))));
    assert_eq!(fx.all().await.len(), 1);
}

#[tokio::test]
async fn a_comment_edited_before_posting_is_just_changed_and_the_platform_hears_of_it_with_the_review() {
    let (fx, hosting) = on_a_pull_request().await;
    let thread = fx.thread(on("a.txt", 20, 21)).await;
    let draft = fx.view(thread).await.messages[0].clone();

    let view = fx
        .session
        .edit_message(&hosting, draft.id, "better".into(), 7)
        .await
        .unwrap();

    assert_eq!(bodies(&view), ["better"]);
    assert!(hosting.calls().is_empty());
    fx.session.submit_review(&hosting, "COMMENT", None).await.unwrap();
    assert_eq!(hosting.0.lock().unwrap().submissions[0].threads[0].body, "better");
}

#[tokio::test]
async fn comments_on_a_changeset_that_is_no_pull_request_are_never_drafts() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    let id = fx.thread(on("a.txt", 0, 1)).await;

    let view = fx.view(id).await;

    assert_eq!(view.platform, None);
    let error = fx.session.sync_threads(&FakeHosting::default()).await.unwrap_err();
    assert!(matches!(error, SessionError::Invalid(_)));
}

/// Drafts for a platform that takes one comment at a time: a new thread, a reply in marco's
/// thread, and a comment on the whole pull request.
async fn with_three_drafts() -> (Fixture, FakeHosting) {
    let (fx, hosting) = on_an_azure_devops_pull_request().await;
    hosting.thread("a.txt", (4, 4), "pr1", "marco", "Why 3?");
    fx.sync(&hosting).await;
    let on_theirs = fx.all().await[0].id;
    fx.thread(on("a.txt", 20, 21)).await;
    fx.session.reply(on_theirs, "Agreed.".into(), 7).await.unwrap();
    let revision = fx.session.revision().await;
    let create = fx
        .session
        .create_thread(revision, View::Changeset, NewThread::Changeset, "Overall.".into(), 100);
    create.await.unwrap();
    (fx, hosting)
}

#[tokio::test]
async fn on_a_platform_that_takes_one_comment_at_a_time_the_drafts_are_posted_in_turn() {
    let (fx, hosting) = with_three_drafts().await;

    let report = fx.session.post_drafts(&hosting).await.unwrap();

    assert_eq!((report.posted, report.skipped, report.problem), (3, 0, None));
    let posts: Vec<_> = hosting.calls().into_iter().filter(|c| c.starts_with("post")).collect();
    assert_eq!(posts, ["post why?", "post Agreed.", "post Overall."]);
    let all = fx.all().await;
    assert!(all.iter().all(|t| t.messages.iter().all(|m| m.remote.is_some())));
    assert!(
        all.iter().all(|t| remote_id(t).is_some()),
        "the comment on the pull request is a thread"
    );
    let again = fx.sync(&hosting).await;
    assert!(!again.changed, "nothing is added twice: {again:?}");
    assert_eq!(fx.all().await.len(), 3);
}

#[tokio::test]
async fn a_comment_on_the_whole_pull_request_deleted_where_it_is_a_thread_goes_here_too() {
    let (fx, hosting) = with_three_drafts().await;
    fx.session.post_drafts(&hosting).await.unwrap();
    let whole = fx
        .all()
        .await
        .into_iter()
        .find(|t| t.target == CommentTarget::Changeset)
        .expect("the comment on the pull request");
    hosting.delete_thread(remote_id(&whole).unwrap());

    let report = fx.sync(&hosting).await;

    assert!(report.changed);
    let left = fx.all().await;
    assert_eq!(left.len(), 2);
    assert!(left.iter().all(|t| t.target != CommentTarget::Changeset));
}

#[tokio::test]
async fn a_thread_whose_first_comment_was_removed_on_a_platform_that_keeps_it_keeps_the_others() {
    let (fx, _) = on_a_pull_request().await;
    let hosting = FakeHosting::per_comment();
    let theirs = hosting.thread("a.txt", (4, 4), "pr1", "marco", "Why 3?");
    let answer = hosting.say(&theirs, "lena", "Because.");
    let mine = hosting.my_thread("a.txt", (21, 21), "pr1", "Why 20?");
    hosting.say(&mine, "marco", "Because 20.");
    fx.sync(&hosting).await;
    let first_of_theirs = fx.all().await[0].messages[0].remote.clone().unwrap().id;

    // Someone removes the first comment in the browser.
    hosting.delete_comment(&first_of_theirs);
    fx.sync(&hosting).await;
    let on_theirs = fx
        .all()
        .await
        .into_iter()
        .find(|t| remote_id(t) == Some(theirs.as_str()))
        .expect("the thread stays");
    assert_eq!(bodies(&on_theirs), ["Because."]);
    assert_eq!(on_theirs.messages[0].remote.as_ref().unwrap().id, answer);
    assert_eq!((on_theirs.state, on_theirs.location), (Some(EXACT), at("a.txt", 3, 4)));

    // The viewer removes the first comment of their own thread here.
    let my_thread = fx
        .all()
        .await
        .into_iter()
        .find(|t| remote_id(t) == Some(mine.as_str()))
        .unwrap();
    fx.session
        .remove_message(&hosting, my_thread.messages[0].id)
        .await
        .unwrap();
    assert_eq!(bodies(&fx.view(my_thread.id).await), ["Because 20."]);
    assert!(hosting.thread_ids().contains(&mine));
    assert!(!fx.sync(&hosting).await.changed, "here and there agree");
}

#[tokio::test]
async fn posting_stops_at_the_first_draft_the_platform_refuses_and_keeps_the_rest_as_drafts() {
    let (fx, hosting) = with_three_drafts().await;
    hosting.fail("post Agreed.", PlatformError::Forbidden("no access".to_owned()));

    let report = fx.session.post_drafts(&hosting).await.unwrap();

    assert_eq!(report.posted, 1, "the new thread went through");
    assert!(matches!(report.problem, Some(PlatformError::Forbidden(_))));
    assert_eq!(report.not_posted, ["Agreed.", "Overall."]);
    assert!(
        hosting.calls().iter().all(|c| c != "post Overall."),
        "nothing after the refusal was tried"
    );
    assert_eq!(report.failed, 1);
    assert_eq!(
        outgoing(&fx).await,
        [
            ("Agreed.".to_owned(), failed("no access")),
            ("Overall.".to_owned(), Outgoing::Draft)
        ],
        "the refused one failed, the one after it was not tried"
    );

    // Once the platform takes them, the next post sends what is left.
    hosting.recover();
    let report = fx.session.post_drafts(&hosting).await.unwrap();
    assert_eq!((report.posted, report.problem), (2, None));
    assert!(outgoing(&fx).await.is_empty());
}

#[tokio::test]
async fn a_post_that_loses_the_connection_midway_leaves_only_the_one_on_its_way_to_be_looked_for() {
    let (fx, hosting) = with_three_drafts().await;
    hosting.fail("post Agreed.", PlatformError::Unreachable("offline".to_owned()));

    fx.session.post_drafts(&hosting).await.unwrap();

    let left = outgoing(&fx).await;
    assert!(
        matches!(&left[..], [(a, Outgoing::Posting { .. }), (b, Outgoing::Draft)] if a == "Agreed." && b == "Overall."),
        "{left:?}"
    );
    hosting.recover();
    fx.sync(&hosting).await;
    assert_eq!(
        outgoing(&fx).await,
        [
            ("Agreed.".to_owned(), Outgoing::Draft),
            ("Overall.".to_owned(), Outgoing::Draft)
        ],
        "the platform never got it"
    );
}

#[tokio::test]
async fn a_platform_is_posted_to_the_way_it_takes_comments_and_an_empty_post_is_refused() {
    let (fx, github) = on_a_pull_request().await;
    let azure = FakeHosting::per_comment();

    let batched = fx.session.post_drafts(&github).await;
    let single = fx.session.submit_review(&azure, "approved", None).await;
    let empty = fx.session.post_drafts(&azure).await;

    for refused in [batched, single, empty] {
        assert!(matches!(refused, Err(SessionError::Invalid(_))), "{refused:?}");
    }
    assert!(github.calls().is_empty() && azure.calls().is_empty());
}

async fn pending(fx: &Fixture) -> Option<(String, Outgoing)> {
    let vote = fx.session.pending_vote().await.unwrap()?;
    Some((vote.verdict, vote.outgoing))
}

fn draft_vote(verdict: &str) -> Option<(String, Outgoing)> {
    Some((verdict.to_owned(), Outgoing::Draft))
}

#[tokio::test]
async fn a_vote_is_staged_only_where_the_platform_keeps_it_apart_and_knows_the_verdict_and_sends_nothing() {
    let (fx, github) = on_a_pull_request().await;
    let azure = FakeHosting::per_comment();

    fx.session.stage_vote(&azure, "wait-for-author").await.unwrap();
    let unknown = fx.session.stage_vote(&azure, "APPROVE").await;
    let joined = fx.session.stage_vote(&github, "APPROVE").await;

    assert!(matches!(unknown, Err(SessionError::Invalid(_))));
    assert!(matches!(joined, Err(SessionError::Invalid(_))));
    assert_eq!(pending(&fx).await, draft_vote("wait-for-author"));
    assert!(
        azure.calls().is_empty() && github.calls().is_empty(),
        "staging asks no platform"
    );
}

#[tokio::test]
async fn staging_again_replaces_the_vote_and_the_viewers_current_vote_clears_it() {
    let (fx, hosting) = on_an_azure_devops_pull_request().await;
    hosting.0.lock().unwrap().discussion.viewer_verdict = Some("approved".to_owned());
    fx.sync(&hosting).await;

    fx.session.stage_vote(&hosting, "rejected").await.unwrap();
    fx.session.stage_vote(&hosting, "wait-for-author").await.unwrap();
    assert_eq!(pending(&fx).await, draft_vote("wait-for-author"));

    fx.session.stage_vote(&hosting, "approved").await.unwrap();
    assert_eq!(pending(&fx).await, None, "the platform has this vote already");

    fx.session.stage_vote(&hosting, "rejected").await.unwrap();
    fx.session.discard_vote().await.unwrap();
    assert_eq!(pending(&fx).await, None);
}

#[tokio::test]
async fn a_post_sends_the_comments_and_then_the_vote() {
    let (fx, hosting) = with_three_drafts().await;
    fx.session.stage_vote(&hosting, "approved").await.unwrap();

    let report = fx.session.post_drafts(&hosting).await.unwrap();

    assert_eq!((report.posted, report.vote, report.problem), (3, VoteSent::Cast, None));
    let calls = hosting.calls();
    let last_post = calls.iter().rposition(|c| c.starts_with("post")).unwrap();
    assert_eq!(calls.iter().position(|c| c == "vote approved"), Some(last_post + 1));
    assert_eq!(pending(&fx).await, None);
    assert_eq!(hosting.votes(), ["approved"]);
    fx.sync(&hosting).await;
    assert_eq!(pending(&fx).await, None);
}

#[tokio::test]
async fn a_vote_alone_is_posted_without_asking_the_platform_for_comments() {
    let (fx, hosting) = on_an_azure_devops_pull_request().await;
    fx.session.stage_vote(&hosting, "rejected").await.unwrap();

    let report = fx.session.post_drafts(&hosting).await.unwrap();

    assert_eq!((report.posted, report.vote), (0, VoteSent::Cast));
    assert_eq!(hosting.calls(), ["vote rejected"]);
}

#[tokio::test]
async fn a_vote_the_platform_refuses_fails_with_its_reason_and_is_sent_again_with_the_next_post() {
    let (fx, hosting) = on_an_azure_devops_pull_request().await;
    fx.session.stage_vote(&hosting, "rejected").await.unwrap();
    hosting.fail("vote", PlatformError::Forbidden("no access".to_owned()));

    let report = fx.session.post_drafts(&hosting).await.unwrap();

    assert_eq!(report.vote, VoteSent::Failed);
    assert!(matches!(report.problem, Some(PlatformError::Forbidden(_))));
    assert_eq!(pending(&fx).await, Some(("rejected".to_owned(), failed("no access"))));
    assert!(hosting.votes().is_empty());

    hosting.recover();
    let report = fx.session.post_drafts(&hosting).await.unwrap();
    assert_eq!((report.vote, report.problem), (VoteSent::Cast, None));
    assert_eq!(pending(&fx).await, None);
    assert_eq!(hosting.votes(), ["rejected"]);
}

#[tokio::test]
async fn a_vote_waits_while_a_comment_stopped_the_post() {
    let (fx, hosting) = with_three_drafts().await;
    fx.session.stage_vote(&hosting, "approved").await.unwrap();
    hosting.fail("post Agreed.", PlatformError::Forbidden("no access".to_owned()));

    let report = fx.session.post_drafts(&hosting).await.unwrap();

    assert_eq!(report.vote, VoteSent::NotSent);
    assert!(hosting.calls().iter().all(|c| !c.starts_with("vote")));
    assert_eq!(pending(&fx).await, draft_vote("approved"));
}

#[tokio::test]
async fn a_vote_that_lost_the_connection_is_looked_for_at_the_next_sync() {
    let (fx, hosting) = on_an_azure_devops_pull_request().await;
    fx.session.stage_vote(&hosting, "rejected").await.unwrap();
    hosting.fail("vote", PlatformError::Unreachable("offline".to_owned()));

    let report = fx.session.post_drafts(&hosting).await.unwrap();

    assert_eq!(report.vote, VoteSent::Failed);
    assert!(matches!(pending(&fx).await, Some((_, Outgoing::Posting { .. }))));
    assert!(
        matches!(fx.session.discard_vote().await, Err(SessionError::Invalid(_))),
        "it may be on the platform"
    );

    hosting.recover();
    fx.sync(&hosting).await;
    assert_eq!(pending(&fx).await, draft_vote("rejected"), "the platform never got it");

    // Taken, but the answer was lost.
    fx.session.post_drafts(&hosting).await.unwrap();
    fx.session.stage_vote(&hosting, "wait-for-author").await.unwrap();
    hosting.fail("vote", PlatformError::Unreachable("offline".to_owned()));
    fx.session.post_drafts(&hosting).await.unwrap();
    hosting.recover();
    hosting.0.lock().unwrap().discussion.viewer_verdict = Some("wait-for-author".to_owned());
    fx.sync(&hosting).await;
    assert_eq!(pending(&fx).await, None, "the platform has it");
}

#[tokio::test]
async fn a_vote_survives_a_sync_that_fails_and_is_dropped_once_the_platform_has_it() {
    let (fx, hosting) = on_an_azure_devops_pull_request().await;
    fx.session.stage_vote(&hosting, "rejected").await.unwrap();
    hosting.fail("", PlatformError::Unreachable("offline".to_owned()));

    assert!(fx.session.sync_threads(&hosting).await.is_err());
    assert_eq!(pending(&fx).await, draft_vote("rejected"));

    hosting.recover();
    hosting.0.lock().unwrap().discussion.viewer_verdict = Some("rejected".to_owned());
    fx.sync(&hosting).await;
    assert_eq!(pending(&fx).await, None, "voted elsewhere, so there is nothing to send");
}
