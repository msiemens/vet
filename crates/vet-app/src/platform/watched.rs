//! The one way a platform is called: every call is noted in the platforms' [`Health`], and what a
//! call learns of pull requests goes into their [`PullRequestCache`].

use std::sync::Arc;

use super::checks::{CheckLog, CommitChecks, PipelineAction, Step};
use super::discussion::{CommentRef, Discussion, PlatformComment, Submission, Submitted};
use super::health::Health;
use super::host::PlatformKind;
use super::known::PullRequestCache;
use super::model::{PlatformError, PlatformTraits, PullRequestDetail, PullRequestListing, PullRequestQuery, Viewer};
use super::port::Platform;
use super::remote::RemoteKey;

/// `platform`, of the kind `kind`, with its calls watched.
pub struct Watched<P> {
    platform: P,
    kind: PlatformKind,
    health: Arc<Health>,
    known: Arc<PullRequestCache>,
}

impl<P> Watched<P> {
    pub fn new(platform: P, kind: PlatformKind, health: Arc<Health>, known: Arc<PullRequestCache>) -> Self {
        Self {
            platform,
            kind,
            health,
            known,
        }
    }

    /// `call`, a call about `remote`, noted in the health of its account.
    async fn watched<T>(
        &self,
        remote: &RemoteKey,
        call: impl Future<Output = Result<T, PlatformError>>,
    ) -> Result<T, PlatformError> {
        self.health.watch(&remote.account(), self.kind, call).await
    }
}

impl<P: Platform + Sync> Platform for Watched<P> {
    fn traits(&self) -> PlatformTraits {
        self.platform.traits()
    }

    async fn viewer(&self, remote: &RemoteKey) -> Result<Viewer, PlatformError> {
        self.watched(remote, self.platform.viewer(remote)).await
    }

    async fn list_pull_requests(
        &self,
        remote: &RemoteKey,
        query: &PullRequestQuery,
    ) -> Result<PullRequestListing, PlatformError> {
        let listing = self
            .watched(remote, self.platform.list_pull_requests(remote, query))
            .await?;
        for pull_request in &listing.pull_requests {
            self.known.observe(remote, pull_request);
        }
        Ok(listing)
    }

    async fn pull_request(&self, remote: &RemoteKey, number: u64) -> Result<PullRequestDetail, PlatformError> {
        let detail = self.watched(remote, self.platform.pull_request(remote, number)).await?;
        self.known.observe(remote, &detail.pull_request);
        Ok(detail)
    }

    async fn checks(&self, remote: &RemoteKey, commit: &str) -> Result<CommitChecks, PlatformError> {
        self.watched(remote, self.platform.checks(remote, commit)).await
    }

    fn forget_checks(&self, remote: &RemoteKey) {
        self.platform.forget_checks(remote);
    }

    async fn check_log(&self, remote: &RemoteKey, check: &str) -> Result<CheckLog, PlatformError> {
        self.watched(remote, self.platform.check_log(remote, check)).await
    }

    async fn steps(&self, remote: &RemoteKey, check: &str) -> Result<Vec<Step>, PlatformError> {
        self.watched(remote, self.platform.steps(remote, check)).await
    }

    async fn pipeline_action(&self, remote: &RemoteKey, action: &PipelineAction) -> Result<(), PlatformError> {
        self.watched(remote, self.platform.pipeline_action(remote, action))
            .await
    }

    async fn discussion(&self, remote: &RemoteKey, number: u64) -> Result<Discussion, PlatformError> {
        self.watched(remote, self.platform.discussion(remote, number)).await
    }

    async fn comment(&self, remote: &RemoteKey, comment: &CommentRef) -> Result<PlatformComment, PlatformError> {
        self.watched(remote, self.platform.comment(remote, comment)).await
    }

    async fn edit_comment(
        &self,
        remote: &RemoteKey,
        comment: &CommentRef,
        body: &str,
    ) -> Result<PlatformComment, PlatformError> {
        self.watched(remote, self.platform.edit_comment(remote, comment, body))
            .await
    }

    async fn delete_comment(&self, remote: &RemoteKey, comment: &CommentRef) -> Result<(), PlatformError> {
        self.watched(remote, self.platform.delete_comment(remote, comment))
            .await
    }

