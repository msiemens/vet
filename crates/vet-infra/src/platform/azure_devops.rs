//! `vet_app::platform::Platform` for Azure DevOps Services, on `azure_devops_rust_api`.
//!
//! One page of the repository's active pull requests carries what a queue row shows
//! except the checks and the unresolved threads, which take a request each per pull
//! request (the API has no batch for either). Those are asked for the pull requests
//! that are on the requested list only, concurrently, and reused for a while while the
//! source branch stays where it was. Votes carry no commit and the list no time of the
//! last change; the wire module says what stands in for them.
//!
//! The token goes to the host's API and nowhere else: no redirect is followed. Nothing
//! here logs a request or a response.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use azure_devops_rust_api::git::models::PullRequestStatus;
use azure_devops_rust_api::ims::models::QueryMembership;
use futures::future::try_join_all;
use url::Url;
use vet_app::platform::{
    CiState, CredentialProvider, Platform, PlatformError, PlatformKind, PlatformTraits, PullRequest, PullRequestDetail,
    PullRequestQuery, QueueTab, RemoteKey, Viewer, matches,
};

use crate::clock::unix_now;
use crate::http::http_client;
use access::{Access, PROFILE_ENDPOINT, failure};
use wire::Me;

pub(crate) mod access;
mod wire;

#[cfg(test)]
mod tests;

const TIMEOUT: Duration = Duration::from_secs(30);

/// Pull requests per page of the list.
const PAGE_SIZE: i32 = 100;

/// The most pull requests one list holds.
const MAX_LISTED: usize = 200;

/// How many pull requests have their checks and threads asked for at once.
const CONCURRENCY: usize = 6;

/// How many groups are named by one request.
const GROUPS_PER_REQUEST: usize = 50;

/// How long the checks and threads of a pull request are reused while its source branch
/// stays where it was.
const EXTRAS_MAX_AGE: i64 = 180;

/// How long who the viewer is, and their groups, are reused.
const VIEWER_MAX_AGE: i64 = 600;

/// Where the services are: the pull requests, the identities and the profile.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Endpoints {
    api: String,
    identities: String,
    profile: String,
}

impl Endpoints {
    /// An organization at `<organization>.visualstudio.com` is also at `dev.azure.com`.
    fn services() -> Self {
        Self {
            api: "https://dev.azure.com".to_owned(),
            identities: "https://vssps.dev.azure.com".to_owned(),
            profile: PROFILE_ENDPOINT.to_owned(),
        }
    }
}

/// What a pull request needs a request of its own for.
#[derive(Clone, Copy)]
struct Extras {
    ci: Option<CiState>,
    unresolved_threads: u32,
}

#[derive(Default)]
struct State {
    /// The profile by host, with when it was read.
    profiles: HashMap<String, (i64, access::Me)>,
    /// The user's groups by host and organization.
    teams: HashMap<(String, String), (i64, Vec<String>)>,
    /// By repository and pull request: the head they were read for, and when.
    extras: HashMap<(RemoteKey, u64), (i64, String, Extras)>,
}

pub struct AzureDevOpsClient<C> {
    credentials: C,
    /// Set up when first needed: loading the system's certificates is not free, and may fail.
    http: OnceLock<Result<reqwest::Client, String>>,
    /// Where the services are, if not where Azure DevOps says.
    endpoints: Option<Endpoints>,
    /// Pull requests per page of the list.
    page_size: i32,
    now: fn() -> i64,
    state: Mutex<State>,
}

/// An organization, project and repository.
struct Repository<'a> {
    organization: &'a str,
    project: &'a str,
    name: &'a str,
}

