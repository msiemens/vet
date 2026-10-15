//! The port to a hosting platform; implemented over HTTP by `vet-infra`, one adapter per
//! platform.

use super::checks::{CheckLog, CommitChecks, PipelineAction, Step};
use super::discussion::{CommentRef, Discussion, PlatformComment, Submission, Submitted};
use super::model::{PlatformError, PlatformTraits, PullRequestDetail, PullRequestListing, PullRequestQuery, Viewer};
use super::queue::{PullRequestRow, rows};
use super::remote::RemoteKey;
use vet_core::review::Verdict;

pub trait Platform {
    fn traits(&self) -> PlatformTraits;

    /// The user the credential for the host of `remote` belongs to, with the teams that
    /// count in `remote` (an Azure DevOps team belongs to an organization).
    fn viewer(&self, remote: &RemoteKey) -> impl Future<Output = Result<Viewer, PlatformError>> + Send;

    /// The open pull requests of `remote` that `query` may ask for, narrowed as far as the
    /// platform can, most recently updated first. The platform lists a bounded number, and says
    /// when it stopped there; [`list_rows`] applies the exact rule and the limit.
    fn list_pull_requests(
        &self,
        remote: &RemoteKey,
        query: &PullRequestQuery,
    ) -> impl Future<Output = Result<PullRequestListing, PlatformError>> + Send;

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

    /// Forgets what it learnt of the checks of `remote`'s commits, so the next [`Platform::checks`]
    /// asks the platform again. For after an action on a run, whose result the platform shows late,
    /// where what was learnt of a finished commit would otherwise be the answer once more.
    fn forget_checks(&self, remote: &RemoteKey) {
        let _ = remote;
    }

    /// The log of the check `check` (a [`Check::id`] of `remote`), with where it failed.
    fn check_log(
        &self,
        remote: &RemoteKey,
        check: &str,
    ) -> impl Future<Output = Result<CheckLog, PlatformError>> + Send;

    /// The steps of the job `check` (a [`Check::id`] of `remote` with a run), in the order they run.
    fn steps(&self, remote: &RemoteKey, check: &str) -> impl Future<Output = Result<Vec<Step>, PlatformError>> + Send {
        let _ = (remote, check);
        async { Err(unsupported("reading the steps of a job")) }
    }

    /// Asks the platform to change a run. The platform does its work later; what it makes of the
    /// request shows in the next [`Platform::checks`].
    fn pipeline_action(
        &self,
        remote: &RemoteKey,
        action: &PipelineAction,
    ) -> impl Future<Output = Result<(), PlatformError>> + Send {
        let _ = (remote, action);
        async { Err(unsupported("changing a run")) }
    }

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

    /// Removes the viewer's comment. Removing the first of a thread removes the thread where
    /// [`PlatformTraits::first_comment_ends_thread`] says so; the last always does.
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

    /// Posts the viewer's drafts ([`super::PlatformTraits::posting`]). Where they batch, as one
    /// review with a verdict, and nothing stays pending on the platform: if the review cannot be
    /// posted whole, none of it is. Where they do not, one draft after the other.
    fn submit_review(
        &self,
        remote: &RemoteKey,
        number: u64,
        submission: &Submission,
    ) -> impl Future<Output = Result<Submitted, PlatformError>> + Send {
        let _ = (remote, number, submission);
        async { Err(unsupported("submitting a review")) }
    }

    /// Sets the viewer's vote on a platform that keeps it apart from posting
    /// ([`super::PlatformTraits::verdict_separate`]); `verdict` is one of its verdicts.
    fn vote(
        &self,
        remote: &RemoteKey,
        number: u64,
        verdict: &Verdict,
    ) -> impl Future<Output = Result<(), PlatformError>> + Send {
        let _ = (remote, number, verdict);
        async { Err(unsupported("voting")) }
    }
}

fn unsupported(what: &str) -> PlatformError {
    PlatformError::Unexpected(format!("{what} is not supported on this platform yet"))
}

