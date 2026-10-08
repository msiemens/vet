//! Review threads, the conversation and what is posted, against a local server that answers as
//! GitHub's GraphQL API does.

use vet_app::platform::{
    CommentKind, CommentRef, DiffSide, NewConversationComment, NewReply, NewThread, PullRequestState, ReviewerState,
    Submission,
};

use super::*;

fn comment(id: &str, body: &str, who: &str, mine: bool) -> Value {
    json!({
        "id": id, "body": body, "createdAt": "2024-05-01T12:00:00Z", "lastEditedAt": null,
        "state": "SUBMITTED", "viewerDidAuthor": mine,
        "author": { "login": who, "avatarUrl": format!("https://avatars.example/{who}") },
        "originalCommit": { "oid": "c1c1c1" }
    })
}

fn with(mut object: Value, key: &str, value: Value) -> Value {
    object[key] = value;
    object
}

fn thread(id: &str, resolved: bool, line: Option<u32>, comments: Value, next: Option<&str>) -> Value {
    json!({
        "id": id, "isResolved": resolved, "isOutdated": false, "path": "src/job.rs", "diffSide": "RIGHT",
        "line": line, "startLine": null, "originalLine": line, "originalStartLine": null,
        "subjectType": if line.is_some() { "LINE" } else { "FILE" },
        "resolvedBy": if resolved { json!({ "login": "lena" }) } else { Value::Null },
        "comments": { "nodes": comments, "pageInfo": { "hasNextPage": next.is_some(), "endCursor": next } }
    })
}

/// The pull request as the first page of a discussion query shows it.
fn first_page(threads: Value, next: Option<&str>) -> String {
    json!({ "data": { "repository": { "pullRequest": {
        "title": "Add idempotency keys", "url": "https://github.com/acme/app/pull/482", "isDraft": false,
        "state": "OPEN", "headRefName": "lena/keys", "baseRefName": "main",
        "author": { "login": "lena", "avatarUrl": "https://avatars.example/lena" },
        "labels": { "nodes": [{ "name": "sync" }, { "name": "database" }] },
        "reviewRequests": { "nodes": [
            { "requestedReviewer": { "__typename": "User", "login": "ada" } },
            { "requestedReviewer": { "__typename": "User", "login": "marco" } }
        ] },
        "latestReviews": { "nodes": [
            { "state": "APPROVED", "author": { "login": "marco" } },
            { "state": "COMMENTED", "author": { "login": "jonas" } },
            { "state": "PENDING", "author": { "login": "ada" } }
        ] },
        "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "state": "FAILURE" } } }] },
        "comments": { "nodes": [{
            "id": "IC1", "body": "Looks good to me", "createdAt": "2024-05-01T10:00:00Z", "lastEditedAt": null,
            "viewerDidAuthor": false, "author": { "login": "jonas", "avatarUrl": null }
        }] },
        "reviews": { "nodes": [
            { "id": "R1", "state": "APPROVED", "body": "", "submittedAt": "2024-05-01T11:00:00Z",
              "author": { "login": "marco", "avatarUrl": null } },
            { "id": "R2", "state": "COMMENTED", "body": "", "submittedAt": "2024-05-01T11:30:00Z",
              "author": { "login": "jonas", "avatarUrl": null } },
            { "id": "R3", "state": "COMMENTED", "body": "A few notes.", "submittedAt": "2024-05-01T09:00:00Z",
              "author": { "login": "jonas", "avatarUrl": null } },
            { "id": "R4", "state": "PENDING", "body": "mine, not yet", "submittedAt": null, "author": null }
        ] },
        "reviewThreads": { "nodes": threads, "pageInfo": { "hasNextPage": next.is_some(), "endCursor": next } }
    } } } })
    .to_string()
}

fn threads_page(threads: Value, more: bool) -> String {
    json!({ "data": { "repository": { "pullRequest": {
        "reviewThreads": { "nodes": threads, "pageInfo": { "hasNextPage": more, "endCursor": more.then_some("again") } }
    } } } })
    .to_string()
}

