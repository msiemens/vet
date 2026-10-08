//! A GitHub pull request as a changeset, against the real GitHub with the `gh` CLI's token.
//! Not part of the suite: `cargo test -p vet-backend --test github -- --ignored --nocapture`.
//! `VET_SMOKE_REPO` (`owner/name`, public) and `VET_SMOKE_PR` choose the pull request.

mod common;

use common::{Client, git, handler};
use vet_protocol::{ChangesetKey, Request, Response, ReviewInfo, View};

#[tokio::test]
#[ignore = "talks to github.com"]
async fn a_pull_request_from_a_fork_is_opened_and_reviewed_against_its_target() {
    let repo = std::env::var("VET_SMOKE_REPO").unwrap_or_else(|_| "msiemens/tinydb".to_owned());
    let number: u32 = std::env::var("VET_SMOKE_PR").map_or(643, |n| n.parse().unwrap());
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("project");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            &format!("https://github.com/{repo}"),
            project_dir.to_str().unwrap(),
        ],
    );
    git(&project_dir, &["config", "core.autocrlf", "false"]);
    git(&project_dir, &["config", "core.excludesFile", "/dev/null"]);

    let (handler, _data) = handler().await;
    let client = Client::new(&handler);
    let project = client.open(&project_dir).await;

    let Response::PullRequests { rows, sources } = client.call(Request::ListPullRequests { project }).await.unwrap()
    else {
        panic!("not the pull requests")
    };
    println!("sources {sources:#?}");
    println!("{} pull requests of the viewer", rows.len());
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].error, None);

    let key = ChangesetKey::PullRequest {
        remote: format!("github.com/{repo}"),
        number,
    };
    let opened = client
        .call(Request::OpenChangeset {
            project,
            changeset: key.clone(),
        })
        .await
        .unwrap();
    assert!(matches!(opened, Response::Done));

    let Response::Manifest(manifest) = client
        .call(Request::Manifest {
            project,
            view: View::Changeset,
        })
        .await
        .unwrap()
    else {
        panic!("not a manifest")
    };
    println!(
        "{} against {} ({}): {:?}",
        manifest.target.name,
        manifest.base.name,
        manifest.base.commit.id,
        manifest.files.iter().map(|f| (&f.path, f.delta)).collect::<Vec<_>>()
    );
    assert_eq!(manifest.target.changeset, key);
    assert_eq!(manifest.target.name, format!("#{number}"));
    assert_eq!(manifest.review, ReviewInfo::Browsing);
    assert!(!manifest.files.is_empty());

    let Response::Commits { commits, .. } = client.call(Request::Commits { project }).await.unwrap() else {
        panic!("not the commits")
    };
    let ids = commits.iter().map(|c| c.commit.id.clone()).collect();
    let Response::Checks { checks, error } = client.call(Request::Checks { project, commits: ids }).await.unwrap()
    else {
        panic!("not the checks")
    };
    println!("checks {checks:#?}, error {error:?}");
    assert_eq!(error, None);
}
