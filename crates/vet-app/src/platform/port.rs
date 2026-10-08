//! The port to a hosting platform; implemented over HTTP by `vet-infra`, one adapter per
//! platform.

use super::checks::{CheckLog, CommitChecks};
use super::discussion::{CommentRef, Discussion, PlatformComment, Submission, Submitted};
use super::model::{PlatformError, PlatformTraits, PullRequest, PullRequestDetail, PullRequestQuery, Viewer};
use super::queue::{PullRequestRow, rows};
use super::remote::RemoteKey;

pub trait Platform {
    fn traits(&self) -> PlatformTraits;

    /// The user the credential for the host of `remote` belongs to, with the teams that
    /// count in `remote` (an Azure DevOps team belongs to an organization).
    fn viewer(&self, remote: &RemoteKey) -> impl Future<Output = Result<Viewer, PlatformError>> + Send;

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

    /// The checks of `commit` in `remote`. A platform keeps what it has already learnt of a
    /// commit whose checks are all done, and asks again for one that has some running.
    fn checks(
        &self,
        remote: &RemoteKey,
        commit: &str,
    ) -> impl Future<Output = Result<CommitChecks, PlatformError>> + Send;

    /// The log of the check `check` (a [`Check::id`] of `remote`), with where it failed.
    fn check_log(
        &self,
        remote: &RemoteKey,
        check: &str,
    ) -> impl Future<Output = Result<CheckLog, PlatformError>> + Send;

    /// The review threads of a pull request with their comments, who reviews it, and its
    /// conversation. Comments of a review that is not submitted yet are left out.
    fn discussion(
        &self,
        remote: &RemoteKey,
        number: u64,
    ) -> impl Future<Output = Result<Discussion, PlatformError>> + Send {
        let _ = (remote, number);
        async { Err(unsupported("reading the review threads")) }
    }

    /// The comment as it is now.
    fn comment(
        &self,
        remote: &RemoteKey,
        comment: &CommentRef,
    ) -> impl Future<Output = Result<PlatformComment, PlatformError>> + Send {
        let _ = (remote, comment);
        async { Err(unsupported("reading a comment")) }
    }

    /// Replaces the text of the viewer's comment; answers with it as it is then.
    fn edit_comment(
        &self,
        remote: &RemoteKey,
        comment: &CommentRef,
        body: &str,
    ) -> impl Future<Output = Result<PlatformComment, PlatformError>> + Send {
        let _ = (remote, comment, body);
        async { Err(unsupported("editing a comment")) }
    }

    /// Removes the viewer's comment; removing the first of a thread removes the thread.
    fn delete_comment(
        &self,
        remote: &RemoteKey,
        comment: &CommentRef,
    ) -> impl Future<Output = Result<(), PlatformError>> + Send {
        let _ = (remote, comment);
        async { Err(unsupported("removing a comment")) }
    }

    /// Resolves a review thread, or opens it again.
    fn set_thread_resolved(
        &self,
        remote: &RemoteKey,
        thread: &str,
        resolved: bool,
    ) -> impl Future<Output = Result<(), PlatformError>> + Send {
        let _ = (remote, thread, resolved);
        async { Err(unsupported("resolving a thread")) }
    }

    /// Posts the viewer's drafts as one review with a verdict, on platforms that batch them
    /// ([`super::PlatformTraits::posting`]). Nothing stays pending on the platform: if the
    /// review cannot be posted whole, none of it is.
    fn submit_review(
        &self,
        remote: &RemoteKey,
        number: u64,
        submission: &Submission,
    ) -> impl Future<Output = Result<Submitted, PlatformError>> + Send {
        let _ = (remote, number, submission);
        async { Err(unsupported("submitting a review")) }
    }
}

fn unsupported(what: &str) -> PlatformError {
    PlatformError::Unexpected(format!("{what} is not supported on this platform yet"))
}

/// The queue's rows for `query`.
pub async fn list_rows<P: Platform>(
    platform: &P,
    remote: &RemoteKey,
    query: &PullRequestQuery,
) -> Result<Vec<PullRequestRow>, PlatformError> {
    let viewer = platform.viewer(remote).await?;
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

        async fn viewer(&self, _: &RemoteKey) -> Result<Viewer, PlatformError> {
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

        async fn checks(&self, _: &RemoteKey, _: &str) -> Result<CommitChecks, PlatformError> {
            unreachable!("the queue lists")
        }

        async fn check_log(&self, _: &RemoteKey, _: &str) -> Result<CheckLog, PlatformError> {
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
