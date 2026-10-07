//! Requests about which changeset is on view and which there are.

use vet_app::changeset::Source;
use vet_app::platform::{PlatformError, PlatformKind};
use vet_app::pull_request::{self, FetchError, hosted_repositories};
use vet_mapping as mapping;
use vet_protocol::{ChangesetKey, Event, ProjectId, Response};

use crate::fetch::Held;
use crate::{Open, Orchestrator, message, start_servers_in_background};

impl Orchestrator {
    pub(crate) async fn list_changesets(&self, project: ProjectId) -> Result<Response, String> {
        let list = self.ready(project).await?.session.changesets().await.map_err(message)?;
        Ok(mapping::changesets(&list))
    }

    pub(crate) async fn list_pull_requests(&self, project: ProjectId) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let remotes = open.open.fetch.remotes().await?;
        let repositories: Vec<_> = hosted_repositories(&remotes, PlatformKind::GitHub)
            .into_iter()
            .map(|h| h.key)
            .collect();
        let list = pull_request::list_pull_requests(&*self.github, &repositories).await;
        for source in &list.sources {
            if let Some(error) = &source.error {
                self.refused(error, &source.remote.host);
            }
        }
        let sources: Vec<_> = list
            .rows
            .iter()
            .map(|listed| mapping::pull_request_source(&listed.remote, listed.row.number))
            .collect();
        let reviews = open.session.reviews_of(&sources).await.map_err(message)?;
        Ok(mapping::pull_requests(&list, &reviews))
    }

    pub(crate) async fn open_changeset(&self, project: ProjectId, changeset: ChangesetKey) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let source = mapping::core_changeset_key(changeset)?;
        if let Source::PullRequest { remote, number } = &source {
            let brought = bring_in(&open.open, remote, *number).await;
            if let Err(FetchError::Platform(error)) = &brought {
                self.refused(error, &remote.host);
            }
            brought.map_err(|e| format!("cannot fetch pull request #{number}: {e}"))?;
        }
        let moved = open.session.open_changeset(source.clone()).await.map_err(message)?;
        if let Some(revision) = moved {
            open.subscribers.broadcast(Event::ChangesetOpened {
                project,
                revision,
                changeset: mapping::changeset_key(&source),
            });
            start_servers_in_background(open.open.clone());
        }
        Ok(Response::Done)
    }

    /// Asks the shell for the credential of `host` again when the platform refused the one the
    /// backend used (an expired token). A host with no credential at all is asked for already.
    fn refused(&self, error: &PlatformError, host: &str) {
        if matches!(error, PlatformError::Rejected(_)) {
            self.process_subscribers
                .broadcast(Event::CredentialsNeeded { host: host.to_owned() });
        }
    }
}

/// Fetches the head of pull request `number` of `remote` and its target branch.
pub(crate) async fn bring_in(
    open: &Open,
    remote: &vet_app::changeset::RemoteKey,
    number: u64,
) -> Result<(), FetchError> {
    pull_request::fetch_pull_request(&*open.github, &open.fetch, &Held(&open.accounts), remote, number).await
}
