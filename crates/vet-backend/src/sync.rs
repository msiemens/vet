//! Keeping the threads of the pull request on view in step with its platform, and telling the
//! subscribers how that goes.

use std::sync::{Arc, OnceLock, Weak};

use futures::FutureExt;
use vet_app::changeset::Source;
use vet_app::platform::{RemoteKey, Standing};
use vet_app::session::{Keep, SyncReport, VoteSent};
use vet_app::supervise::{Supervisor, Trigger};
use vet_core::review::{MessageId, Verdict};
use vet_infra::clock::unix_now;
use vet_mapping as mapping;
use vet_protocol::{EditChoice, Event, ProjectId, Response};

use crate::announce::{thread_changed, threads_changed};
use crate::error::{Error, Result};
use crate::open::Booted;
use crate::supervisor::{Reason, SyncOutcome, Syncs};
use crate::{Open, Orchestrator};

impl Orchestrator {
    pub(crate) async fn edit_message(&self, project: ProjectId, id: u32, body: String) -> Result<Response> {
        let open = self.ready(project).await?;
        let thread = (open.session)
            .edit_message(&open.hosting().await?, MessageId(id.into()), body, unix_now())
            .await?;
        thread_changed(&open, &thread).await
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
        thread_changed(&open, &thread).await
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
        open.open.sync.request(vec![Reason::Requested]).wait().await;
        Ok(Response::Done)
    }

    pub(crate) async fn submit_review(
        &self,
        project: ProjectId,
        verdict: Verdict,
        body: Option<String>,
    ) -> Result<Response> {
        let open = self.ready(project).await?;
        let report = (open.session)
            .submit_review(&open.hosting().await?, verdict, body)
            .await?;
        self.after_posting(&open, project, &report).await
    }

    pub(crate) async fn post_drafts(&self, project: ProjectId) -> Result<Response> {
        let open = self.ready(project).await?;
        let report = (open.session).post_drafts(&open.hosting().await?).await?;
        self.after_posting(&open, project, &report).await
    }

    /// Tells what went wrong while posting, if anything, and syncs to bring back what is on the
    /// platform now, with a sync that begins after the post. Posted drafts are no drafts anymore,
    /// and refused ones failed, which every subscriber is told, the requester included, also when
    /// the sync finds nothing else to change.
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
        let announced = matches!(
            open.open.sync.request(vec![Reason::Posted]).wait().await,
            Some(SyncOutcome::Synced { told: true })
        );
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

    pub(crate) async fn stage_vote(&self, project: ProjectId, verdict: Verdict) -> Result<Response> {
        let open = self.ready(project).await?;
        (open.session).stage_vote(&open.hosting().await?, verdict).await?;
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
        let seen = open.session.comments_seen().await?;
        let unread = open.session.unread(&view.discussion.conversation, seen);
        let outgoing = open.session.outgoing(pending_vote.as_ref());
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
            unread,
            outgoing,
        ))))
    }
}

/// The kind of the project's syncs in its supervisor.
const KIND: &str = "sync";

/// The syncs of a project, reaching it through `entry` so a waiting sync keeps nothing open.
pub(crate) fn trigger(supervisor: &Supervisor, entry: Arc<OnceLock<Weak<Open>>>) -> Syncs {
    Trigger::new(supervisor.clone(), KIND, merge, move |reasons| {
        let entry = entry.clone();
        async move {
            let Some(open) = entry.get().and_then(Weak::upgrade) else {
                return SyncOutcome::Failed("the project was closed".into());
            };
            tracing::debug!(?reasons, "syncing the pull request");
            match open.booted().await {
                Ok(booted) => sync_pull_request(&open, booted).await,
                Err(error) => SyncOutcome::Failed(error.to_string()),
            }
        }
        .boxed()
    })
}

fn merge(into: &mut Vec<Reason>, more: Vec<Reason>) {
    for reason in more {
        if !into.contains(&reason) {
            into.push(reason);
        }
    }
}

/// Syncs the threads of the pull request on view, if one is, and tells every subscriber what
/// came of it. A platform that cannot be asked is reported as its status (`Hosting` notes it) too.
/// Cooperates with the project closing: it ends before its next step instead.
async fn sync_pull_request(open: &Arc<Open>, booted: &Booted) -> SyncOutcome {
    let Source::PullRequest { remote, number } = booted.session.source().await else {
        return SyncOutcome::NotPullRequest;
    };
    let hosting = match open.hosting_for(&remote).await {
        Ok(hosting) => hosting,
        Err(error) => return SyncOutcome::Failed(error.to_string()),
    };
    if open.supervisor.closed() {
        return SyncOutcome::Failed("the project was closed".into());
    }
    let report = match booted.session.sync_threads(&hosting).await {
        Ok(report) if report.source_changed => return SyncOutcome::SourceChanged,
        Ok(report) => report,
        Err(error) => return SyncOutcome::Failed(error.to_string()),
    };
    if open.supervisor.closed() {
        return SyncOutcome::Failed("the project was closed".into());
    }
    SyncOutcome::Synced {
        told: announce(open, booted, &remote, number, &report).await,
    }
}

/// Answers whether the subscribers were told that threads changed. A sync is also what the
/// pull request's information is read after, so the platform's status is told with it.
async fn announce(open: &Open, booted: &Booted, remote: &RemoteKey, number: u64, report: &SyncReport) -> bool {
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
    if report.listing_changed {
        // What the details the sync read add to the pull request's row of the queue, so the
        // client patches it rather than listing again.
        let changed = open
            .platforms
            .known()
            .added(remote, number)
            .map(|added| vec![mapping::pull_request_update(remote, number, &added)])
            .unwrap_or_default();
        open.subscribers.broadcast(Event::PullRequestsChanged {
            project: open.id,
            changed,
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
    fn threads_changed(inbox: &mut tokio::sync::mpsc::Receiver<ServerMessage>) -> usize {
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
