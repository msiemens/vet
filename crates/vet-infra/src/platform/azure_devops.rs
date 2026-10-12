//! `vet_app::platform::Platform` for Azure DevOps Services, on `azure_devops_rust_api`.
//!
//! One page of the repository's active pull requests carries what a queue row shows
//! except the checks and the threads (and with them the comment count and the unresolved
//! threads), which take a request each per pull
//! request (the API has no batch for either). Those are asked for the pull requests
//! that are on the requested list only, concurrently, and reused for a while while the
//! source branch stays where it was. Votes carry no commit and the list no time of the
//! last change; the wire module says what stands in for them. The time comes with the
//! threads and statuses, so the list is ordered by it once they are in.
//!
//! The token goes to the host's API and nowhere else: no redirect is followed. A failed
//! request is logged with the account, its method, address (path and query), the scheme
//! of its credential, the status, the platform's message and the start of the answer; never
//! the credential.
//!
//! Accounts are of one organization (`dev.azure.com/<organization>`), and who the user
//! is comes from that organization's `connectionData`, which takes a token scoped to it.
//!
//! A pull request's threads, votes and checks are in `discussion` and `checks`.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use azure_devops_rust_api::git::models::{GitPullRequest, PullRequestStatus};
use azure_devops_rust_api::ims::models::QueryMembership;
use futures::future::try_join_all;
use url::Url;
use vet_app::platform::{
    CheckLog, CiState, CommentRef, CommitChecks, CredentialProvider, Discussion, Platform, PlatformComment,
    PlatformError, PlatformKind, PlatformTraits, PullRequest, PullRequestDetail, PullRequestListing, PullRequestQuery,
    QueueTab, RemoteKey, Submission, Submitted, Viewer, matches,
};

use crate::clock::unix_now;
use crate::http::http_client;
use access::{API_ENDPOINT, Access, failure};
use wire::Me;

pub(crate) mod access;
mod checks;
mod discussion;
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

/// How long the groups of a user are not asked for again after the organization refused them.
const TEAMS_RETRY_AFTER: i64 = 3600;

/// Where the services are: the organizations (pull requests, who the user is) and the identities.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Endpoints {
    api: String,
    identities: String,
}

impl Endpoints {
    /// An organization at `<organization>.visualstudio.com` is also at `dev.azure.com`.
    fn services() -> Self {
        Self {
            api: API_ENDPOINT.to_owned(),
            identities: "https://vssps.dev.azure.com".to_owned(),
        }
    }
}

/// What a pull request needs a request of its own for.
#[derive(Clone, Copy)]
struct Extras {
    ci: Option<CiState>,
    unresolved_threads: u32,
    comments: u32,
    /// A policy that holds the merge back has failed.
    blocked: bool,
    /// When the pull request last changed, as its threads and statuses say.
    updated_at: i64,
}

#[derive(Default)]
struct State {
    /// The id of the project of each repository, as its pull requests say it; the policies of
    /// a pull request are asked for by it.
    project_ids: HashMap<RemoteKey, String>,
    /// The profile by host, with when it was read.
    profiles: HashMap<String, (i64, access::Me)>,
    /// The user's groups by host and organization.
    teams: HashMap<(String, String), (i64, Vec<String>)>,
    /// By host and organization: when the groups were last refused.
    teams_unreadable: HashMap<(String, String), i64>,
    /// By repository and pull request: the head they were read for, and when.
    extras: HashMap<(RemoteKey, u64), (i64, String, Extras)>,
    /// The pull request each commit that an iteration has as its source was seen in.
    pull_requests_of_commits: HashMap<(RemoteKey, String), i32>,
    /// The checks of commits whose checks are all done, which nothing changes any more.
    checks: HashMap<(RemoteKey, String), CommitChecks>,
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

/// What a request about one repository starts from.
struct Call<'a> {
    remote: &'a RemoteKey,
    repository: Repository<'a>,
    access: Access,
    git: azure_devops_rust_api::git::Client,
    me: Me,
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