    async fn set_thread_resolved(&self, remote: &RemoteKey, thread: &str, resolved: bool) -> Result<(), PlatformError> {
        self.watched(remote, self.platform.set_thread_resolved(remote, thread, resolved))
            .await
    }

    async fn submit_review(
        &self,
        remote: &RemoteKey,
        number: u64,
        submission: &Submission,
    ) -> Result<Submitted, PlatformError> {
        self.watched(remote, self.platform.submit_review(remote, number, submission))
            .await
    }

    async fn vote(&self, remote: &RemoteKey, number: u64, verdict: &str) -> Result<(), PlatformError> {
        self.watched(remote, self.platform.vote(remote, number, verdict)).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::platform::known::tests::{remote, titled};
    use crate::platform::{PullRequest, QueueTab, Standing};

    /// A platform with the pull requests the test gives it, which fails when the test says so.
    #[derive(Default)]
    struct Hosted {
        pull_requests: Mutex<Vec<PullRequest>>,
        down: Mutex<bool>,
    }

    impl Platform for Hosted {
        fn traits(&self) -> PlatformTraits {
            PlatformTraits::github()
        }

        async fn viewer(&self, _: &RemoteKey) -> Result<Viewer, PlatformError> {
            unreachable!("only pull requests are asked for")
        }

        async fn list_pull_requests(
            &self,
            _: &RemoteKey,
            _: &PullRequestQuery,
        ) -> Result<PullRequestListing, PlatformError> {
            if *self.down.lock().unwrap() {
                return Err(PlatformError::Unreachable("down".to_owned()));
            }
            Ok(PullRequestListing {
                pull_requests: self.pull_requests.lock().unwrap().clone(),
                truncated: false,
            })
        }

        async fn pull_request(&self, _: &RemoteKey, number: u64) -> Result<PullRequestDetail, PlatformError> {
            let found = self
                .pull_requests
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.number == number)
                .cloned();
            Ok(PullRequestDetail {
                pull_request: found.ok_or_else(|| PlatformError::NotFound(format!("#{number}")))?,
                base: String::new(),
                pushes: Vec::new(),
            })
        }

        async fn checks(&self, _: &RemoteKey, _: &str) -> Result<CommitChecks, PlatformError> {
            unreachable!("only pull requests are asked for")
        }

        async fn check_log(&self, _: &RemoteKey, _: &str) -> Result<CheckLog, PlatformError> {
            unreachable!("only pull requests are asked for")
        }
    }

    fn watching() -> (Watched<Hosted>, Arc<Health>, Arc<PullRequestCache>) {
        let health = Arc::new(Health::new(|_| {}));
        let known = Arc::new(PullRequestCache::default());
        let watched = Watched::new(Hosted::default(), PlatformKind::GitHub, health.clone(), known.clone());
        (watched, health, known)
    }

    #[tokio::test]
    async fn a_listed_pull_request_and_one_asked_for_are_known_as_the_latest_of_them_says() {
        let (watched, _, known) = watching();
        *watched.platform.pull_requests.lock().unwrap() = vec![titled(7, "Add keys", 100), titled(8, "Fix", 100)];
        watched
            .list_pull_requests(&remote(), &PullRequestQuery::new(QueueTab::AllOpen))
            .await
            .unwrap();
        assert_eq!(known.head(&remote(), 7, None).unwrap().title, "Add keys");

        *watched.platform.pull_requests.lock().unwrap() = vec![titled(7, "Add idempotency keys", 200)];
        watched.pull_request(&remote(), 7).await.unwrap();

        assert_eq!(known.head(&remote(), 7, None).unwrap().title, "Add idempotency keys");
        assert_eq!(known.head(&remote(), 8, None).unwrap().title, "Fix");
    }

    #[tokio::test]
    async fn a_failing_call_is_noted_in_the_health_of_its_account_and_teaches_nothing() {
        let (watched, health, known) = watching();
        *watched.platform.pull_requests.lock().unwrap() = vec![titled(7, "Add keys", 100)];
        *watched.platform.down.lock().unwrap() = true;

        let listed = watched
            .list_pull_requests(&remote(), &PullRequestQuery::new(QueueTab::AllOpen))
            .await;

        assert!(listed.is_err());
        assert_eq!(known.head(&remote(), 7, None), None);
        assert!(matches!(health.standing("github.com"), Some(Standing::Failing(_))));
    }
}
