//! `vet_app::platform::Platform` for GitHub.com and GitHub Enterprise Server, on `octocrab`.
//!
//! GraphQL gives a page of pull requests with everything a queue row shows in one
//! request (the search costs about 3 points of the 5000 an hour per 50 pull requests).
//! REST serves what is cheaper there: the viewer and their teams, and a one-item
//! "anything changed in this repository?" probe. `octocrab`'s HTTP cache makes every
//! REST request conditional (a 304 is free), and an unchanged repository is answered
//! from the last list.
//!
//! The token goes to the host's API and nowhere else: no redirect is followed, and
//! `octocrab` leaves it off a request to another host, as for a `Link` header that points
//! elsewhere. Nothing here logs a request or a response.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use octocrab::Octocrab;
use octocrab::service::middleware::cache::mem::InMemoryCache;
use octocrab::service::middleware::retry::RetryConfig;
use reqwest::StatusCode;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use vet_app::platform::{
    CheckLog, CommitChecks, Credential, CredentialProvider, Platform, PlatformError, PlatformKind, PlatformTraits,
    PullRequest, PullRequestDetail, PullRequestQuery, QueueTab, RemoteKey, Viewer, pushes_from_force_pushes,
};

use crate::clock::unix_now;
use crate::http::http_client;
use wire::{DetailData, SearchData};

mod checks;
mod wire;

#[cfg(test)]
mod tests;

const TIMEOUT: Duration = Duration::from_secs(30);

/// How often a request that failed on the network or with a server error is sent again.
const RETRIES: usize = 2;

/// Pull requests per GraphQL page.
const PAGE_SIZE: u32 = 50;

/// The most pull requests one list holds, so a repository with thousands open costs a
/// few requests.
const MAX_LISTED: usize = 200;

/// How long a list is reused while the repository says nothing changed. The probe does
/// not notice what bumps no pull request, a check run finishing for one.
const LIST_MAX_AGE: i64 = 180;

/// How long who the viewer is, and their teams, are reused.
const VIEWER_MAX_AGE: i64 = 600;

/// Where a host's APIs are: the base URIs that the routes of `octocrab` (REST) and
/// `/graphql` are relative to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Endpoints {
    graphql: String,
    pub(crate) rest: String,
}

/// A client for the API under `base` that sends `credential` to it and nowhere else.
/// With `cache`, requests are conditional on what was answered before.
pub(crate) fn client(
    http: &reqwest::Client,
    base: &str,
    credential: &Credential,
    cache: bool,
) -> octocrab::Result<Octocrab> {
    let builder = Octocrab::builder()
        .base_uri(base)?
        .personal_token(credential.token.expose().to_owned())
        .add_retry_config(RetryConfig::Simple(RETRIES));
    let builder = if cache {
        builder.cache(InMemoryCache::new())
    } else {
        builder
    };
    builder.build_with_reqwest(http.clone())
}

impl Endpoints {
    pub(crate) fn of(host: &str) -> Self {
        if host == "github.com" {
            Self {
                graphql: "https://api.github.com".to_owned(),
                rest: "https://api.github.com".to_owned(),
            }
        } else {
            Self {
                graphql: format!("https://{host}/api"),
                rest: format!("https://{host}/api/v3"),
            }
        }
    }
}

/// The rate-limit bucket a request is counted in.
#[derive(Clone, Copy)]
enum Bucket {
    Core,
    GraphQl,
}

/// The clients of one host: Enterprise serves GraphQL beside, not under, its REST API.
#[derive(Clone)]
struct Clients {
    rest: Octocrab,
    graphql: Octocrab,
}

/// What the probe saw of a repository's pull requests, open or not: the one updated last.
#[derive(Clone, PartialEq, Eq, Deserialize)]
struct Newest {
    number: u64,
    updated_at: String,
}

#[derive(Default)]
struct State {
    /// By host; rebuilt when the credential for the host changes.
    clients: HashMap<String, (Credential, Clients)>,
    /// What the probe saw last, by repository.
    probed: HashMap<RemoteKey, Option<Newest>>,
    /// Lists by repository, then search; with the time they were fetched.
    lists: HashMap<RemoteKey, HashMap<String, (i64, Vec<PullRequest>)>>,
    viewers: HashMap<String, (i64, Viewer)>,
    /// The checks of commits whose checks are all done, which nothing changes any more.
    checks: HashMap<(RemoteKey, String), CommitChecks>,
}

pub struct GitHubClient<C> {
    credentials: C,
    /// Set up when first needed: loading the system's certificates is not free, and may fail.
    http: OnceLock<Result<reqwest::Client, String>>,
    /// Where the APIs are, if not where the host says.
    endpoints: Option<Endpoints>,
    now: fn() -> i64,
    state: Mutex<State>,
}