fn query_of(request: &Seen) -> String {
    serde_json::from_str::<Value>(&request.body).unwrap()["query"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// `addPullRequestReview` for a mutation, `pullRequest` for the lookup of the pull request's id.
fn call_of(request: &Seen) -> String {
    let query = query_of(request);
    match query
        .split_once(") { ")
        .map(|(head, call)| (head, call.split('(').next().unwrap()))
    {
        Some((head, call)) if head.starts_with("mutation") => call.to_owned(),
        _ if query.contains("node(id") => "node".to_owned(),
        _ if query.contains("title url") => "discussion".to_owned(),
        _ if query.contains("reviewThreads") => "threads".to_owned(),
        _ => "pullRequest".to_owned(),
    }
}

fn calls(fake: &Fake) -> Vec<String> {
    fake.graphql_requests().iter().map(call_of).collect()
}

fn input_of(request: &Seen) -> Value {
    request.variables()["input"].clone()
}

fn data(value: Value) -> Canned {
    Canned::json(&json!({ "data": value }).to_string())
}

#[tokio::test]
async fn a_discussion_is_read_with_every_page_of_threads_and_every_comment_of_each() {
    set_now(NOW);
    let fake = Fake::start(|request, _| {
        match call_of(request).as_str() {
        "discussion" => Canned::json(&first_page(
            json!([
                thread("T1", false, Some(21), json!([comment("C1", "Why 20?", "marco", false)]), Some("more-1")),
                // Only the comment of a review that is not submitted: not there for anyone else.
                thread(
                    "T2",
                    false,
                    Some(5),
                    json!([with(comment("C9", "draft", "ada", true), "state", json!("PENDING"))]),
                    None
                )
            ]),
            Some("page-2"),
        )),
        "node" => Canned::json(
            &json!({ "data": { "node": { "comments": {
                "nodes": [with(comment("C2", "Because.", "lena", false), "lastEditedAt", json!("2024-05-01T13:00:00Z"))],
                "pageInfo": { "hasNextPage": false, "endCursor": null }
            } } } })
            .to_string(),
        ),
        "threads" => Canned::json(&threads_page(
            json!([thread(
                "T3",
                true,
                None,
                json!([comment("C5", "On the whole file", "lena", false)]),
                None
            )]),
            false,
        )),
        other => panic!("unexpected {other}"),
    }
    });

    let found = client(&fake).discussion(&remote(), 482).await.unwrap();

    assert_eq!(calls(&fake), ["discussion", "node", "threads"]);
    assert_eq!(fake.graphql_requests()[1].variables()["after"], "more-1");
    assert_eq!(fake.graphql_requests()[2].variables()["after"], "page-2");
    assert!(found.complete);
    assert_eq!(
        found.threads.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["T1", "T3"]
    );
    let first = &found.threads[0];
    assert_eq!(
        (
            first.path.as_str(),
            first.side,
            first.lines,
            first.resolved,
            first.outdated
        ),
        ("src/job.rs", DiffSide::Right, Some((21, 21)), false, false)
    );
    assert_eq!(
        first.comments.iter().map(|c| c.body.as_str()).collect::<Vec<_>>(),
        ["Why 20?", "Because."]
    );
    assert_eq!(first.comments[0].commit.as_deref(), Some("c1c1c1"));
    assert_eq!(
        first.comments[0].avatar.as_deref(),
        Some("https://avatars.example/marco")
    );
    assert_eq!(
        (first.comments[0].created_at, first.comments[0].edited_at),
        (1_714_564_800, 1_714_564_800)
    );
    assert_eq!(first.comments[1].edited_at, 1_714_568_400, "edited an hour after");
    let file = &found.threads[1];
    assert_eq!(
        (file.lines, file.resolved, file.resolved_by.as_deref()),
        (None, true, Some("lena"))
    );
}

#[tokio::test]
async fn the_pull_request_block_says_who_reviews_what_blocks_and_what_was_said() {
    set_now(NOW);
    let fake = Fake::start(|_, _| Canned::json(&first_page(json!([]), None)));

    let found = client(&fake).discussion(&remote(), 482).await.unwrap();

    assert_eq!(
        (found.title.as_str(), found.author.as_str(), found.state, found.draft),
        ("Add idempotency keys", "lena", PullRequestState::Open, false)
    );
    assert_eq!(
        (found.source_branch.as_str(), found.target_branch.as_str()),
        ("lena/keys", "main")
    );
    assert_eq!(found.labels, ["sync", "database"]);
    assert_eq!(found.ci, Some(CiState::Failure));
    assert_eq!(
        found
            .reviewers
            .iter()
            .map(|r| (r.name.as_str(), r.state))
            .collect::<Vec<_>>(),
        [
            ("marco", ReviewerState::Waiting),
            ("jonas", ReviewerState::Commented),
            ("ada", ReviewerState::Waiting)
        ],
        "marco approved and was asked again; a review that is not submitted is no verdict"
    );
    assert_eq!(
        found
            .conversation
            .iter()
            .map(|i| (i.id.as_str(), i.verdict))
            .collect::<Vec<_>>(),
        [
            ("R3", Some(ReviewVerdict::Commented)),
            ("IC1", None),
            ("R1", Some(ReviewVerdict::Approved))
        ],
        "oldest first; a review with line comments only says nothing, and one not submitted is not shown"
    );
}

#[tokio::test]
async fn too_many_threads_make_a_partial_listing_that_says_so() {
    set_now(NOW);
    let many: Vec<Value> = (0..50)
        .map(|n| {
            thread(
                &format!("T{n}"),
                false,
                Some(1),
                json!([comment(&format!("C{n}"), "x", "marco", false)]),
                None,
            )
        })
        .collect();
    let fake = Fake::start(move |request, _| match call_of(request).as_str() {
        "discussion" => Canned::json(&first_page(json!(many), Some("next"))),
        _ => Canned::json(&threads_page(json!(many), true)),
    });

    let found = client(&fake).discussion(&remote(), 482).await.unwrap();

    assert!(!found.complete, "{} threads", found.threads.len());
    assert!(found.threads.len() >= 2000);
}

#[tokio::test]
async fn comments_are_changed_and_removed_with_the_calls_for_their_kind_and_threads_are_resolved() {
    set_now(NOW);
    let fake = Fake::start(|request, _| match call_of(request).as_str() {
        "updatePullRequestReviewComment" => data(
            json!({ "updatePullRequestReviewComment": { "pullRequestReviewComment": comment("C1", "new", "ada", true) } }),
        ),
        "updateIssueComment" => {
            data(json!({ "updateIssueComment": { "issueComment": comment("IC1", "overall", "ada", true) } }))
        }
        _ => data(json!({ "x": { "clientMutationId": null } })),
    });
    let client = client(&fake);
    let review = CommentRef {
        id: "C1".to_owned(),
        kind: CommentKind::Review,
    };
    let flat = CommentRef {
        id: "IC1".to_owned(),
        kind: CommentKind::Conversation,
    };

    let edited = client.edit_comment(&remote(), &review, "new").await.unwrap();
    client.edit_comment(&remote(), &flat, "overall").await.unwrap();
    client.delete_comment(&remote(), &review).await.unwrap();
    client.delete_comment(&remote(), &flat).await.unwrap();
    client.set_thread_resolved(&remote(), "T1", true).await.unwrap();
    client.set_thread_resolved(&remote(), "T1", false).await.unwrap();

    assert_eq!((edited.body.as_str(), edited.mine), ("new", true));
    assert_eq!(
        calls(&fake),
        [
            "updatePullRequestReviewComment",
            "updateIssueComment",
            "deletePullRequestReviewComment",
            "deleteIssueComment",
            "resolveReviewThread",
            "unresolveReviewThread"
        ]
    );
    let sent: Vec<Value> = fake.graphql_requests().iter().map(input_of).collect();
    assert_eq!(sent[0], json!({ "pullRequestReviewCommentId": "C1", "body": "new" }));
    assert_eq!(sent[1], json!({ "id": "IC1", "body": "overall" }));
    assert_eq!(sent[2], json!({ "id": "C1" }));
    assert_eq!(sent[4], json!({ "threadId": "T1" }));
    assert_eq!(
        fake.graphql_requests()[0].header("authorization"),
        Some("Bearer gho_secret")
    );
}

#[tokio::test]
async fn a_comment_is_read_as_it_is_now_and_one_that_is_gone_is_not_found() {
    set_now(NOW);
    let fake = Fake::start(|request, _| {
        if request.variables()["id"] == "C1" {
            data(json!({ "node": comment("C1", "now", "ada", true) }))
        } else {
            data(json!({ "node": {} }))
        }
    });
    let client = client(&fake);
    let at = |id: &str| CommentRef {
        id: id.to_owned(),
        kind: CommentKind::Review,
    };

    assert_eq!(client.comment(&remote(), &at("C1")).await.unwrap().body, "now");
    let gone = client.comment(&remote(), &at("C2")).await.unwrap_err();
    assert!(matches!(gone, PlatformError::NotFound(_)), "{gone:?}");
}

fn review() -> Submission {
    Submission {
        verdict: "REQUEST_CHANGES".to_owned(),
        body: Some("Needs a TTL.".to_owned()),
        commit: "c2c2c2".to_owned(),
        threads: vec![
            NewThread {
                key: 1,
                path: "src/job.rs".to_owned(),
                side: DiffSide::Right,
                lines: Some((12, 14)),
                body: "Why this?".to_owned(),
                replies: vec![NewReply {
                    key: 2,
                    thread: String::new(),
                    body: "And this?".to_owned(),
                }],
            },
            NewThread {
                key: 3,
                path: "src/old.rs".to_owned(),
                side: DiffSide::Left,
                lines: None,
                body: "Gone, fine.".to_owned(),
                replies: vec![],
            },
        ],
        replies: vec![NewReply {
            key: 4,
            thread: "T7".to_owned(),
            body: "Agreed.".to_owned(),
        }],
        comments: vec![NewConversationComment {
            key: 5,
            body: "Overall: good.".to_owned(),
        }],
    }
}

/// Answers each call of a review as GitHub does; `fail` is the call that is refused.
fn reviewing(fail: &'static str) -> Fake {
    Fake::start(move |request, _| {
        let call = call_of(request);
        if call == fail {
            return Canned::json(
                r#"{"data":null,"errors":[{"type":"UNPROCESSABLE","message":"Line must be part of the diff"}]}"#,
            );
        }
        let posted = |id: &str, body: &str| comment(id, body, "ada", true);
        match call.as_str() {
            "pullRequest" => data(json!({ "repository": { "pullRequest": { "id": "PR_1" } } })),
            "addPullRequestReview" => {
                data(json!({ "addPullRequestReview": { "pullRequestReview": { "id": "REV_1" } } }))
            }
            "addPullRequestReviewThread" => {
                let body = input_of(request)["body"].as_str().unwrap().to_owned();
                let (thread, id) = if body == "Why this?" {
                    ("T1", "C1")
                } else {
                    ("T3", "C3")
                };
                data(
                    json!({ "addPullRequestReviewThread": { "thread": { "id": thread, "comments": { "nodes": [posted(id, &body)] } } } }),
                )
            }
            "addPullRequestReviewThreadReply" => {
                let body = input_of(request)["body"].as_str().unwrap().to_owned();
                let id = if body == "And this?" { "C2" } else { "C4" };
                data(json!({ "addPullRequestReviewThreadReply": { "comment": posted(id, &body) } }))
            }
            "addComment" => data(json!({ "addComment": { "commentEdge": { "node": {
                "id": "IC5", "body": "Overall: good.", "createdAt": "2024-05-01T12:00:00Z", "lastEditedAt": null,
                "viewerDidAuthor": true, "author": { "login": "ada", "avatarUrl": null }
            } } } })),
            _ => data(json!({ "x": { "clientMutationId": null } })),
        }
    })
}

#[tokio::test]
async fn a_review_is_one_pending_review_filled_and_submitted_with_its_verdict() {
    set_now(NOW);
    let fake = reviewing("");

    let posted = client(&fake).submit_review(&remote(), 482, &review()).await.unwrap();

    assert_eq!(
        calls(&fake),
        [
            "pullRequest",
            "addPullRequestReview",
            "addPullRequestReviewThread",
            "addPullRequestReviewThreadReply",
            "addPullRequestReviewThread",
            "addPullRequestReviewThreadReply",
            "submitPullRequestReview",
            "addComment"
        ]
    );
    let requests = fake.graphql_requests();
    assert_eq!(
        input_of(&requests[1]),
        json!({ "pullRequestId": "PR_1", "commitOID": "c2c2c2" })
    );
    assert_eq!(
        input_of(&requests[2]),
        json!({
            "pullRequestReviewId": "REV_1", "path": "src/job.rs", "body": "Why this?", "subjectType": "LINE",
            "line": 14, "side": "RIGHT", "startLine": 12, "startSide": "RIGHT"
        })
    );
    assert_eq!(
        input_of(&requests[3]),
        json!({ "pullRequestReviewId": "REV_1", "pullRequestReviewThreadId": "T1", "body": "And this?" })
    );
    assert_eq!(
        input_of(&requests[4]),
        json!({ "pullRequestReviewId": "REV_1", "path": "src/old.rs", "body": "Gone, fine.", "subjectType": "FILE" })
    );
    assert_eq!(
        input_of(&requests[5]),
        json!({ "pullRequestReviewId": "REV_1", "pullRequestReviewThreadId": "T7", "body": "Agreed." })
    );
    assert_eq!(
        input_of(&requests[6]),
        json!({ "pullRequestReviewId": "REV_1", "event": "REQUEST_CHANGES", "body": "Needs a TTL." })
    );
    assert_eq!(
        input_of(&requests[7]),
        json!({ "subjectId": "PR_1", "body": "Overall: good." })
    );

    assert_eq!(
        posted
            .threads
            .iter()
            .map(|t| (t.key, t.thread.as_str(), t.comment.id.as_str()))
            .collect::<Vec<_>>(),
        [(1, "T1", "C1"), (3, "T3", "C3")]
    );
    assert_eq!(
        posted.threads[0]
            .replies
            .iter()
            .map(|r| (r.key, r.comment.id.as_str()))
            .collect::<Vec<_>>(),
        [(2, "C2")]
    );
    assert_eq!(
        posted
            .replies
            .iter()
            .map(|r| (r.key, r.comment.id.as_str()))
            .collect::<Vec<_>>(),
        [(4, "C4")]
    );
    assert_eq!(
        posted
            .comments
            .iter()
            .map(|r| (r.key, r.comment.id.as_str()))
            .collect::<Vec<_>>(),
        [(5, "IC5")]
    );
    assert_eq!(posted.failed, None);
}

#[tokio::test]
async fn a_review_that_cannot_be_filled_is_deleted_and_posts_nothing() {
    set_now(NOW);
    for refused in [
        "addPullRequestReviewThread",
        "addPullRequestReviewThreadReply",
        "submitPullRequestReview",
    ] {
        let fake = reviewing(refused);

        let error = client(&fake)
            .submit_review(&remote(), 482, &review())
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains("Line must be part of the diff"),
            "{refused}: {error}"
        );
        let made = calls(&fake);
        assert_eq!(
            made.last().map(String::as_str),
            Some("deletePullRequestReview"),
            "{refused}: {made:?}"
        );
        assert!(!made.contains(&"addComment".to_owned()), "{refused}: {made:?}");
        assert_eq!(
            input_of(fake.graphql_requests().last().unwrap()),
            json!({ "pullRequestReviewId": "REV_1" })
        );
    }
}

#[tokio::test]
async fn a_comment_on_the_pull_request_that_is_refused_is_reported_after_the_review_was_posted() {
    set_now(NOW);
    let fake = reviewing("addComment");

    let posted = client(&fake).submit_review(&remote(), 482, &review()).await.unwrap();

    assert_eq!(posted.threads.len(), 2, "the review is there");
    assert!(posted.comments.is_empty());
    assert!(
        matches!(posted.failed, Some(PlatformError::Unexpected(_))),
        "{:?}",
        posted.failed
    );
}
