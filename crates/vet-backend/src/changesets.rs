//! Requests about which changeset is on view and which there are.

use vet_app::changeset::{RemoteKey, Source};
use vet_app::fetch::FetchMark;
use vet_app::platform::{Platform, PlatformError, PlatformKind, commits_checks};
use vet_app::pull_request::{self, FetchError, PullRequestList, hosted_repositories, hosts_to_ask_about, on_github};
use vet_app::session::RowReview;
use vet_infra::clock::unix_now;
use vet_mapping as mapping;
use vet_protocol::{ChangesetKey, Event, ProjectId, Response};

use crate::accounts::Held;
use crate::hosting::Hosting;
use crate::open::Ready;
use crate::sync::sync_in_background;
use crate::{Live, Open, Orchestrator, message, start_servers_in_background};

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
        let on_github = on_github(candidates.clone(), &hosts_with_account);
        for skipped in candidates.iter().filter(|c| !on_github.contains(c)) {
            tracing::warn!(
                remote = %skipped.remote,
                host = %skipped.key.host,
                "not listing pull requests: the host is not github.com and has no account, so it is not known to be GitHub"
            );
        }
        let github: Vec<_> = on_github.into_iter().map(|h| h.key).collect();
        let azure_devops: Vec<_> = hosted_repositories(&remotes, PlatformKind::AzureDevOps)
            .into_iter()
            .map(|h| h.key)
            .collect();
        if github.is_empty() && azure_devops.is_empty() {
            let names: Vec<_> = remotes.iter().map(|r| r.name.as_str()).collect();
            tracing::warn!(
                ?names,
                "not listing pull requests: no remote of the repository is a hosted repository"
            );
        }
        let on_view = match open.session.source().await {
            Source::PullRequest { remote, number } => Some((remote, number)),
            _ => None,
        };
        let on_view = on_view.as_ref().map(|(remote, number)| (remote, *number));
        let since = open.open.fetch.mark();
        let list = pull_request::list_pull_requests(&*self.github, &github, on_view)
            .await
            .and(pull_request::list_pull_requests(&*self.azure_devops, &azure_devops, on_view).await);
        if let Err(error) = prune_pull_request_refs(&open, &list, since).await {
            tracing::warn!(%error, "deleting the refs of pull requests no longer listed failed");
        }
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

    pub(crate) async fn changeset_lines(
        &self,
        project: ProjectId,
        changeset: ChangesetKey,
    ) -> Result<Response, String> {
        let source = mapping::core_changeset_key(changeset)?;
        let lines = self
            .ready(project)
            .await?
            .session
            .lines_of(&source)
            .await
            .map_err(message)?;
        Ok(mapping::changeset_lines(lines))
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
        let mut renamed = false;
        if let Source::PullRequest { remote, number } = &source {
            let brought = bring_in(&open.open, &open.session, remote, *number).await;
            if let Err(FetchError::Platform(error)) = &brought {
                self.refused(error, &remote.account());
            }
            renamed = brought.map_err(|e| format!("cannot fetch pull request #{number}: {e}"))?;
        }
        let moved = open.session.open_changeset(source.clone()).await.map_err(message)?;
        if let Some(revision) = moved {
            open.subscribers.broadcast(Event::ChangesetOpened {
                project,
                revision,
                changeset: mapping::changeset_key(&source),
            });
            start_servers_in_background(open.open.clone());
        } else if renamed {
            open.subscribers.broadcast(Event::TargetChanged { project });
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

/// Deletes the refs of the pull requests fetched before `since` that `list` no longer has, unless
/// one is on view or a review of it is open: those may need their commits before the platform can
/// be asked again (a closed pull request's branch may be gone).
async fn prune_pull_request_refs(open: &Ready, list: &PullRequestList, since: FetchMark) -> Result<(), String> {
    let unlisted = pull_request::unlisted_fetched(&open.open.fetch, list).await?;
    if unlisted.is_empty() {
        return Ok(());
    }
    let sources: Vec<Source> = unlisted
        .iter()
        .map(|(remote, number)| Source::PullRequest {
            remote: remote.clone(),
            number: *number,
        })
        .collect();
    let reviews = open.session.reviews_of(&sources).await.map_err(message)?;
    let on_view = open.session.source().await;
    let doomed: Vec<_> = unlisted
        .into_iter()
        .zip(sources.iter().zip(reviews))
        .filter(|(_, (source, review))| **source != on_view && !matches!(review, RowReview::Open { .. }))
        .map(|(pull_request, _)| pull_request)
        .collect();
    let deleted = pull_request::delete_pull_request_refs(&open.open.fetch, &doomed, since).await?;
    if !deleted.is_empty() {
        tracing::info!(
            refs = deleted.len(),
            "deleted the refs of pull requests no longer listed"
        );
    }
    Ok(())
}

/// Fetches the head of pull request `number` of `remote` and its target branch, and stores how the
/// platform names it. Answers whether that is not what was stored.
pub(crate) async fn bring_in(open: &Open, session: &Live, remote: &RemoteKey, number: u64) -> Result<bool, FetchError> {
    let held = Held(&open.accounts);
    let head = match remote.kind() {
        PlatformKind::GitHub => {
            pull_request::fetch_pull_request(&*open.github, &open.fetch, &held, remote, number).await?
        }
        PlatformKind::AzureDevOps => {
            pull_request::fetch_pull_request(&*open.azure_devops, &open.fetch, &held, remote, number).await?
        }
    };
    let source = Source::PullRequest {
        remote: remote.clone(),
        number,
    };
    // The fetch is done; a title that cannot be stored is shown again after the next one.
    Ok(session
        .remember_pull_request_head(&source, head)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "storing the title of a pull request failed");
            false
        }))
}
