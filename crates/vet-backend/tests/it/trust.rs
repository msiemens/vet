//! Trust (§6.5) through the protocol: a project is trusted until it is marked untrusted again,
//! which the manifest says and every change announces.

use crate::common::{Client, handler, repo_with};
use vet_protocol::{AnalysisTrust, Event, Manifest, ProjectId, Request, Response, ServerMessage, View};

async fn manifest(client: &Client, project: ProjectId) -> Manifest {
    match client
        .call(Request::Manifest {
            project,
            view: View::Changeset,
        })
        .await
        .unwrap()
    {
        Response::Manifest(m) => *m,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn marking_a_project_trusted_is_kept_and_announced_and_marking_it_untrusted_takes_it_back() {
    let repo = repo_with(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    let (handler, _data) = handler().await;
    let mut client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    assert_eq!(
        manifest(&client, project).await.trust,
        AnalysisTrust {
            project: false,
            pull_request: None
        }
    );

    let set = |trusted| client.call(Request::SetProjectTrusted { project, trusted });
    set(true).await.unwrap();
    assert!(manifest(&client, project).await.trust.project);
    let heard: Vec<_> = std::iter::from_fn(|| client.inbox.try_recv().ok())
        .filter_map(|m| match m {
            ServerMessage::Event { event } => Some(event),
            _ => None,
        })
        .collect();
    assert!(heard.contains(&Event::TargetChanged { project }), "{heard:?}");
    assert!(heard.contains(&Event::ServersChanged { project }), "{heard:?}");

    // Another connection finds it trusted too.
    drop(client);
    let again = Client::new(&handler);
    let project = again.open(repo.path()).await;
    assert!(manifest(&again, project).await.trust.project);
    again
        .call(Request::SetProjectTrusted {
            project,
            trusted: false,
        })
        .await
        .unwrap();
    assert!(!manifest(&again, project).await.trust.project);
}

#[tokio::test]
async fn a_changeset_that_is_no_pull_request_cannot_be_trusted_as_one() {
    let repo = repo_with(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    let (handler, _data) = handler().await;
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    let refused = client.call(Request::TrustPullRequest { project }).await;
    assert!(refused.is_err(), "{refused:?}");
}
