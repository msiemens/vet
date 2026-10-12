//! Threads, comments, votes and posting, against a local server that keeps a pull request's
//! threads as Azure DevOps does.

use serde_json::{Value, json};
use vet_app::platform::{
    CommentKind, CommentRef, DiffSide, NewConversationComment, NewReply, NewThread, PullRequestState, ReviewerState,
    Submission,
};

use super::*;

const PR: &str = "/contoso/shop/_apis/git/repositories/api/pullrequests/7";

/// Threads the server holds, and what it was asked.
#[derive(Default)]
struct Hosted {
    threads: Vec<Value>,
    next_thread: i32,
}

fn author(unique_name: &str, id: &str) -> Value {
    json!({ "id": id, "displayName": unique_name, "uniqueName": unique_name })
}

fn comment(id: i64, who: Value, content: &str) -> Value {
    json!({
        "id": id, "author": who, "content": content, "commentType": "text",
        "publishedDate": "2024-05-01T12:00:00Z", "lastUpdatedDate": "2024-05-01T12:00:00Z",
        "lastContentUpdatedDate": "2024-05-01T12:00:00Z"
    })
}

fn thread(id: i32, status: &str, context: Value, comments: Vec<Value>) -> Value {
    let mut thread = json!({ "id": id, "status": status, "comments": comments });
    if !context.is_null() {
        thread["threadContext"] = context;
    }
    thread
}

fn file_context(path: &str, side: &str, lines: (i32, i32)) -> Value {
    json!({
        "filePath": path,
        format!("{side}FileStart"): { "line": lines.0, "offset": 1 },
        format!("{side}FileEnd"): { "line": lines.1, "offset": 1 },
    })
}

fn iterations() -> Value {
    let iteration = |id: i32, source: &str| {
        json!({ "id": id, "reason": "push", "createdDate": "2024-04-01T10:00:00Z",
            "sourceRefCommit": { "commitId": source }, "commonRefCommit": { "commitId": "base" } })
    };
    json!({ "count": 2, "value": [iteration(1, "a1"), iteration(2, "a2")] })
}

fn pull_request() -> Value {
    let mut doc = Doc::new(7).json();
    doc["isDraft"] = json!(false);
    doc["lastMergeSourceCommit"] = json!({ "commitId": "a2" });
    doc["repository"]["project"]["id"] = json!("proj-1");
    let mut declined = reviewer(identity("d", "dennis@contoso.com", false), 0);
    declined["hasDeclined"] = json!(true);
    doc["reviewers"] = json!([
        reviewer(ada(), 5),
        reviewer(identity("l", "linus@contoso.com", false), -10),
        reviewer(team(), 0),
        declined,
    ]);
    doc
}

fn is(request: &Seen, method: &str, suffix: &str) -> bool {
    let path = request.path.split('?').next().unwrap().to_ascii_lowercase();
    request.method == method && path.ends_with(suffix)
}

fn body_of(request: &Seen) -> Value {
    serde_json::from_str(&request.body).unwrap()
}