    /// The names of the groups the user is a member of.
    async fn teams(
        &self,
        access: &Access,
        host: &str,
        organization: &str,
        id: &str,
    ) -> Result<Vec<String>, PlatformError> {
        let identities = access.identities(&self.endpoints().identities)?;
        let member_of = self
            .settle(
                host,
                identities
                    .identities_client()
                    .read_identities(organization)
                    .identity_ids(id)
                    .query_membership(QueryMembership::ExpandedUp)
                    .await,
            )?
            .value
            .into_iter()
            .flat_map(|i| i.identity_base.member_of)
            .collect::<Vec<_>>();
        let mut names = Vec::new();
        for chunk in member_of.chunks(GROUPS_PER_REQUEST) {
            let groups = self
                .settle(
                    host,
                    identities
                        .identities_client()
                        .read_identities(organization)
                        .descriptors(chunk.join(","))
                        .await,
                )?
                .value;
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
        let host = remote.account();
        let host = host.as_str();
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
                let profile = access
                    .me(&self.endpoints().api, repository.organization, host, now)
                    .await?;
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
        let unreadable = self
            .state()
            .teams_unreadable
            .get(&key)
            .is_some_and(|at| now - at < TEAMS_RETRY_AFTER);
        let teams = match (cached, profile.id.as_deref()) {
            (Some(teams), _) => teams,
            (None, _) if unreadable => Vec::new(),
            (None, Some(id)) => match self.teams(access, host, repository.organization, id).await {
                Ok(teams) => {
                    self.state().teams.insert(key, (now, teams.clone()));
                    teams
                }
                // The token cannot read identities, or the organization has none to give:
                // the queue works without, and is not asked again for a while.
                Err(PlatformError::Forbidden(_) | PlatformError::NotFound(_)) => {
                    if self.state().teams_unreadable.insert(key, now).is_none() {
                        tracing::warn!(
                            account = host,
                            "the groups of the user cannot be read (the token may lack the identity scope); \
                             requests to a team are not shown as waiting for the user"
                        );
                    }
                    Vec::new()
                }
                Err(other) => return Err(other),
            },
            (None, None) => Vec::new(),
        };
        Ok(Me {
            login,
            id: profile.id,
            email: profile.email_address,
            teams,
        })
    }

    async fn call<'a>(&self, remote: &'a RemoteKey) -> Result<Call<'a>, PlatformError> {
        let repository = repository(remote)?;
        let access = self.access(&remote.account()).await?;
        let me = self.me(&access, remote, &repository).await?;
        let git = access.git(&self.endpoints().api)?;
        Ok(Call {
            remote,
            repository,
            access,
            git,
            me,
        })
    }

    /// The checks and the unresolved threads of `pr`, from before while its source branch
    /// is where it was.
    async fn extras(
        &self,
        access: &Access,
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
            self.settle(&remote.account(), list).map(|list| list.value)
        };
        let threads = async {
            let list = git
                .pull_request_threads_client()
                .list(organization, name, id, project)
                .await;
            self.settle(&remote.account(), list).map(|list| list.value)
        };
        let evaluations = async {
            let Some(project_id) = self.state().project_ids.get(remote).cloned() else {
                return Ok(Vec::new());
            };
            let artifact = format!("vstfs:///CodeReview/CodeReviewId/{project_id}/{id}");
            let list = access
                .policies(&self.endpoints().api)?
                .evaluations_client()
                .list(organization, project, artifact)
                .await;
            Self::or_nothing(self.settle(&remote.account(), list).map(|list| list.value))
        };
        let (statuses, threads, evaluations) = futures::future::join3(statuses, threads, evaluations).await;
        let threads = Self::or_nothing(threads)?;
        let statuses = Self::or_nothing(statuses)?;
        let extras = Extras {
            ci: wire::ci_state(&statuses),
            unresolved_threads: wire::unresolved_threads(&threads),
            comments: wire::comment_count(&threads),
            blocked: checks::blocked_by(&evaluations?),
            updated_at: wire::last_activity(pr.updated_at, &threads, &statuses),
        };
        self.state().extras.insert(key, (now, pr.head.clone(), extras));
        Ok(extras)
    }

    /// Remembers the id of the project `remote` is in, which a pull request of it names.
    fn learn_project(&self, remote: &RemoteKey, pr: Option<&GitPullRequest>) {
        if let Some(id) = pr.and_then(|pr| pr.repository.project.id.clone()) {
            self.state().project_ids.insert(remote.clone(), id);
        }
    }

    /// `prs` with their checks, threads and policies, a few at a time.
    async fn enrich(
        &self,
        access: &Access,
        git: &azure_devops_rust_api::git::Client,
        remote: &RemoteKey,
        repository: &Repository<'_>,
        prs: &mut [PullRequest],
    ) -> Result<(), PlatformError> {
        for chunk in prs.chunks_mut(CONCURRENCY) {
            let extras = try_join_all(chunk.iter().map(|pr| self.extras(access, git, remote, repository, pr))).await?;
            for (pr, extras) in chunk.iter_mut().zip(extras) {
                pr.ci = extras.ci;
                pr.unresolved_threads = extras.unresolved_threads;
                pr.comments = extras.comments;
                pr.blocked = extras.blocked;
                pr.updated_at = extras.updated_at;
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
        let access = self.access(&remote.account()).await?;
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
    ) -> Result<PullRequestListing, PlatformError> {
        let repository = repository(remote)?;
        let access = self.access(&remote.account()).await?;
        let me = self.me(&access, remote, &repository).await?;
        let git = access.git(&self.endpoints().api)?;
        let web = web_address(remote);

        let mut found = Vec::new();
        let mut skip = 0;
        let truncated = loop {
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
            let page = self.settle(&remote.account(), request.await)?.value;
            self.learn_project(remote, page.first());
            found.extend(page.iter().filter_map(|pr| wire::pull_request(pr, &me, &web)));
            skip += self.page_size;
            if page.len() < usize::try_from(self.page_size).unwrap_or(usize::MAX) {
                break false;
            }
            if found.len() >= MAX_LISTED {
                break true;
            }
        };

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
        // The newest created are kept; the threads and statuses then say when each last changed.
        let newest_first =
            |a: &PullRequest, b: &PullRequest| b.updated_at.cmp(&a.updated_at).then(b.number.cmp(&a.number));
        found.sort_by(newest_first);
        let truncated = truncated || found.len() > MAX_LISTED;
        found.truncate(MAX_LISTED);
        self.enrich(&access, &git, remote, &repository, &mut found).await?;
        found.sort_by(newest_first);
        Ok(PullRequestListing {
            pull_requests: found,
            truncated,
        })
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
                remote.account()
            ))
        };
        let id = i32::try_from(number).map_err(|_| not_found())?;
        let access = self.access(&remote.account()).await?;
        let me = self.me(&access, remote, &repository).await?;
        let git = access.git(&self.endpoints().api)?;

        let found = git
            .pull_requests_client()
            .get_pull_request(organization, name, id, project)
            .await;
        let found = self.settle(&remote.account(), found).map_err(|e| match e {
            PlatformError::NotFound(_) => not_found(),
            other => other,
        })?;
        self.learn_project(remote, Some(&found));
        let mut pull_request = wire::pull_request(&found, &me, &web_address(remote))
            .ok_or_else(|| PlatformError::Unexpected(format!("{} has no source commit", remote)))?;
        let iterations = git
            .pull_request_iterations_client()
            .list(organization, name, id, project)
            .await;
        let iterations = self.settle(&remote.account(), iterations)?.value;
        self.enrich(
            &access,
            &git,
            remote,
            &repository,
            std::slice::from_mut(&mut pull_request),
        )
        .await?;

        let (base, pushes) = wire::pushes(&iterations, &pull_request.head);
        {
            let mut state = self.state();
            for push in &pushes {
                state
                    .pull_requests_of_commits
                    .insert((remote.clone(), push.head.clone()), id);
            }
        }
        let base = base
            .or_else(|| found.last_merge_target_commit.as_ref()?.commit_id.clone())
            .ok_or_else(|| {
                PlatformError::Unexpected(format!(
                    "{} says nothing of where {number} branched off",
                    remote.account()
                ))
            })?;
        Ok(PullRequestDetail {
            pull_request,
            base,
            pushes,
        })
    }

    async fn checks(&self, remote: &RemoteKey, commit: &str) -> Result<CommitChecks, PlatformError> {
        self.commit_checks(remote, commit).await
    }

    async fn check_log(&self, remote: &RemoteKey, check: &str) -> Result<CheckLog, PlatformError> {
        self.build_log(remote, check).await
    }

    async fn discussion(&self, remote: &RemoteKey, number: u64) -> Result<Discussion, PlatformError> {
        self.read_discussion(remote, number).await
    }

    async fn comment(&self, remote: &RemoteKey, comment: &CommentRef) -> Result<PlatformComment, PlatformError> {
        self.read_comment(remote, comment).await
    }

    async fn edit_comment(
        &self,
        remote: &RemoteKey,
        comment: &CommentRef,
        body: &str,
    ) -> Result<PlatformComment, PlatformError> {
        self.write_comment(remote, comment, body).await
    }

    async fn delete_comment(&self, remote: &RemoteKey, comment: &CommentRef) -> Result<(), PlatformError> {
        self.remove_comment(remote, comment).await
    }

    async fn set_thread_resolved(&self, remote: &RemoteKey, thread: &str, resolved: bool) -> Result<(), PlatformError> {
        self.resolve_thread(remote, thread, resolved).await
    }

    async fn submit_review(
        &self,
        remote: &RemoteKey,
        number: u64,
        submission: &Submission,
    ) -> Result<Submitted, PlatformError> {
        self.post_in_turn(remote, number, submission).await
    }

    async fn vote(&self, remote: &RemoteKey, number: u64, verdict: &str) -> Result<(), PlatformError> {
        self.set_vote(remote, number, verdict).await
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
fn web_address(remote: &RemoteKey) -> String {
    let name = remote.names().nth(2).unwrap_or_default();
    address(remote, &["_git", &name, "pullrequest", ""])
}

/// The address in the browser of `rest` in the repository's project.
fn address(remote: &RemoteKey, rest: &[&str]) -> String {
    let mut url = Url::parse(&format!("https://{}", remote.host)).expect("a host name is an address");
    {
        let mut path = url.path_segments_mut().expect("a web address has a path");
        let mut names = remote.names();
        let organization = names.next().unwrap_or_default();
        // An organization at `<organization>.visualstudio.com` is the host.
        if remote.host == "dev.azure.com" {
            path.push(&organization);
        }
        path.push(&names.next().unwrap_or_default());
        path.extend(rest);
    }
    url.to_string()
}