impl<C: CredentialProvider + Sync> AzureDevOpsClient<C> {
    pub fn new(credentials: C) -> Self {
        Self {
            credentials,
            http: OnceLock::new(),
            endpoints: None,
            page_size: PAGE_SIZE,
            now: unix_now,
            state: Mutex::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn endpoints(&self) -> Endpoints {
        self.endpoints.clone().unwrap_or_else(Endpoints::services)
    }

    async fn access(&self, host: &str) -> Result<Access, PlatformError> {
        let credential = self
            .credentials
            .credential(host, (self.now)())
            .await
            .ok_or_else(|| PlatformError::SignedOut(host.to_owned()))?;
        let http = self
            .http
            .get_or_init(|| http_client(TIMEOUT))
            .as_ref()
            .map_err(|why| PlatformError::Unreachable(why.clone()))?;
        Ok(Access::new(&credential, http))
    }

    /// `result`, or why it failed in terms fit to show.
    fn settle<T>(&self, host: &str, result: azure_core::Result<T>) -> Result<T, PlatformError> {
        result.map_err(|e| failure(host, &e, (self.now)()))
    }

    /// What the token may not see is nothing.
    fn or_nothing<T: Default>(result: Result<T, PlatformError>) -> Result<T, PlatformError> {
        match result {
            Err(PlatformError::Forbidden(_) | PlatformError::NotFound(_)) => Ok(T::default()),
            other => other,
        }
    }

    /// The names of the groups the user is a member of. Without the scope to read
    /// identities there are none to name.
    async fn teams(
        &self,
        access: &Access,
        host: &str,
        organization: &str,
        id: &str,
    ) -> Result<Vec<String>, PlatformError> {
        let identities = access.identities(&self.endpoints().identities)?;
        let member_of = Self::or_nothing(
            self.settle(
                host,
                identities
                    .identities_client()
                    .read_identities(organization)
                    .identity_ids(id)
                    .query_membership(QueryMembership::ExpandedUp)
                    .await,
            )
            .map(|list| {
                list.value
                    .into_iter()
                    .flat_map(|i| i.identity_base.member_of)
                    .collect::<Vec<_>>()
            }),
        )?;
        let mut names = Vec::new();
        for chunk in member_of.chunks(GROUPS_PER_REQUEST) {
            let groups = Self::or_nothing(
                self.settle(
                    host,
                    identities
                        .identities_client()
                        .read_identities(organization)
                        .descriptors(chunk.join(","))
                        .await,
                )
                .map(|list| list.value),
            )?;
            names.extend(
                groups
                    .into_iter()
                    .filter(|g| g.identity_base.is_container == Some(true))
                    .filter_map(|g| {
                        g.identity_base
                            .provider_display_name
                            .or(g.identity_base.custom_display_name)
                    }),
            );
        }
        Ok(names)
    }

    /// Who the user is in `repository`'s organization.
    async fn me(&self, access: &Access, remote: &RemoteKey, repository: &Repository<'_>) -> Result<Me, PlatformError> {
        let host = remote.host.as_str();
        let now = (self.now)();
        let cached = self
            .state()
            .profiles
            .get(host)
            .filter(|(at, _)| now - at < VIEWER_MAX_AGE)
            .map(|(_, me)| me.clone());
        let profile = match cached {
            Some(profile) => profile,
            None => {
                let profile = access.me(&self.endpoints().profile, host, now).await?;
                self.state().profiles.insert(host.to_owned(), (now, profile.clone()));
                profile
            }
        };
        let login = profile
            .email_address
            .clone()
            .or_else(|| profile.display_name.clone())
            .ok_or_else(|| PlatformError::Unexpected(format!("{host} did not say who the user is")))?;

        let key = (host.to_owned(), repository.organization.to_owned());
        let cached = self
            .state()
            .teams
            .get(&key)
            .filter(|(at, _)| now - at < VIEWER_MAX_AGE)
            .map(|(_, teams)| teams.clone());
        let teams = match (cached, profile.id.as_deref()) {
            (Some(teams), _) => teams,
            (None, Some(id)) => {
                let teams = self.teams(access, host, repository.organization, id).await?;
                self.state().teams.insert(key, (now, teams.clone()));
                teams
            }
            (None, None) => Vec::new(),
        };
        Ok(Me {
            login,
            id: profile.id,
            email: profile.email_address,
            teams,
        })
    }

    /// The checks and the unresolved threads of `pr`, from before while its source branch
    /// is where it was.
    async fn extras(
        &self,
        git: &azure_devops_rust_api::git::Client,
        remote: &RemoteKey,
        repository: &Repository<'_>,
        pr: &PullRequest,
    ) -> Result<Extras, PlatformError> {
        let now = (self.now)();
        let key = (remote.clone(), pr.number);
        if let Some((at, head, extras)) = self.state().extras.get(&key)
            && *head == pr.head
            && now - at < EXTRAS_MAX_AGE
        {
            return Ok(*extras);
        }
        let id = i32::try_from(pr.number)
            .map_err(|_| PlatformError::Unexpected(format!("{remote}: pull request {}", pr.number)))?;
        let Repository {
            organization,
            project,
            name,
        } = *repository;
        let statuses = async {
            let list = git
                .pull_request_statuses_client()
                .list(organization, name, id, project)
                .await;
            self.settle(&remote.host, list).map(|list| list.value)
        };
        let threads = async {
            let list = git
                .pull_request_threads_client()
                .list(organization, name, id, project)
                .await;
            self.settle(&remote.host, list).map(|list| list.value)
        };
        let (statuses, threads) = futures::future::join(statuses, threads).await;
        let extras = Extras {
            ci: wire::ci_state(&Self::or_nothing(statuses)?),
            unresolved_threads: wire::unresolved_threads(&Self::or_nothing(threads)?),
        };
        self.state().extras.insert(key, (now, pr.head.clone(), extras));
        Ok(extras)
    }

    /// `prs` with their checks and threads, a few at a time.
    async fn enrich(
        &self,
        git: &azure_devops_rust_api::git::Client,
        remote: &RemoteKey,
        repository: &Repository<'_>,
        prs: &mut [PullRequest],
    ) -> Result<(), PlatformError> {
        for chunk in prs.chunks_mut(CONCURRENCY) {
            let extras = try_join_all(chunk.iter().map(|pr| self.extras(git, remote, repository, pr))).await?;
            for (pr, extras) in chunk.iter_mut().zip(extras) {
                pr.ci = extras.ci;
                pr.unresolved_threads = extras.unresolved_threads;
            }
        }
        Ok(())
    }
}

impl<C: CredentialProvider + Sync> Platform for AzureDevOpsClient<C> {
    fn traits(&self) -> PlatformTraits {
        PlatformTraits::azure_devops()
    }

    async fn viewer(&self, remote: &RemoteKey) -> Result<Viewer, PlatformError> {
        let repository = repository(remote)?;
        let access = self.access(&remote.host).await?;
        let me = self.me(&access, remote, &repository).await?;
        Ok(Viewer {
            login: me.login,
            teams: me.teams,
        })
    }

    async fn list_pull_requests(
        &self,
        remote: &RemoteKey,
        query: &PullRequestQuery,
    ) -> Result<Vec<PullRequest>, PlatformError> {
        let repository = repository(remote)?;
        let access = self.access(&remote.host).await?;
        let me = self.me(&access, remote, &repository).await?;
        let git = access.git(&self.endpoints().api)?;
        let web = web_address(remote, &repository);

        let mut found = Vec::new();
        let mut skip = 0;
        loop {
            let mut request = git
                .pull_requests_client()
                .get_pull_requests(repository.organization, repository.name, repository.project)
                .search_criteria_status(PullRequestStatus::Active)
                .top(self.page_size)
                .skip(skip);
            // A team's request is not the user's own, so only the author is searched for.
            if let (QueueTab::Mine, Some(id)) = (query.tab, &me.id) {
                request = request.search_criteria_creator_id(id.clone());
            }
            let page = self.settle(&remote.host, request.await)?.value;
            found.extend(page.iter().filter_map(|pr| wire::pull_request(pr, &me, &web)));
            skip += self.page_size;
            if page.len() < usize::try_from(self.page_size).unwrap_or(usize::MAX) || found.len() >= MAX_LISTED {
                break;
            }
        }

        // Checks and threads are asked for what the list is going to hold. The check
        // filter waits for them.
        let viewer = Viewer {
            login: me.login,
            teams: me.teams,
        };
        let without_ci = PullRequestQuery {
            ci: None,
            ..query.clone()
        };
        found.retain(|pr| matches(pr, &viewer, &without_ci));
        found.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(b.number.cmp(&a.number)));
        found.truncate(MAX_LISTED);
        self.enrich(&git, remote, &repository, &mut found).await?;
        Ok(found)
    }

    async fn pull_request(&self, remote: &RemoteKey, number: u64) -> Result<PullRequestDetail, PlatformError> {
        let repository = repository(remote)?;
        let Repository {
            organization,
            project,
            name,
        } = repository;
        let not_found = || {
            PlatformError::NotFound(format!(
                "{organization}/{project}/{name}#{number} was not found on {}, or the token cannot see it",
                remote.host
            ))
        };
        let id = i32::try_from(number).map_err(|_| not_found())?;
        let access = self.access(&remote.host).await?;
        let me = self.me(&access, remote, &repository).await?;
        let git = access.git(&self.endpoints().api)?;

        let found = git
            .pull_requests_client()
            .get_pull_request(organization, name, id, project)
            .await;
        let found = self.settle(&remote.host, found).map_err(|e| match e {
            PlatformError::NotFound(_) => not_found(),
            other => other,
        })?;
        let mut pull_request = wire::pull_request(&found, &me, &web_address(remote, &repository))
            .ok_or_else(|| PlatformError::Unexpected(format!("{} has no source commit", remote)))?;
        let iterations = git
            .pull_request_iterations_client()
            .list(organization, name, id, project)
            .await;
        let iterations = self.settle(&remote.host, iterations)?.value;
        self.enrich(&git, remote, &repository, std::slice::from_mut(&mut pull_request))
            .await?;

        let (base, pushes) = wire::pushes(&iterations, &pull_request.head);
        let base = base
            .or_else(|| found.last_merge_target_commit.as_ref()?.commit_id.clone())
            .ok_or_else(|| {
                PlatformError::Unexpected(format!("{} says nothing of where {number} branched off", remote.host))
            })?;
        Ok(PullRequestDetail {
            pull_request,
            base,
            pushes,
        })
    }
}

fn repository(remote: &RemoteKey) -> Result<Repository<'_>, PlatformError> {
    let segments: Vec<&str> = remote.segments().collect();
    match segments[..] {
        [organization, project, name] if remote.kind() == PlatformKind::AzureDevOps => Ok(Repository {
            organization,
            project,
            name,
        }),
        _ => Err(PlatformError::Unexpected(format!(
            "{remote} is not an Azure DevOps repository"
        ))),
    }
}

/// The address of a repository's pull requests in the browser, up to the number.
fn web_address(remote: &RemoteKey, repository: &Repository<'_>) -> String {
    let mut url = Url::parse(&format!("https://{}", remote.host)).expect("a host name is an address");
    {
        let mut path = url.path_segments_mut().expect("a web address has a path");
        // An organization at `<organization>.visualstudio.com` is the host.
        if remote.host == "dev.azure.com" {
            path.push(repository.organization);
        }
        path.extend([repository.project, "_git", repository.name, "pullrequest", ""]);
    }
    url.to_string()
}
