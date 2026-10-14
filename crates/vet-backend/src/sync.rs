//! Keeping the threads of the pull request on view in step with its platform, and telling the
//! subscribers how that goes.

use std::sync::Arc;

use vet_app::changeset::Source;
use vet_app::platform::{RemoteKey, Standing};
use vet_app::session::{Keep, SyncReport, VoteSent};
use vet_core::review::MessageId;
use vet_infra::clock::unix_now;
use vet_mapping as mapping;
use vet_protocol::{EditChoice, Event, ProjectId, Response};

use crate::announce::{thread_changed, threads_changed};
use crate::error::{Error, Result};
use crate::open::Booted;
use crate::{Open, Orchestrator};

impl Orchestrator {
    pub(crate) async fn edit_message(&self, project: ProjectId, id: u32, body: String) -> Result<Response> {
        let open = self.ready(project).await?;
        let thread = (open.session)
            .edit_message(&open.hosting().await?, MessageId(id.into()), body, unix_now())
            .await?;
        Ok(thread_changed(&open, &thread).await)
    }

    pub(crate) async fn remove_message(&self, project: ProjectId, id: u32) -> Result<Response> {
        let open = self.ready(project).await?;
        (open.session)
            .remove_message(&open.hosting().await?, MessageId(id.into()))
            .await?;
        threads_changed(&open).await;
        Ok(Response::Done)
    }

    pub(crate) async fn keep_edit(&self, project: ProjectId, id: u32, keep: EditChoice) -> Result<Response> {
        let open = self.ready(project).await?;
        let keep = match keep {
            EditChoice::Vet => Keep::Vet,
            EditChoice::Remote => Keep::Remote,
        };
        let thread = (open.session)
            .keep_edit(&open.hosting().await?, MessageId(id.into()), keep)
            .await?;
        Ok(thread_changed(&open, &thread).await)
    }

    pub(crate) async fn edit_conversation_comment(
        &self,
        project: ProjectId,
        comment: &str,
        body: String,
    ) -> Result<Response> {
        let open = self.ready(project).await?;
        (open.session)
            .edit_conversation_comment(&open.hosting().await?, comment, body)
            .await?;
        threads_changed(&open).await;
        Ok(Response::Done)
    }

    pub(crate) async fn delete_conversation_comment(&self, project: ProjectId, comment: &str) -> Result<Response> {
        let open = self.ready(project).await?;
        (open.session)
            .remove_conversation_comment(&open.hosting().await?, comment)
            .await?;
        threads_changed(&open).await;
        Ok(Response::Done)
    }

    pub(crate) async fn sync_threads(&self, project: ProjectId) -> Result<Response> {
        let open = self.ready(project).await?;
        if !matches!(open.session.source().await, Source::PullRequest { .. }) {
            return Err(Error::not_a_pull_request());
        }
        sync_pull_request(&open.open, &open).await;
        Ok(Response::Done)
    }

    pub(crate) async fn submit_review(
        &self,
        project: ProjectId,
        verdict: String,
        body: Option<String>,
    ) -> Result<Response> {
        let open = self.ready(project).await?;
        let report = (open.session)
            .submit_review(&open.hosting().await?, &verdict, body)
            .await?;
        self.after_posting(&open, project, &report).await
    }

    pub(crate) async fn post_drafts(&self, project: ProjectId) -> Result<Response> {
        let open = self.ready(project).await?;
        let report = (open.session).post_drafts(&open.hosting().await?).await?;
        self.after_posting(&open, project, &report).await
    }

    /// Tells what went wrong while posting, if anything, and syncs to bring back what is on the
    /// platform now. Posted drafts are no drafts anymore, and refused ones failed, which every
    /// subscriber is told, the requester included, also when the sync finds nothing else to change.
    async fn after_posting(
        &self,
        open: &crate::open::Ready,
        project: ProjectId,
        report: &vet_app::session::SubmitReport,
    ) -> Result<Response> {
        if let Some(notification) = mapping::submit_notification(report) {
            open.subscribers
                .broadcast(Event::Notification { project, notification });
        }
        let announced = sync_pull_request(&open.open, open).await;
        let changed =
            report.posted > 0 || report.failed > 0 || report.problem.is_some() || report.vote != VoteSent::NotSent;
        if changed && !announced {
            open.subscribers.broadcast(Event::ThreadsChanged {
                project,
                revision: open.session.revision().await,
            });
        }
        Ok(mapping::review_submitted(report))
    }

    pub(crate) async fn stage_vote(&self, project: ProjectId, verdict: String) -> Result<Response> {
        let open = self.ready(project).await?;
        (open.session).stage_vote(&open.hosting().await?, &verdict).await?;
        threads_changed(&open).await;
        Ok(Response::Done)
    }

    pub(crate) async fn discard_vote(&self, project: ProjectId) -> Result<Response> {
        let open = self.ready(project).await?;
        open.session.discard_vote().await?;
        threads_changed(&open).await;
        Ok(Response::Done)
    }

