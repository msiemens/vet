//! The checks of commits are those of a pull request on GitHub, and of no other changeset.

use crate::common::{Client, handler, repo_with};
use vet_protocol::{PipelineAction, Request, Response};

#[tokio::test]
async fn a_changeset_that_is_not_a_pull_request_has_no_checks_and_no_logs() {
    let repo = repo_with(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    let (handler, _data) = handler().await;
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;

    let answer = client
        .call(Request::Checks {
            project,
            commits: vec!["abc123".to_owned()],
        })
        .await
        .unwrap();
    assert_eq!(
        answer,
        Response::Checks {
            checks: Vec::new(),
            error: None,
            limited_until: None,
        }
    );

    let refused = client
        .call(Request::CheckLog {
            project,
            run: "11".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(refused.contains("not a pull request"), "{refused}");

    let steps = client
        .call(Request::PipelineSteps {
            project,
            check: "11".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(steps.contains("not a pull request"), "{steps}");
    let changed = client
        .call(Request::ChangePipeline {
            project,
            action: PipelineAction::Cancel { run: "5".to_owned() },
        })
        .await
        .unwrap_err();
    assert!(changed.contains("not a pull request"), "{changed}");
}
