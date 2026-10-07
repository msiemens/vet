//! The recent-projects list over the protocol: it belongs to the backend, not to a session.

mod common;

use common::{Client, handler};
use vet_protocol::{Request, Response};

async fn paths(client: &Client, request: Request) -> Vec<String> {
    match client.call(request).await.unwrap() {
        Response::RecentProjects { paths } => paths,
        other => panic!("{other:?}"),
    }
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