    pub(crate) async fn pull_request_info(&self, project: ProjectId) -> Result<Response> {
        let open = self.ready(project).await?;
        let Source::PullRequest { remote, .. } = open.session.source().await else {
            return Err(Error::not_a_pull_request());
        };
        let view = (open.session.discussion().await)
            .ok_or_else(|| Error::Unavailable("the pull request was not synced yet".into()))?;
        let pending_vote = open.session.pending_vote().await?;
        let account = remote.account();
        let kind = open.open.kind_of(&remote).await?;
        let standing = open.open.platforms.standing(&account);
        let connection = mapping::platform_connection(standing.as_ref().and_then(Standing::error));
        let status = mapping::platform_status(&account, kind, connection);
        Ok(Response::PullRequestInfo(Box::new(mapping::pull_request_info(
            &view,
            pending_vote.as_ref(),
            kind,
            status,
        ))))
    }
}

/// Syncs the pull request on view in the background, for after it was opened.
pub(crate) fn sync_in_background(open: Arc<Open>) {
    tokio::spawn(async move {
        if let Ok(booted) = open.booted().await {
            sync_pull_request(&open, booted).await;
        }
    });
}

/// Syncs the threads of the pull request on view, if one is, and tells every subscriber what
/// came of it. A platform that cannot be asked is reported as its status (`Hosting` notes it) and
/// not as a failure. Answers whether the subscribers were told that threads changed.
pub(crate) async fn sync_pull_request(open: &Arc<Open>, booted: &Booted) -> bool {
    let Source::PullRequest { remote, .. } = booted.session.source().await else {
        return false;
    };
    let Ok(hosting) = open.hosting_for(&remote).await else {
        return false;
    };
    match booted.session.sync_threads(&hosting).await {
        Ok(report) => announce(open, booted, &remote, &report).await,
        // The platform could not be asked, or the changeset on view changed meanwhile.
        Err(_) => false,
    }
}

/// Answers whether the subscribers were told that threads changed. A sync is also what the
/// pull request's information is read after, so the platform's status is told with it.
async fn announce(open: &Open, booted: &Booted, remote: &RemoteKey, report: &SyncReport) -> bool {
    let Ok(kind) = open.kind_of(remote).await else {
        tracing::warn!(repository = %remote, "not announcing a pull request of a host Vet does not know");
        return false;
    };
    let account = remote.account();
    let standing = open.platforms.standing(&account);
    let connection = mapping::platform_connection(standing.as_ref().and_then(Standing::error));
    open.subscribers.broadcast(Event::PlatformStatus {
        project: open.id,
        status: mapping::platform_status(&account, kind, connection),
    });
    if report.changed {
        open.subscribers.broadcast(Event::ThreadsChanged {
            project: open.id,
            revision: booted.session.revision().await,
        });
    }
    for notice in &report.notices {
        open.subscribers.broadcast(Event::Notification {
            project: open.id,
            notification: mapping::notification(notice, kind),
        });
    }
    report.changed
}

#[cfg(test)]
mod tests {
    use vet_app::session::SubmitReport;
    use vet_protocol::ServerMessage;
    use vet_transport::Events;

    use super::*;
    use crate::events::Connection;
    use crate::{Config, open_store};

    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// How many announcements that threads changed are waiting for the connection.
    fn threads_changed(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<ServerMessage>) -> usize {
        std::iter::from_fn(|| inbox.try_recv().ok())
            .filter(|m| {
                matches!(
                    m,
                    ServerMessage::Event {
                        event: Event::ThreadsChanged { .. }
                    }
                )
            })
            .count()
    }

    fn posted(posted: u32) -> SubmitReport {
        SubmitReport {
            posted,
            failed: 0,
            skipped: 0,
            problem: None,
            not_posted: Vec::new(),
            vote: Default::default(),
        }
    }

    #[tokio::test]
    async fn the_window_that_posted_drafts_is_told_that_threads_changed() {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q"]);
        std::fs::write(repo.path().join("a.txt"), "a\n").unwrap();
        git(repo.path(), &["add", "."]);
        git(
            repo.path(),
            &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "a"],
        );
        let data = tempfile::tempdir().unwrap();
        let orchestrator = Orchestrator::new(open_store(data.path()).await.unwrap(), Config::default());
        let (events, mut inbox) = Events::channel();
        let (project, _) = orchestrator
            .open(repo.path().to_str().unwrap(), Connection(events))
            .await
            .unwrap();
        let open = orchestrator.ready(project).await.unwrap();
        threads_changed(&mut inbox);

        orchestrator.after_posting(&open, project, &posted(0)).await.unwrap();
        assert_eq!(threads_changed(&mut inbox), 0, "nothing was posted");

        orchestrator.after_posting(&open, project, &posted(2)).await.unwrap();
        assert_eq!(threads_changed(&mut inbox), 1);
    }
}