#[derive(Deserialize)]
struct GraphQlResponse<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphQlError>,
}

#[derive(Deserialize)]
struct GraphQlError {
    #[serde(rename = "type")]
    kind: Option<String>,
    message: String,
}

impl<C: CredentialProvider + Sync> GitHubClient<C> {
    pub fn new(credentials: C) -> Self {
        Self {
            credentials,
            http: OnceLock::new(),
            endpoints: None,
            now: unix_now,
            state: Mutex::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn endpoints(&self, host: &str) -> Endpoints {
        self.endpoints.clone().unwrap_or_else(|| Endpoints::of(host))
    }

    /// The clients for `host`, built with the credential it has now.
    async fn clients(&self, host: &str) -> Result<Clients, PlatformError> {
        let credential = self
            .credentials
            .credential(host, (self.now)())
            .await
            .ok_or_else(|| PlatformError::SignedOut(host.to_owned()))?;
        if let Some((built_for, clients)) = self.state().clients.get(host)
            && *built_for == credential
        {
            return Ok(clients.clone());
        }
        let http = self
            .http
            .get_or_init(|| http_client(TIMEOUT))
            .as_ref()
            .map_err(|why| PlatformError::Unreachable(why.clone()))?;
        let endpoints = self.endpoints(host);
        let unusable = |e: octocrab::Error| {
            PlatformError::Unexpected(format!("cannot set up the client for {host}: {}", short(&e)))
        };
        let clients = Clients {
            rest: client(http, &endpoints.rest, &credential, true).map_err(unusable)?,
            graphql: client(http, &endpoints.graphql, &credential, false).map_err(unusable)?,
        };
        self.state()
            .clients
            .insert(host.to_owned(), (credential, clients.clone()));
        Ok(clients)
    }

    /// `result`, or why it failed in terms fit to show.
    async fn settle<T>(
        &self,
        host: &str,
        crab: &Octocrab,
        bucket: Bucket,
        result: octocrab::Result<T>,
    ) -> Result<T, PlatformError> {
        let error = match result {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        let now = (self.now)();
        Err(match error {
            octocrab::Error::GitHub { source, .. } => {
                let (status, message) = (source.status_code, source.message);
                if status == StatusCode::TOO_MANY_REQUESTS
                    || (status == StatusCode::FORBIDDEN && message.to_ascii_lowercase().contains("rate limit"))
                {
                    rate_limited(host, reset_of(crab, bucket).await, now)
                } else {
                    refusal(host, status, &message)
                }
            }
            octocrab::Error::Reqwest { source, .. } => unreachable_host(host, &source),
            // The transport's failures arrive wrapped.
            octocrab::Error::Service { source, .. } => match source.downcast_ref::<octocrab::Error>() {
                Some(octocrab::Error::Reqwest { source, .. }) => unreachable_host(host, source),
                _ => PlatformError::Unreachable(format!("cannot reach {host}")),
            },
            octocrab::Error::Hyper { .. } => PlatformError::Unreachable(format!("cannot reach {host}")),
            other => PlatformError::Unexpected(format!("{host} sent something that Vet cannot use: {}", short(&other))),
        })
    }

    async fn graphql<T: DeserializeOwned>(
        &self,
        host: &str,
        clients: &Clients,
        query: &str,
        variables: Value,
    ) -> Result<T, PlatformError> {
        let reply: octocrab::Result<GraphQlResponse<T>> = clients
            .graphql
            .post("/graphql", Some(&json!({ "query": query, "variables": variables })))
            .await;
        let response = self.settle(host, &clients.graphql, Bucket::GraphQl, reply).await?;
        if response
            .errors
            .iter()
            .any(|e| e.kind.as_deref() == Some("RATE_LIMITED"))
        {
            let resets_at = reset_of(&clients.graphql, Bucket::GraphQl).await;
            return Err(rate_limited(host, resets_at, (self.now)()));
        }
        // Errors next to data are about parts of it (a pull request in an organization that
        // enforces SAML); the rest is still the answer.
        match (response.data, response.errors.first()) {
            (Some(data), _) => Ok(data),
            (None, Some(error)) if error.kind.as_deref() == Some("NOT_FOUND") => {
                Err(PlatformError::NotFound(format!("{host}: {}", error.message)))
            }
            (None, Some(error)) if error.kind.as_deref() == Some("FORBIDDEN") => {
                Err(PlatformError::Forbidden(format!("{host}: {}", error.message)))
            }
            (None, Some(error)) => Err(PlatformError::Unexpected(format!("{host}: {}", error.message))),
            (None, None) => Err(PlatformError::Unexpected(format!("{host} answered without data"))),
        }
    }

    /// The names of the viewer's teams. Without the scope to list them there are none
    /// to name.
    async fn teams(&self, host: &str, crab: &Octocrab) -> Result<Vec<String>, PlatformError> {
        let found = async {
            let first = crab
                .current()
                .list_all_teams_for_auth_user()
                .per_page(100u8)
                .send()
                .await?;
            crab.all_pages(first).await
        }
        .await;
        match self.settle(host, crab, Bucket::Core, found).await {
            Ok(teams) => Ok(teams
                .into_iter()
                .filter_map(|team| Some(format!("{}/{}", team.organization?.login, team.slug)))
                .collect()),
            Err(PlatformError::Forbidden(_) | PlatformError::NotFound(_)) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Whether the repository's pull requests are as they were when this was last asked:
    /// the most recently updated one (open or not) is the same, as of the same time.
    async fn unchanged_since_last_asked(
        &self,
        host: &str,
        clients: &Clients,
        remote: &RemoteKey,
        owner: &str,
        name: &str,
    ) -> Result<bool, PlatformError> {
        let found: octocrab::Result<Vec<Newest>> = clients
            .rest
            .get(
                format!("/repos/{owner}/{name}/pulls"),
                Some(&[
                    ("state", "all"),
                    ("sort", "updated"),
                    ("direction", "desc"),
                    ("per_page", "1"),
                ]),
            )
            .await;
        let newest = self.settle(host, &clients.rest, Bucket::Core, found).await?.pop();
        Ok(self.state().probed.insert(remote.clone(), newest.clone()) == Some(newest))
    }
}

impl<C: CredentialProvider + Sync> Platform for GitHubClient<C> {
    fn traits(&self) -> PlatformTraits {
        PlatformTraits::github()
    }

    async fn viewer(&self, remote: &RemoteKey) -> Result<Viewer, PlatformError> {
        let host = remote.host.as_str();
        let now = (self.now)();
        if let Some((at, viewer)) = self.state().viewers.get(host)
            && now - at < VIEWER_MAX_AGE
        {
            return Ok(viewer.clone());
        }
        let clients = self.clients(host).await?;
        let user = clients.rest.current().user().await;
        let user = self.settle(host, &clients.rest, Bucket::Core, user).await?;
        let viewer = Viewer {
            login: user.login,
            teams: self.teams(host, &clients.rest).await?,
        };
        self.state().viewers.insert(host.to_owned(), (now, viewer.clone()));
        Ok(viewer)
    }

    async fn list_pull_requests(
        &self,
        remote: &RemoteKey,
        query: &PullRequestQuery,
    ) -> Result<Vec<PullRequest>, PlatformError> {
        let (owner, name) = repository(remote)?;
        let search = search_string(owner, name, query);
        let now = (self.now)();
        let clients = self.clients(&remote.host).await?;
        if self
            .unchanged_since_last_asked(&remote.host, &clients, remote, owner, name)
            .await?
        {
            let state = self.state();
            if let Some((at, found)) = state.lists.get(remote).and_then(|by_search| by_search.get(&search))
                && now - at < LIST_MAX_AGE
            {
                return Ok(found.clone());
            }
        } else {
            self.state().lists.remove(remote);
        }

        let mut found = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let data: SearchData = self
                .graphql(
                    &remote.host,
                    &clients,
                    &wire::search_query(),
                    json!({ "q": search, "first": PAGE_SIZE, "after": after }),
                )
                .await?;
            for node in data.search.nodes.into_iter().flatten() {
                found.push(node.into_pull_request()?);
            }
            match data.search.page_info.end_cursor {
                Some(cursor) if data.search.page_info.has_next_page && found.len() < MAX_LISTED => after = Some(cursor),
                _ => break,
            }
        }
        self.state()
            .lists
            .entry(remote.clone())
            .or_default()
            .insert(search, (now, found.clone()));
        Ok(found)
    }

    async fn pull_request(&self, remote: &RemoteKey, number: u64) -> Result<PullRequestDetail, PlatformError> {
        let (owner, name) = repository(remote)?;
        let not_found = || {
            PlatformError::NotFound(format!(
                "{owner}/{name}#{number} was not found on {}, or the token cannot see it",
                remote.host
            ))
        };
        let clients = self.clients(&remote.host).await?;
        let mut detail = None;
        let mut force_pushes = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let data: DetailData = self
                .graphql(
                    &remote.host,
                    &clients,
                    &wire::detail_query(),
                    json!({ "owner": owner, "name": name, "number": number, "after": after }),
                )
                .await?;
            let node = data.repository.and_then(|r| r.pull_request).ok_or_else(not_found)?;
            let page = &node.timeline_items.page_info;
            let next = page.end_cursor.clone().filter(|_| page.has_next_page);
            if detail.is_none() {
                let (first, pushes) = node.into_detail()?;
                detail = Some(first);
                force_pushes.extend(pushes);
            } else {
                force_pushes.extend(wire::force_pushes(node.timeline_items.nodes)?);
            }
            match next {
                Some(cursor) => after = Some(cursor),
                None => break,
            }
        }
        let mut detail = detail.ok_or_else(not_found)?;
        detail.pushes = pushes_from_force_pushes(&force_pushes, &detail.pull_request.head);
        Ok(detail)
    }

    async fn checks(&self, remote: &RemoteKey, commit: &str) -> Result<CommitChecks, PlatformError> {
        self.commit_checks(remote, commit).await
    }

    async fn check_log(&self, remote: &RemoteKey, check: &str) -> Result<CheckLog, PlatformError> {
        self.job_log(remote, check).await
    }
}

fn repository(remote: &RemoteKey) -> Result<(&str, &str), PlatformError> {
    let github = remote.kind() == PlatformKind::GitHub;
    match remote.path.split_once('/') {
        Some((owner, name)) if github && !name.contains('/') => Ok((owner, name)),
        _ => Err(PlatformError::Unexpected(format!(
            "{remote} is not a GitHub repository"
        ))),
    }
}

/// GitHub's search for the open pull requests that `query` asks for. Qualifiers that
/// cannot be said (a quote in a label) are left to `vet_app::platform::matches`.
fn search_string(owner: &str, name: &str, query: &PullRequestQuery) -> String {
    let mut search = format!("repo:{owner}/{name} is:pr is:open sort:updated-desc");
    search.push_str(match query.tab {
        QueueTab::WaitingForMe => " review-requested:@me",
        QueueTab::Mine => " author:@me",
        QueueTab::Assigned => " assignee:@me",
        QueueTab::AllOpen => "",
    });
    // A second author would be an or, not an and.
    if let Some(author) = query
        .author
        .as_deref()
        .filter(|a| query.tab != QueueTab::Mine && a.chars().all(is_login_char))
    {
        search.push_str(&format!(" author:{author}"));
    }
    if let Some(label) = query.label.as_deref().filter(|l| !l.contains('"')) {
        search.push_str(&format!(" label:\"{label}\""));
    }
    if let Some(draft) = query.draft {
        search.push_str(&format!(" draft:{draft}"));
    }
    search
}

fn is_login_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '[' || c == ']'
}

/// When the bucket is used up, if it is and the platform says. `octocrab` reports a
/// refusal without the rate-limit headers, so the limits are asked for (which costs
/// nothing). A secondary limit leaves the bucket unspent and has no time to give.
async fn reset_of(crab: &Octocrab, bucket: Bucket) -> Option<i64> {
    let limits = crab.ratelimit().get().await.ok()?;
    let rate = match bucket {
        Bucket::Core => limits.resources.core,
        Bucket::GraphQl => limits.resources.graphql?,
    };
    (rate.remaining == 0).then(|| i64::try_from(rate.reset).ok())?
}

fn unreachable_host(host: &str, error: &reqwest::Error) -> PlatformError {
    let why = if error.is_timeout() {
        "timed out"
    } else if error.is_connect() {
        "connection failed"
    } else {
        "request failed"
    };
    PlatformError::Unreachable(format!("cannot reach {host}: {why}"))
}

/// What an `octocrab` error says, without the backtrace.
fn short(error: &octocrab::Error) -> String {
    error.to_string().lines().next().unwrap_or_default().to_owned()
}

fn rate_limited(host: &str, resets_at: Option<i64>, now: i64) -> PlatformError {
    let when = resets_at.map_or_else(
        || "; try again in a minute".to_owned(),
        |at| format!("; it resets in {}", duration_in_words(at - now)),
    );
    PlatformError::RateLimited {
        message: format!("{host}'s rate limit is used up{when}"),
        retry_at: resets_at,
    }
}

fn duration_in_words(seconds: i64) -> String {
    let minutes = u64::try_from(seconds).unwrap_or(0).div_ceil(60).max(1);
    match minutes {
        1 => "a minute".to_owned(),
        2..=119 => format!("{minutes} minutes"),
        _ => format!("{} hours", minutes.div_ceil(60)),
    }
}

/// Why GitHub refused a request. Every message is fit to show.
fn refusal(host: &str, status: StatusCode, message: &str) -> PlatformError {
    let reason = |what: &str| {
        if message.is_empty() {
            format!("{host} {what} ({status})")
        } else {
            format!("{host} {what}: {message} ({status})")
        }
    };
    match status {
        StatusCode::UNAUTHORIZED => PlatformError::Rejected(reason("refused the token")),
        StatusCode::FORBIDDEN => PlatformError::Forbidden(reason("does not allow the token this")),
        StatusCode::NOT_FOUND => PlatformError::NotFound(reason("does not know this, or the token cannot see it")),
        s if s.is_server_error() => PlatformError::Unreachable(format!("{host} answered {status}")),
        _ => PlatformError::Unexpected(reason("answered")),
    }
}
