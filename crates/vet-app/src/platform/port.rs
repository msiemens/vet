//! The port to a hosting platform; implemented over HTTP by `vet-infra`, one adapter per
//! platform.

use super::model::{PlatformError, PlatformTraits, PullRequest, PullRequestDetail, PullRequestQuery, Viewer};
use super::queue::{PullRequestRow, rows};
use super::remote::RemoteKey;

pub trait Platform {
    fn traits(&self) -> PlatformTraits;

    /// The user the platform's credential for the host belongs to.
    fn viewer(&self) -> impl Future<Output = Result<Viewer, PlatformError>> + Send;

    /// The open pull requests of `remote` that `query` may ask for, narrowed as far as the
    /// platform can, most recently updated first. The platform lists a bounded number;
    /// [`list_rows`] applies the exact rule and the limit.
    fn list_pull_requests(
        &self,
        remote: &RemoteKey,
        query: &PullRequestQuery,
    ) -> impl Future<Output = Result<Vec<PullRequest>, PlatformError>> + Send;

    /// One pull request, with the states of its source branch the platform lets us know of.
    fn pull_request(
        &self,
        remote: &RemoteKey,
        number: u64,
    ) -> impl Future<Output = Result<PullRequestDetail, PlatformError>> + Send;
}

/// The queue's rows for `query`.
pub async fn list_rows<P: Platform>(
    platform: &P,
    remote: &RemoteKey,
    query: &PullRequestQuery,
) -> Result<Vec<PullRequestRow>, PlatformError> {
    let viewer = platform.viewer().await?;
    let candidates = platform.list_pull_requests(remote, query).await?;
    Ok(rows(candidates, &viewer, query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::model::{QueueTab, ReviewRequest};
    use crate::platform::queue::tests::{pr, viewer};

    /// A platform that lists the same pull requests for every query, as one that cannot
    /// narrow by the query would.
    struct Fixed {
        prs: Vec<PullRequest>,
    }

    impl Platform for Fixed {
        fn traits(&self) -> PlatformTraits {
            PlatformTraits::github()
        }

        async fn viewer(&self) -> Result<Viewer, PlatformError> {
            Ok(viewer())
        }

        async fn list_pull_requests(
            &self,
            _: &RemoteKey,
            _: &PullRequestQuery,
        ) -> Result<Vec<PullRequest>, PlatformError> {
            Ok(self.prs.clone())
        }

        async fn pull_request(&self, _: &RemoteKey, _: u64) -> Result<PullRequestDetail, PlatformError> {
            unreachable!("the queue lists")
        }
    }

    fn remote() -> RemoteKey {
        RemoteKey {
            host: "github.com".to_owned(),
            path: "acme/app".to_owned(),
        }
    }

    #[tokio::test]
    async fn the_rule_is_applied_to_whatever_the_platform_lists() {
        let mut waiting = pr(1);
        waiting.review_requests = vec![ReviewRequest::User("ada".to_owned())];
        let platform = Fixed {
            prs: vec![waiting, pr(2)],
        };
        let query = PullRequestQuery::new(QueueTab::WaitingForMe);
        let listed = list_rows(&platform, &remote(), &query).await.unwrap();
        assert_eq!(listed.iter().map(|r| r.number).collect::<Vec<_>>(), [1]);
    }
}
