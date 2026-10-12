//! The platform clients against the real services, with the token of the signed-in `gh` or `az`.
//! It prints what they read and is for looking at by hand.
//!
//! ```sh
//! cargo run -p vet-infra --example platform_smoke -- github [owner/repository]   # default cli/cli
//! cargo run -p vet-infra --example platform_smoke -- azure organization/project/repository
//! ```
//!
//! For GitHub, `VET_SMOKE_JOB` names an Actions job whose log is fetched instead of the listing.

#![allow(clippy::disallowed_methods)]

use std::process::Command;

use vet_app::platform::{
    CiState, Credential, CredentialProvider, CredentialSource, Platform, PullRequestQuery, QueueTab, RemoteKey, Token,
};
use vet_infra::platform::azure_devops::AzureDevOpsClient;
use vet_infra::platform::github::GitHubClient;

struct CliToken(String);

impl CredentialProvider for CliToken {
    fn credential(&self, _: &str, _: i64) -> impl Future<Output = Option<Credential>> + Send {
        std::future::ready(Token::new(&self.0).map(|token| Credential {
            token,
            source: CredentialSource::Cli,
        }))
    }
}

fn output(program: &str, args: &[&str]) -> String {
    let out = Command::new(program).args(args).output().expect("the CLI starts");
    assert!(
        out.status.success(),
        "{program}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8").trim().to_owned()
}

fn all_open() -> PullRequestQuery {
    PullRequestQuery::new(QueueTab::AllOpen)
}

async fn listings<P: Platform>(client: &P, remote: &RemoteKey) {
    println!("viewer {:?}", client.viewer(remote).await.unwrap());
    for tab in [
        QueueTab::AllOpen,
        QueueTab::WaitingForMe,
        QueueTab::Mine,
        QueueTab::Assigned,
    ] {
        let started = std::time::Instant::now();
        let found = client
            .list_pull_requests(remote, &PullRequestQuery::new(tab))
            .await
            .unwrap()
            .pull_requests;
        println!("{tab:?}: {} pull requests in {:?}", found.len(), started.elapsed());
        if let Some(first) = found.first() {
            println!("  {first:?}");
        }
    }
}

async fn checks<P: Platform>(client: &P, remote: &RemoteKey) {
    let found = client
        .list_pull_requests(remote, &all_open())
        .await
        .unwrap()
        .pull_requests;
    for pr in found.iter().take(5) {
        let checks = client.checks(remote, &pr.head).await.unwrap();
        println!(
            "#{} {} {:?} {}",
            pr.number,
            pr.head,
            checks.state(),
            checks.checks.len()
        );
        if let Some(check) = checks.checks.iter().find(|c| c.log && c.state == CiState::Failure) {
            let log = client.check_log(remote, &check.id).await.unwrap();
            println!(
                "  log of {}: {} lines, failure {:?}",
                check.name,
                log.text.lines().count(),
                log.failure
            );
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("github") => {
            let client = GitHubClient::new(CliToken(output("gh", &["auth", "token"])));
            let remote = RemoteKey {
                host: "github.com".to_owned(),
                path: args.next().unwrap_or_else(|| "cli/cli".to_owned()),
            };
            if let Ok(job) = std::env::var("VET_SMOKE_JOB") {
                let log = client.check_log(&remote, &job).await.unwrap();
                println!(
                    "log of {job}: {} lines, failure {:?}",
                    log.text.lines().count(),
                    log.failure
                );
                return;
            }
            listings(&client, &remote).await;
            checks(&client, &remote).await;
            let found = client
                .list_pull_requests(&remote, &all_open())
                .await
                .unwrap()
                .pull_requests;
            if let Some(first) = found.first() {
                println!("{:?}", client.pull_request(&remote, first.number).await.unwrap());
            }
        }
        Some("azure") => {
            let token = output(
                "az",
                &[
                    "account",
                    "get-access-token",
                    "--resource",
                    "499b84ac-1321-427f-aa17-267ca6975798",
                    "--query",
                    "accessToken",
                    "-o",
                    "tsv",
                ],
            );
            let client = AzureDevOpsClient::new(CliToken(token));
            let remote = RemoteKey {
                host: "dev.azure.com".to_owned(),
                path: args.next().expect("organization/project/repository"),
            };
            listings(&client, &remote).await;
            checks(&client, &remote).await;
            let found = client
                .list_pull_requests(&remote, &all_open())
                .await
                .unwrap()
                .pull_requests;
            if let Some(first) = found.first() {
                let detail = client.pull_request(&remote, first.number).await.unwrap();
                println!("{detail:?}");
                let discussion = client.discussion(&remote, first.number).await.unwrap();
                println!(
                    "{} threads, reviewers {:?}, vote {:?}, checks {:?}",
                    discussion.threads.len(),
                    discussion.reviewers,
                    discussion.viewer_verdict,
                    discussion.ci
                );
            }
        }
        _ => eprintln!("usage: platform_smoke github [owner/repository] | azure organization/project/repository"),
    }
}