/// A server for pull request 7 that holds `threads` and answers like Azure DevOps to what is
/// posted to it.
fn hosting(threads: Vec<Value>) -> (Fake, Arc<Mutex<Hosted>>) {
    let hosted = Arc::new(Mutex::new(Hosted {
        next_thread: 100,
        threads,
    }));
    let state = hosted.clone();
    let fake = Fake::start(move |request, _| {
        let mut hosted = state.lock().unwrap();
        let path = request.path.split('?').next().unwrap().to_ascii_lowercase();
        let find = |hosted: &mut Hosted, id: i64| {
            hosted
                .threads
                .iter()
                .position(|t| t["id"].as_i64() == Some(id))
                .expect("a thread")
        };
        let ids = |after: &str| -> Vec<i64> {
            path.split_once(after)
                .map(|(_, rest)| rest.split('/').filter_map(|p| p.parse().ok()).collect())
                .unwrap_or_default()
        };
        if path.ends_with("/_apis/connectiondata") {
            Canned::json(&profile())
        } else if path == "/contoso/_apis/identities" {
            Canned::json(&no_value())
        } else if path == PR {
            Canned::json(&pull_request().to_string())
        } else if path == format!("{PR}/iterations") {
            Canned::json(&iterations().to_string())
        } else if path == format!("{PR}/iterations/2/changes") {
            Canned::json(
                &json!({ "changeEntries": [
                    { "changeTrackingId": 41, "changeId": 1, "changeType": "edit", "item": { "path": "/src/other.rs" } },
                    { "changeTrackingId": 42, "changeId": 2, "changeType": "edit", "item": { "path": "/src/job.rs" } },
                ]})
                .to_string(),
            )
        } else if path.ends_with("/statuses") || path.starts_with("/contoso/shop/_apis/policy/evaluations") {
            Canned::json(&no_value())
        } else if is(request, "GET", "/threads") {
            Canned::json(&json!({ "count": hosted.threads.len(), "value": hosted.threads }).to_string())
        } else if is(request, "POST", "/threads") {
            if request.body.contains("refuse me") {
                return Canned::status(403, r#"{"message":"TF401027: You need the permission."}"#);
            }
            hosted.next_thread += 1;
            let mut created = body_of(request);
            created["id"] = json!(hosted.next_thread);
            let comments = created["comments"].as_array_mut().unwrap();
            for (n, c) in comments.iter_mut().enumerate() {
                *c = comment(i64::try_from(n).unwrap() + 1, ada(), c["content"].as_str().unwrap());
            }
            hosted.threads.push(created.clone());
            Canned::json(&created.to_string())
        } else if is(request, "POST", "/comments") {
            if request.body.contains("refuse me") {
                return Canned::status(403, r#"{"message":"TF401027: You need the permission."}"#);
            }
            let thread = ids("/threads/")[0];
            let at = find(&mut hosted, thread);
            let comments = hosted.threads[at]["comments"].as_array_mut().unwrap();
            let added = comment(
                i64::try_from(comments.len()).unwrap() + 1,
                ada(),
                body_of(request)["content"].as_str().unwrap(),
            );
            comments.push(added.clone());
            Canned::json(&added.to_string())
        } else if request.method == "PATCH" && path.contains("/comments/") {
            let ids = ids("/threads/");
            let at = find(&mut hosted, ids[0]);
            let comments = hosted.threads[at]["comments"].as_array_mut().unwrap();
            let found = comments.iter_mut().find(|c| c["id"].as_i64() == Some(ids[1])).unwrap();
            found["content"] = body_of(request)["content"].clone();
            found["lastContentUpdatedDate"] = json!("2024-05-02T08:00:00Z");
            Canned::json(&found.to_string())
        } else if request.method == "DELETE" && path.contains("/comments/") {
            let ids = ids("/threads/");
            let at = find(&mut hosted, ids[0]);
            // A comment is marked deleted, and the thread with its last one.
            let thread = &mut hosted.threads[at];
            let comments = thread["comments"].as_array_mut().unwrap();
            let found = comments.iter_mut().find(|c| c["id"].as_i64() == Some(ids[1])).unwrap();
            found["isDeleted"] = json!(true);
            found.as_object_mut().unwrap().remove("content");
            if comments.iter().all(|c| c["isDeleted"] == json!(true)) {
                thread["isDeleted"] = json!(true);
            }
            Canned::status(204, "")
        } else if request.method == "GET" && path.contains("/comments/") {
            let ids = ids("/threads/");
            let at = find(&mut hosted, ids[0]);
            let comments = hosted.threads[at]["comments"].as_array().unwrap();
            let found = comments.iter().find(|c| c["id"].as_i64() == Some(ids[1])).unwrap();
            Canned::json(&found.to_string())
        } else if request.method == "PATCH" && path.contains("/threads/") {
            let at = find(&mut hosted, ids("/threads/")[0]);
            hosted.threads[at]["status"] = body_of(request)["status"].clone();
            Canned::json(&hosted.threads[at].to_string())
        } else if request.method == "PUT" && path.contains("/reviewers/") {
            Canned::json(&reviewer(ada(), body_of(request)["vote"].as_i64().unwrap()).to_string())
        } else {
            panic!("unexpected request {} {}", request.method, request.path)
        }
    });
    (fake, hosted)
}

fn lena() -> Value {
    author("lena@contoso.com", "lena-id")
}

fn marco() -> Value {
    author("marco@contoso.com", "marco-id")
}

#[tokio::test]
async fn a_discussion_has_the_threads_of_people_anchored_to_the_iteration_they_were_made_on() {
    set_now(NOW);
    let mut tracked = thread(
        4,
        "closed",
        file_context("/src/other.rs", "right", (3, 3)),
        vec![comment(1, marco(), "tracked")],
    );
    tracked["pullRequestThreadContext"] = json!({
        "iterationContext": { "firstComparingIteration": 1, "secondComparingIteration": 1 },
        "trackingCriteria": { "firstComparingIteration": 2, "secondComparingIteration": 2 },
    });
    let mut made = thread(
        3,
        "active",
        file_context("/src/job.rs", "right", (14, 12)),
        vec![
            comment(1, marco(), "Why 12?"),
            comment(2, lena(), "Because."),
            json!({ "id": 3, "content": "gone", "commentType": "text", "isDeleted": true, "author": marco() }),
            json!({ "id": 4, "content": "Marco updated the vote", "commentType": "system", "author": marco() }),
            comment(5, ada(), "My answer"),
        ],
    );
    made["pullRequestThreadContext"] = json!({
        "iterationContext": { "firstComparingIteration": 1, "secondComparingIteration": 1 },
    });
    let on_the_left = thread(
        5,
        "wontFix",
        file_context("/src/job.rs", "left", (2, 2)),
        vec![comment(1, marco(), "old code")],
    );
    let on_the_file = thread(
        6,
        "pending",
        json!({ "filePath": "/src/job.rs" }),
        vec![comment(1, lena(), "about the file")],
    );
    let on_the_pull_request = thread(7, "fixed", Value::Null, vec![comment(1, ada(), "Overall.")]);
    let only_system = thread(
        8,
        "active",
        Value::Null,
        vec![json!({ "id": 1, "content": "Policy status", "commentType": "system", "author": marco() })],
    );
    let mut deleted = thread(9, "active", Value::Null, vec![comment(1, marco(), "x")]);
    deleted["isDeleted"] = json!(true);
    let (fake, _) = hosting(vec![
        made,
        tracked,
        on_the_left,
        on_the_file,
        on_the_pull_request,
        only_system,
        deleted,
    ]);

    let discussion = client(&fake).discussion(&remote(), 7).await.unwrap();

    let ids: Vec<&str> = discussion.threads.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        ["7/3", "7/4", "7/5", "7/6", "7/7"],
        "no thread of system comments or one that was deleted"
    );
    let [made, tracked, left, file, whole]: [_; 5] = discussion.threads.clone().try_into().unwrap();
    assert_eq!(
        (made.path.as_str(), made.side, made.lines, made.resolved),
        ("src/job.rs", DiffSide::Right, Some((12, 14)), false)
    );
    assert_eq!(
        made.comments
            .iter()
            .map(|c| (
                c.id.as_str(),
                c.author.as_str(),
                c.body.as_str(),
                c.mine,
                c.commit.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            ("7/3/1", "marco@contoso.com", "Why 12?", false, Some("a1")),
            ("7/3/2", "lena@contoso.com", "Because.", false, Some("a1")),
            ("7/3/5", "ada@contoso.com", "My answer", true, Some("a1")),
        ],
        "a comment deleted or by the system is left out, and the iteration of the thread is the commit"
    );
    assert_eq!(
        tracked.comments[0].commit.as_deref(),
        Some("a2"),
        "a thread tracked to a later iteration has its lines there"
    );
    assert!(tracked.resolved, "closed is resolved");
    assert_eq!(
        (left.side, left.lines, left.resolved),
        (DiffSide::Left, Some((2, 2)), true)
    );
    assert_eq!((file.lines, file.resolved), (None, false), "pending is open");
    assert_eq!((whole.path.as_str(), whole.lines, whole.resolved), ("", None, true));
    assert!(discussion.threads.iter().all(|t| t.resolved_by.is_none()));
    assert!(discussion.complete && discussion.conversation.is_empty());
}

#[tokio::test]
async fn comment_authors_are_shown_by_display_name_and_by_address_only_without_one() {
    set_now(NOW);
    // The shape of a real IdentityRef: the display name beside the address, and a picture link.
    let person = |id: &str, display: &str, address: &str| {
        json!({
            "id": id, "displayName": display, "uniqueName": address,
            "url": format!("https://dev.azure.com/contoso/_apis/Identities/{id}"),
            "imageUrl": format!("https://dev.azure.com/contoso/_api/_common/identityImage?id={id}"),
        })
    };
    let me = person("me-id", "Ada Lovelace", "ada@contoso.com");
    let other = person("marco-id", "Marco Polo", "marco@contoso.com");
    let nameless = json!({ "id": "x-id", "uniqueName": "x@contoso.com" });
    let (fake, _) = hosting(vec![thread(
        3,
        "fixed",
        Value::Null,
        vec![comment(1, other, "a"), comment(2, me, "b"), comment(3, nameless, "c")],
    )]);

    let discussion = client(&fake).discussion(&remote(), 7).await.unwrap();

    let authors: Vec<(&str, bool)> = discussion.threads[0]
        .comments
        .iter()
        .map(|c| (c.author.as_str(), c.mine))
        .collect();
    assert_eq!(
        authors,
        [("Marco Polo", false), ("Ada Lovelace", true), ("x@contoso.com", false)]
    );
}

#[tokio::test]
async fn a_discussion_has_the_pull_request_its_reviewers_the_viewers_vote_and_the_labels() {
    set_now(NOW);
    let (fake, _) = hosting(vec![]);

    let discussion = client(&fake).discussion(&remote(), 7).await.unwrap();

    assert_eq!(
        (
            discussion.title.as_str(),
            discussion.url.as_str(),
            discussion.draft,
            discussion.state,
            discussion.source_branch.as_str(),
            discussion.target_branch.as_str(),
            discussion.author.as_str(),
        ),
        (
            "PR 7",
            "https://dev.azure.com/contoso/Shop/_git/api/pullrequest/7",
            false,
            PullRequestState::Open,
            "feature",
            "main",
            "grace@contoso.com",
        )
    );
    assert_eq!(
        discussion
            .reviewers
            .iter()
            .map(|r| (r.name.as_str(), r.state))
            .collect::<Vec<_>>(),
        [
            ("ada@contoso.com", ReviewerState::Approved),
            ("linus@contoso.com", ReviewerState::ChangesRequested),
            ("[Shop]\\Reviewers", ReviewerState::Waiting),
        ],
        "who declined is not listed"
    );
    assert_eq!(discussion.viewer_verdict.as_deref(), Some("approved-with-suggestions"));
    assert_eq!(discussion.labels, ["performance"]);
    assert_eq!(discussion.ci, None);
}

#[tokio::test]
async fn editing_removing_resolving_and_reading_a_comment_go_by_its_ids() {
    set_now(NOW);
    let (fake, hosted) = hosting(vec![thread(
        3,
        "active",
        file_context("/src/job.rs", "right", (5, 5)),
        vec![comment(1, ada(), "mine"), comment(2, marco(), "his")],
    )]);
    let client = client(&fake);
    let mine = CommentRef {
        id: "7/3/1".to_owned(),
        kind: CommentKind::Review,
    };

    let read = client.comment(&remote(), &mine).await.unwrap();
    let edited = client.edit_comment(&remote(), &mine, "better").await.unwrap();
    client.set_thread_resolved(&remote(), "7/3", true).await.unwrap();
    let status_after_resolve = hosted.lock().unwrap().threads[0]["status"].clone();
    client.set_thread_resolved(&remote(), "7/3", false).await.unwrap();
    let status_after_reopen = hosted.lock().unwrap().threads[0]["status"].clone();
    client.delete_comment(&remote(), &mine).await.unwrap();

    assert_eq!((read.body.as_str(), read.mine), ("mine", true));
    assert_eq!((edited.body.as_str(), edited.mine), ("better", true));
    assert!(edited.edited_at > edited.created_at, "the edit time is the content's");
    assert_eq!(status_after_resolve, json!("fixed"));
    assert_eq!(status_after_reopen, json!("active"));
    let sent = fake.requests();
    assert!(
        sent.iter()
            .any(|r| r.method == "PATCH" && r.path.to_ascii_lowercase().contains("/threads/3/comments/1")),
        "{sent:?}"
    );

    // The thread stays with the comment that is left, where it was.
    let discussion = client.discussion(&remote(), 7).await.unwrap();
    let [thread] = <[_; 1]>::try_from(discussion.threads).expect("the thread stays");
    assert_eq!(
        (thread.id.as_str(), thread.path.as_str(), thread.lines),
        ("7/3", "src/job.rs", Some((5, 5)))
    );
    assert_eq!(
        thread.comments.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ["7/3/2"]
    );
}

#[tokio::test]
async fn a_thread_whose_comments_are_all_removed_is_gone() {
    set_now(NOW);
    let (fake, _) = hosting(vec![thread(
        7,
        "active",
        Value::Null,
        vec![comment(1, ada(), "Overall.")],
    )]);
    let client = client(&fake);
    let only = CommentRef {
        id: "7/7/1".to_owned(),
        kind: CommentKind::Review,
    };

    client.delete_comment(&remote(), &only).await.unwrap();

    let discussion = client.discussion(&remote(), 7).await.unwrap();
    assert!(discussion.threads.is_empty() && discussion.complete);
}

#[tokio::test]
async fn an_id_that_names_no_thread_or_comment_is_not_found_without_asking() {
    set_now(NOW);
    let (fake, _) = hosting(vec![]);
    let client = client(&fake);

    let comment = client
        .comment(
            &remote(),
            &CommentRef {
                id: "IC_kwDO".to_owned(),
                kind: CommentKind::Review,
            },
        )
        .await;
    let thread = client.set_thread_resolved(&remote(), "PRRT_x", true).await;

    assert!(matches!(comment, Err(PlatformError::NotFound(_))), "{comment:?}");
    assert!(matches!(thread, Err(PlatformError::NotFound(_))), "{thread:?}");
    assert!(fake.requests().is_empty());
}

#[tokio::test]
async fn the_vote_is_set_for_the_viewer_alone_and_a_verdict_azure_devops_lacks_is_refused() {
    set_now(NOW);
    let (fake, _) = hosting(vec![]);
    let client = client(&fake);

    client.vote(&remote(), 7, "wait-for-author").await.unwrap();
    let unknown = client.vote(&remote(), 7, "APPROVE").await;

    let sent = fake.requests();
    let put = sent.iter().find(|r| r.method == "PUT").expect("a vote");
    assert!(
        put.path
            .to_ascii_lowercase()
            .contains("/pullrequests/7/reviewers/me-id"),
        "{}",
        put.path
    );
    assert_eq!(body_of(put)["vote"], json!(-5));
    assert_eq!(sent.iter().filter(|r| r.method == "PUT").count(), 1);
    assert!(matches!(unknown, Err(PlatformError::Unexpected(_))));
}

fn review() -> Submission {
    Submission {
        verdict: String::new(),
        body: None,
        commit: "a2".to_owned(),
        threads: vec![NewThread {
            key: 1,
            path: "src/job.rs".to_owned(),
            side: DiffSide::Right,
            lines: Some((5, 6)),
            body: "Why 5?".to_owned(),
            replies: vec![NewReply {
                key: 2,
                thread: String::new(),
                body: "And 6?".to_owned(),
            }],
        }],
        replies: vec![NewReply {
            key: 3,
            thread: "7/3".to_owned(),
            body: "Agreed.".to_owned(),
        }],
        comments: vec![NewConversationComment {
            key: 4,
            body: "Overall.".to_owned(),
        }],
    }
}

fn existing() -> Value {
    thread(
        3,
        "active",
        file_context("/src/job.rs", "right", (1, 1)),
        vec![comment(1, marco(), "Hm")],
    )
}

fn posts(fake: &Fake) -> Vec<(String, String)> {
    fake.requests()
        .into_iter()
        .filter(|r| r.method == "POST")
        .map(|r| (r.path.split('?').next().unwrap().to_ascii_lowercase(), r.body))
        .collect()
}

#[tokio::test]
async fn drafts_are_posted_one_comment_after_the_other_on_the_iteration_of_the_head() {
    set_now(NOW);
    let (fake, hosted) = hosting(vec![existing()]);

    let posted = client(&fake).submit_review(&remote(), 7, &review()).await.unwrap();

    assert_eq!(posted.failed, None);
    let sent = posts(&fake);
    let shape: Vec<&str> = sent.iter().map(|(path, _)| path.rsplit('/').next().unwrap()).collect();
    assert_eq!(
        shape,
        ["threads", "comments", "comments", "threads"],
        "the new thread, its reply, the reply in the old thread, the thread on the pull request"
    );
    let first: Value = serde_json::from_str(&sent[0].1).unwrap();
    assert_eq!(
        first["threadContext"],
        json!({
            "filePath": "/src/job.rs",
            "rightFileStart": { "line": 5, "offset": 1 },
            "rightFileEnd": { "line": 6, "offset": 1 },
        })
    );
    assert_eq!(
        first["pullRequestThreadContext"],
        json!({
            "changeTrackingId": 42,
            "iterationContext": { "firstComparingIteration": 2, "secondComparingIteration": 2 },
        }),
        "the iteration is the head's, and the file is followed across pushes"
    );
    assert_eq!(first["comments"][0]["content"], json!("Why 5?"));
    assert_eq!(first["comments"][0]["parentCommentId"], json!(0));
    assert_eq!(first["status"], json!("active"));
    let reply: Value = serde_json::from_str(&sent[1].1).unwrap();
    assert_eq!(
        (reply["content"].clone(), reply["parentCommentId"].clone()),
        (json!("And 6?"), json!(1))
    );
    let last: Value = serde_json::from_str(&sent[3].1).unwrap();
    assert!(last.get("threadContext").is_none() && last.get("pullRequestThreadContext").is_none());

    assert_eq!(posted.threads.len(), 2);
    let opened = &posted.threads[0];
    assert_eq!(
        (opened.key, opened.thread.as_str(), opened.comment.id.as_str()),
        (1, "7/101", "7/101/1")
    );
    assert_eq!(opened.comment.commit.as_deref(), Some("a2"));
    assert!(opened.comment.mine);
    assert_eq!(
        opened
            .replies
            .iter()
            .map(|r| (r.key, r.comment.id.as_str()))
            .collect::<Vec<_>>(),
        [(2, "7/101/2")]
    );
    assert_eq!(
        posted
            .replies
            .iter()
            .map(|r| (r.key, r.comment.id.as_str()))
            .collect::<Vec<_>>(),
        [(3, "7/3/2")]
    );
    let whole = &posted.threads[1];
    assert_eq!((whole.key, whole.thread.as_str()), (4, "7/102"));
    assert_eq!(hosted.lock().unwrap().threads.len(), 3);
}

#[tokio::test]
async fn posting_stops_at_the_first_comment_the_platform_refuses() {
    set_now(NOW);
    let (fake, hosted) = hosting(vec![existing()]);
    let mut refused = review();
    refused.replies[0].body = "refuse me".to_owned();

    let posted = client(&fake).submit_review(&remote(), 7, &refused).await.unwrap();

    assert!(
        matches!(posted.failed, Some(PlatformError::Forbidden(_))),
        "{:?}",
        posted.failed
    );
    assert_eq!(posted.threads.len(), 1, "what went through before stays");
    assert_eq!(posted.threads[0].replies.len(), 1);
    assert!(posted.replies.is_empty() && posted.comments.is_empty());
    assert_eq!(posts(&fake).len(), 3, "nothing after the refusal was tried");
    assert_eq!(hosted.lock().unwrap().threads.len(), 2);
}

#[tokio::test]
async fn a_thread_is_still_posted_when_the_changes_of_the_iteration_cannot_be_read() {
    set_now(NOW);
    let fake = Fake::start(|request, _| {
        let path = request.path.split('?').next().unwrap().to_ascii_lowercase();
        if path.ends_with("/_apis/connectiondata") {
            Canned::json(&profile())
        } else if path == "/contoso/_apis/identities" {
            Canned::json(&no_value())
        } else if path.ends_with("/iterations") {
            Canned::json(&iterations().to_string())
        } else if path.ends_with("/changes") {
            Canned::status(500, "")
        } else if request.method == "POST" {
            let mut created: Value = serde_json::from_str(&request.body).unwrap();
            created["id"] = json!(5);
            created["comments"][0]["id"] = json!(1);
            Canned::json(&created.to_string())
        } else {
            panic!("unexpected request {} {}", request.method, request.path)
        }
    });
    let mut one = review();
    one.replies.clear();
    one.comments.clear();
    one.threads[0].replies.clear();

    let posted = client(&fake).submit_review(&remote(), 7, &one).await.unwrap();

    assert_eq!(posted.failed, None);
    let sent = posts(&fake);
    let thread: Value = serde_json::from_str(&sent[0].1).unwrap();
    assert!(thread["pullRequestThreadContext"].get("changeTrackingId").is_none());
}
