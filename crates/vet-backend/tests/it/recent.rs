//! The recent-projects list over the protocol: it belongs to the backend, not to a project.

use crate::common::{Client, handler};
use vet_protocol::{RecentProject, Request, Response};

async fn projects(client: &Client, request: Request) -> Vec<RecentProject> {
    match client.call(request).await.unwrap() {
        Response::RecentProjects { projects } => projects,
        other => panic!("{other:?}"),
    }
}

async fn paths(client: &Client, request: Request) -> Vec<String> {
    projects(client, request).await.into_iter().map(|p| p.path).collect()
}

#[tokio::test]
async fn recorded_projects_are_listed_newest_first_until_forgotten() {
    let (handler, _data) = handler().await;
    let client = Client::new(&handler);
    let record = |path: &str| Request::RecordRecentProject { path: path.into() };

    assert!(paths(&client, Request::RecentProjects).await.is_empty());
    paths(&client, record("/a")).await;
    paths(&client, record("/b")).await;
    assert_eq!(paths(&client, record("/a")).await, ["/a", "/b"]);
    assert_eq!(paths(&client, Request::RecentProjects).await, ["/a", "/b"]);

    let forget = Request::ForgetRecentProject { path: "/a".into() };
    assert_eq!(paths(&client, forget).await, ["/b"]);
}

#[tokio::test]
async fn a_recorded_project_carries_when_it_was_opened() {
    let (handler, _data) = handler().await;
    let client = Client::new(&handler);

    let listed = projects(&client, Request::RecordRecentProject { path: "/a".into() }).await;
    assert!(listed[0].opened_at > 1_700_000_000, "{listed:?}");
}
