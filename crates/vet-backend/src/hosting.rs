//! The client of whichever platform hosts a pull request, so that what is built on the platform
//! port need not know which that is.

use std::sync::Arc;

use vet_app::platform::{
    CheckLog, CommentRef, CommitChecks, Discussion, Platform, PlatformComment, PlatformError, PlatformKind,
    PlatformTraits, PullRequest, PullRequestDetail, PullRequestQuery, RemoteKey, Submission, Submitted, Viewer,
};

use crate::{AzureDevOps, GitHub};

pub(crate) enum Hosting {
    GitHub(Arc<GitHub>),
    AzureDevOps(Arc<AzureDevOps>),
}

impl Hosting {
    pub(crate) fn of(kind: PlatformKind, github: &Arc<GitHub>, azure_devops: &Arc<AzureDevOps>) -> Self {
        match kind {
            PlatformKind::GitHub => Self::GitHub(github.clone()),
            PlatformKind::AzureDevOps => Self::AzureDevOps(azure_devops.clone()),
        }
    }
}

/// Calls `$call` on the client the platform is.
macro_rules! on {
    ($self:ident, $client:ident => $call:expr) => {
        match $self {
            Hosting::GitHub($client) => $call,
            Hosting::AzureDevOps($client) => $call,
        }
    };
}

impl Platform for Hosting {
    fn traits(&self) -> PlatformTraits {
        on!(self, p => p.traits())
    }

    async fn viewer(&self, remote: &RemoteKey) -> Result<Viewer, PlatformError> {
        on!(self, p => p.viewer(remote).await)
    }

    async fn list_pull_requests(
        &self,
        remote: &RemoteKey,
        query: &PullRequestQuery,
    ) -> Result<Vec<PullRequest>, PlatformError> {
        on!(self, p => p.list_pull_requests(remote, query).await)
    }

    async fn pull_request(&self, remote: &RemoteKey, number: u64) -> Result<PullRequestDetail, PlatformError> {
        on!(self, p => p.pull_request(remote, number).await)
    }

    async fn checks(&self, remote: &RemoteKey, commit: &str) -> Result<CommitChecks, PlatformError> {
        on!(self, p => p.checks(remote, commit).await)
    }

    async fn check_log(&self, remote: &RemoteKey, check: &str) -> Result<CheckLog, PlatformError> {
        on!(self, p => p.check_log(remote, check).await)
    }

    async fn discussion(&self, remote: &RemoteKey, number: u64) -> Result<Discussion, PlatformError> {
        on!(self, p => p.discussion(remote, number).await)
    }

    async fn comment(&self, remote: &RemoteKey, comment: &CommentRef) -> Result<PlatformComment, PlatformError> {
        on!(self, p => p.comment(remote, comment).await)
    }

    async fn edit_comment(
        &self,
        remote: &RemoteKey,
        comment: &CommentRef,
        body: &str,
    ) -> Result<PlatformComment, PlatformError> {
        on!(self, p => p.edit_comment(remote, comment, body).await)
    }

    async fn delete_comment(&self, remote: &RemoteKey, comment: &CommentRef) -> Result<(), PlatformError> {
        on!(self, p => p.delete_comment(remote, comment).await)
    }

    async fn set_thread_resolved(&self, remote: &RemoteKey, thread: &str, resolved: bool) -> Result<(), PlatformError> {
        on!(self, p => p.set_thread_resolved(remote, thread, resolved).await)
    }

    async fn submit_review(
        &self,
        remote: &RemoteKey,
        number: u64,
        submission: &Submission,
    ) -> Result<Submitted, PlatformError> {
        on!(self, p => p.submit_review(remote, number, submission).await)
    }

    async fn vote(&self, remote: &RemoteKey, number: u64, verdict: &str) -> Result<(), PlatformError> {
        on!(self, p => p.vote(remote, number, verdict).await)
    }
}
