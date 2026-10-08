//! Requests about which changeset is on view and which there are.

use vet_app::changeset::{RemoteKey, Source};
use vet_app::platform::{Platform, PlatformError, PlatformKind, commits_checks};
use vet_app::pull_request::{self, FetchError, hosted_repositories, hosts_to_ask_about, on_github};
use vet_infra::clock::unix_now;
use vet_mapping as mapping;
use vet_protocol::{ChangesetKey, Event, ProjectId, Response};

use crate::accounts::Held;
use crate::hosting::Hosting;
use crate::sync::sync_in_background;
use crate::{Open, Orchestrator, message, start_servers_in_background};

impl Orchestrator {
    pub(crate) async fn list_changesets(&self, project: ProjectId) -> Result<Response, String> {
        let list = self.ready(project).await?.session.changesets().await.map_err(message)?;
        Ok(mapping::changesets(&list))
    }

    pub(crate) async fn list_pull_requests(&self, project: ProjectId) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let remotes = open.open.fetch.remotes().await?;
        let candidates = hosted_repositories(&remotes, PlatformKind::GitHub);
        let mut hosts_with_account = Vec::new();
        for host in hosts_to_ask_about(&candidates) {
            if self.accounts.has_account(&host, unix_now()).await {
                hosts_with_account.push(host);
            }
        }
        let github: Vec<_> = on_github(candidates, &hosts_with_account)
            .into_iter()
            .map(|h| h.key)
            .collect();
        let azure_devops: Vec<_> = hosted_repositories(&remotes, PlatformKind::AzureDevOps)
            .into_iter()
            .map(|h| h.key)
            .collect();
        let on_view = match open.session.source().await {
            Source::PullRequest { remote, number } => Some((remote, number)),
            _ => None,
        };
        let on_view = on_view.as_ref().map(|(remote, number)| (remote, *number));
        let known = &open.open.pull_requests;
        let list = pull_request::list_pull_requests(&*self.github, &github, on_view, known)
            .await
            .and(pull_request::list_pull_requests(&*self.azure_devops, &azure_devops, on_view, known).await);
        for source in &list.sources {
            if let Some(error) = &source.error {
                self.refused(error, &source.remote.account());
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

    /// The repository of the pull request on view, if one is.
    async fn hosted_pull_request(&self, project: ProjectId) -> Result<Option<RemoteKey>, String> {
        let open = self.ready(project).await?;
        Ok(match open.session.source().await {
            Source::PullRequest { remote, .. } => Some(remote),
            _ => None,
        })
    }

    pub(crate) async fn checks(&self, project: ProjectId, commits: Vec<String>) -> Result<Response, String> {
        let Some(remote) = self.hosted_pull_request(project).await? else {
            return Ok(mapping::checks(&[], None));
        };
        let hosting = Hosting::of(remote.kind(), &self.github, &self.azure_devops);
        let (found, error) = commits_checks(&hosting, &remote, &commits).await;
        if let Some(error) = &error {
            self.refused(error, &remote.account());
        }
        Ok(mapping::checks(&found, error.as_ref()))
    }

    pub(crate) async fn check_log(&self, project: ProjectId, run: String) -> Result<Response, String> {
        let remote = self
            .hosted_pull_request(project)
            .await?
            .ok_or("the changeset on view is not a pull request")?;
        let hosting = Hosting::of(remote.kind(), &self.github, &self.azure_devops);
        let log = hosting.check_log(&remote, &run).await.map_err(|error| {
            self.refused(&error, &remote.account());
            message(error)
        })?;
        Ok(mapping::check_log(&log))
    }

    pub(crate) async fn open_changeset(&self, project: ProjectId, changeset: ChangesetKey) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let source = mapping::core_changeset_key(changeset)?;
        if let Source::PullRequest { remote, number } = &source {
            let brought = bring_in(&open.open, remote, *number).await;
            if let Err(FetchError::Platform(error)) = &brought {
                self.refused(error, &remote.account());
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
        if matches!(source, Source::PullRequest { .. }) {
            sync_in_background(open.open.clone());
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
    let held = Held(&open.accounts);
    match remote.kind() {
        PlatformKind::GitHub => {
            pull_request::fetch_pull_request(&*open.github, &open.fetch, &held, &open.pull_requests, remote, number)
                .await
        }
        PlatformKind::AzureDevOps => {
            pull_request::fetch_pull_request(
                &*open.azure_devops,
                &open.fetch,
                &held,
                &open.pull_requests,
                remote,
                number,
            )
            .await
        }
    }
}