/// What [`list_rows`] answers.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ListedRows {
    pub rows: Vec<PullRequestRow>,
    /// The platform stopped at the most it lists: a pull request that is not here may still be open.
    pub truncated: bool,
}

/// The queue's rows for `query`.
pub async fn list_rows<P: Platform>(
    platform: &P,
    remote: &RemoteKey,
    query: &PullRequestQuery,
) -> Result<ListedRows, PlatformError> {
    let viewer = platform.viewer(remote).await?;
    let listing = platform.list_pull_requests(remote, query).await?;
    Ok(ListedRows {
        rows: rows(listing.pull_requests, &viewer, query),
        truncated: listing.truncated,
    })
}

/// The checks of each of `commits` in `remote`, asked for together, in the order of `commits`. A
/// commit the platform cannot answer for is left out, and the first reason is returned beside the
/// rest, so one commit that was garbage collected does not hide the others.
pub async fn commits_checks<P: Platform>(
    platform: &P,
    remote: &RemoteKey,
    commits: &[String],
) -> (Vec<CommitChecks>, Option<PlatformError>) {
    let answers = futures::future::join_all(commits.iter().map(|commit| platform.checks(remote, commit))).await;
    let mut found = Vec::new();
    let mut error = None;
    for answer in answers {
        match answer {
            Ok(checks) => found.push(checks),
            Err(e) => error = error.or(Some(e)),
        }
    }
    (found, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::model::{PullRequest, QueueTab, ReviewRequest};
    use crate::platform::queue::tests::{pr, viewer};

    /// Knows the checks of some commits and fails for the others.
    struct Known(Vec<CommitChecks>);

    impl Platform for Known {
        fn traits(&self) -> PlatformTraits {
            PlatformTraits::github()
        }

        async fn viewer(&self, _: &RemoteKey) -> Result<Viewer, PlatformError> {
            unreachable!("only checks are asked for")
        }

        async fn list_pull_requests(
            &self,
            _: &RemoteKey,
            _: &PullRequestQuery,
        ) -> Result<PullRequestListing, PlatformError> {
            unreachable!("only checks are asked for")
        }

        async fn pull_request(&self, _: &RemoteKey, _: u64) -> Result<PullRequestDetail, PlatformError> {
            unreachable!("only checks are asked for")
        }

        async fn checks(&self, _: &RemoteKey, commit: &str) -> Result<CommitChecks, PlatformError> {
            self.0
                .iter()
                .find(|c| c.commit == commit)
                .cloned()
                .ok_or_else(|| PlatformError::NotFound(format!("no commit {commit}")))
        }

        async fn check_log(&self, _: &RemoteKey, _: &str) -> Result<CheckLog, PlatformError> {
            unreachable!("only checks are asked for")
        }
    }

    fn commit_of(id: &str) -> CommitChecks {
        CommitChecks {
            commit: id.to_owned(),
            checks: Vec::new(),
            url: None,
        }
    }

    #[tokio::test]
    async fn a_commit_the_platform_does_not_know_does_not_hide_the_others() {
        let platform = Known(vec![commit_of("b"), commit_of("d")]);
        let asked: Vec<String> = ["a", "b", "c", "d"].iter().map(|c| (*c).to_owned()).collect();
        let (found, error) = commits_checks(&platform, &remote(), &asked).await;
        assert_eq!(found.iter().map(|c| c.commit.as_str()).collect::<Vec<_>>(), ["b", "d"]);
        assert_eq!(error, Some(PlatformError::NotFound("no commit a".to_owned())));
    }

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
        ) -> Result<PullRequestListing, PlatformError> {
            Ok(PullRequestListing {
                pull_requests: self.prs.clone(),
                truncated: false,
            })
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
        assert_eq!(listed.rows.iter().map(|r| r.number).collect::<Vec<_>>(), [1]);
    }
}
