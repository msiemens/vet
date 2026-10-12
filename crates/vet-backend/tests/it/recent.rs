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
    let seconds = || {
        let since_epoch = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
        u32::try_from(since_epoch.unwrap().as_secs()).unwrap()
    };
    let before = seconds();
    let listed = projects(&client, record("/a")).await;
    let after = seconds();
    assert_eq!(listed.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), ["/a", "/b"]);
    assert!(
        (before..=after).contains(&listed[0].opened_at),
        "a project carries when it was recorded, in seconds: {listed:?}, between {before} and {after}"
    );
    assert_eq!(paths(&client, Request::RecentProjects).await, ["/a", "/b"]);

    let forget = Request::ForgetRecentProject { path: "/a".into() };
    assert_eq!(paths(&client, forget).await, ["/b"]);
}
