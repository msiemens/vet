//! Keeping the threads of the pull request on view in step with its platform, and telling the
//! subscribers how that goes.

use std::sync::Arc;

use vet_app::changeset::Source;
use vet_app::platform::PlatformError;
use vet_app::session::{Keep, SessionError, SyncReport};
use vet_core::review::MessageId;
use vet_infra::clock::unix_now;
use vet_mapping as mapping;
use vet_protocol::{EditChoice, Event, Notification, NotificationKind, PlatformConnection, ProjectId, Response};

use crate::events::{Connection, thread_changed, threads_changed};
use crate::open::Booted;
use crate::{Open, Orchestrator, message};

impl Orchestrator {
    pub(crate) async fn edit_message(
        &self,
        project: ProjectId,
        from: &Connection,
        id: u32,
        body: String,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let thread = (open.session)
            .edit_message(&*open.open.github, MessageId(id.into()), body, unix_now())
            .await
            .map_err(message)?;
        Ok(thread_changed(&open, from, &thread).await)
    }

    pub(crate) async fn remove_message(
        &self,
        project: ProjectId,
        from: &Connection,
        id: u32,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        (open.session)
            .remove_message(&*open.open.github, MessageId(id.into()))
            .await
            .map_err(message)?;
        threads_changed(&open, from).await;
        Ok(Response::Done)
    }

    pub(crate) async fn keep_edit(
        &self,
        project: ProjectId,
        from: &Connection,
        id: u32,
        keep: EditChoice,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let keep = match keep {
            EditChoice::Vet => Keep::Vet,
            EditChoice::Remote => Keep::Remote,
        };
        let thread = (open.session)
            .keep_edit(&*open.open.github, MessageId(id.into()), keep)
            .await
            .map_err(message)?;
        Ok(thread_changed(&open, from, &thread).await)
    }

    pub(crate) async fn sync_threads(&self, project: ProjectId) -> Result<Response, String> {
        let open = self.ready(project).await?;
        if !matches!(open.session.source().await, Source::PullRequest { .. }) {
            return Err("the changeset on view is not a pull request".into());
        }
        sync_pull_request(&open.open, &open).await;
        Ok(Response::Done)
    }

    pub(crate) async fn submit_review(
        &self,
        project: ProjectId,
        verdict: String,
        body: Option<String>,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let report = (open.session)
            .submit_review(&*open.open.github, &verdict, body)
            .await
            .map_err(message)?;
        sync_pull_request(&open.open, &open).await;
        Ok(mapping::review_submitted(&report))
    }

    pub(crate) async fn pull_request_info(&self, project: ProjectId) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let Source::PullRequest { remote, .. } = open.session.source().await else {
            return Err("the changeset on view is not a pull request".into());
        };
        let view = (open.session.discussion().await).ok_or("the pull request was not synced yet")?;
        let connection = open.open.connection(&remote.host).unwrap_or(PlatformConnection::Ok);
        let status = mapping::platform_status(&remote.host, remote.kind(), connection);
        Ok(Response::PullRequestInfo(Box::new(mapping::pull_request_info(
            &view,
            remote.kind(),
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
/// came of it. A platform that cannot be asked is reported as its status and not as a failure.
pub(crate) async fn sync_pull_request(open: &Arc<Open>, booted: &Booted) {
    let Source::PullRequest { remote, .. } = booted.session.source().await else {
        return;
    };
    let (problem, report) = match booted.session.sync_threads(&*open.github).await {
        Ok(report) => (report.problem.clone(), Some(report)),
        Err(SessionError::Platform(error)) => (Some(error), None),
        // The changeset on view changed meanwhile.
        Err(_) => return,
    };
    announce(
        open,
        booted,
        &remote.host,
        remote.kind(),
        problem.as_ref(),
        report.as_ref(),
    )
    .await;
}

async fn announce(
    open: &Open,
    booted: &Booted,
    host: &str,
    kind: vet_app::platform::PlatformKind,
    problem: Option<&PlatformError>,
    report: Option<&SyncReport>,
) {
    let connection = mapping::platform_connection(problem);
    let was = open.set_connection(host, connection.clone());
    let stopped = matches!(
        connection,
        PlatformConnection::SignedOut | PlatformConnection::TokenExpired { .. }
    );
    if stopped && was.as_ref() != Some(&connection) {
        open.subscribers.broadcast(Event::Notification {
            project: open.id,
            notification: Notification {
                kind: NotificationKind::SyncStopped,
                text: match connection {
                    PlatformConnection::SignedOut => format!("Not signed in to {host}: comments are not synced."),
                    _ => format!("The token for {host} is no longer accepted: comments are not synced."),
                },
                copy_text: None,
                host: Some(host.to_owned()),
            },
        });
    }
    open.subscribers.broadcast(Event::PlatformStatus {
        project: open.id,
        status: mapping::platform_status(host, kind, connection),
    });
    let Some(report) = report else {
        return;
    };
    if report.changed {
        open.subscribers.broadcast(Event::ThreadsChanged {
            project: open.id,
            revision: booted.session.revision().await,
        });
    }
    for notice in &report.notices {
        open.subscribers.broadcast(Event::Notification {
            project: open.id,
            notification: mapping::notification(notice),
        });
    }
}
