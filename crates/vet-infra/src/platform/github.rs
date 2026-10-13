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

use std::backtrace::Backtrace;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use http::HeaderValue;
use http::header::USER_AGENT;
use octocrab::service::middleware::auth_header::AuthHeaderLayer;
use octocrab::service::middleware::base_uri::BaseUriLayer;
use octocrab::service::middleware::cache::mem::InMemoryCache;
use octocrab::service::middleware::cache::{CacheStorage, HttpCacheLayer};
use octocrab::service::middleware::extra_headers::ExtraHeadersLayer;
use octocrab::service::middleware::reqwest_connector::ReqwestConnector;
use octocrab::service::middleware::retry::RetryConfig;
use octocrab::{AuthState, Octocrab, OctocrabBuilder};
use reqwest::StatusCode;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use vet_app::platform::{
    CheckLog, CommentRef, CommitChecks, Credential, CredentialProvider, Discussion, PipelineAction, Platform,
    PlatformComment, PlatformError, PlatformKind, PlatformTraits, PullRequestDetail, PullRequestListing,
    PullRequestQuery, QueueTab, RemoteKey, Step, Submission, Submitted, Viewer, pushes_from_force_pushes,
};

use crate::clock::unix_now;
use crate::http::LazyClient;
use crate::platform::cache::Cache;
use crate::platform::limits::Blocked;
use crate::platform::message::{rate_limited, reason};
use crate::sync::lock;
use limits::{Limits, SECONDARY_WAIT, Watch, WatchLayer};
use tower::Layer;
use tower::retry::RetryLayer;
use wire::{DetailNode, RepositoryData, SearchData};

mod checks;
mod discussion;
mod limits;
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

/// How many entries each of the adapter's caches holds.
const CACHED: usize = 500;

/// Where a host's APIs are: the base URIs that the routes of `octocrab` (REST) and
/// `/graphql` are relative to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Endpoints {
    graphql: String,
    pub(crate) rest: String,
}

/// A client for one request that checks a credential: it shares no rate-limit state with the
/// clients of the platform.
pub(crate) fn checking_client(
    http: &reqwest::Client,
    base: &str,
    credential: &Credential,
    host: &str,
) -> octocrab::Result<Octocrab> {
    let watch = Watch {
        limits: Arc::default(),
        host: host.to_owned(),
        bucket: Bucket::Core,
        now: unix_now,
    };
    client(http, base, credential, false, watch)
}

/// A client for the API under `base` that sends `credential` to it and nowhere else. With `cache`,
/// requests are conditional on what was answered before. `watch` sees every response and holds
/// requests back while the bucket is used up.
///
/// This is the stack `octocrab` builds for a `reqwest` client, with the watch right above the
/// transport: it sees each response before a retry, and a retry meets the gate.
fn client(
    http: &reqwest::Client,
    base: &str,
    credential: &Credential,
    cache: bool,
    watch: Watch,
) -> octocrab::Result<Octocrab> {
    let base: http::Uri = base.parse().map_err(|source| octocrab::Error::Uri {
        source,
        backtrace: Backtrace::capture(),
    })?;
    let mut token = HeaderValue::from_str(&format!("Bearer {}", credential.token.expose())).map_err(|e| {
        octocrab::Error::Other {
            source: Box::new(e),
            backtrace: Backtrace::capture(),
        }
    })?;
    token.set_sensitive(true);
    let storage: Option<Arc<dyn CacheStorage>> = cache.then(|| Arc::new(InMemoryCache::new()) as _);

    let service = WatchLayer(watch).layer(ReqwestConnector::new(http.clone()));
    let service = RetryLayer::new(RetryConfig::Simple(RETRIES)).layer(service);
    let service =
        ExtraHeadersLayer::new(Arc::new(vec![(USER_AGENT, HeaderValue::from_static("octocrab"))])).layer(service);
    let service = BaseUriLayer::new(base.clone()).layer(service);
    let service = AuthHeaderLayer::new(Some(token), base.clone(), base).layer(service);
    let service = HttpCacheLayer::new(storage).layer(service);
    Ok(OctocrabBuilder::new_empty()
        .with_service(service)
        .with_auth(AuthState::None)
        .build()
        .unwrap_or_else(|never| match never {}))
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
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
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

struct State {
    /// By host; rebuilt when the credential for the host changes.
    clients: HashMap<String, (Credential, Clients)>,
    /// What the probe saw last, by repository.
    probed: Cache<RemoteKey, Option<Newest>>,
    /// Lists by repository and search.
    lists: Cache<(RemoteKey, String), PullRequestListing>,
    viewers: Cache<String, Viewer>,
    /// The checks of commits whose checks are all done, which nothing changes any more.
    checks: Cache<(RemoteKey, String), CommitChecks>,
}

impl State {
    fn new() -> Self {
        Self {
            clients: HashMap::new(),
            probed: Cache::new(CACHED),
            lists: Cache::new(CACHED),
            viewers: Cache::new(CACHED),
            checks: Cache::new(CACHED),
        }
    }
}

pub struct GitHubClient<C> {
    credentials: C,
    http: LazyClient,
    /// Where the APIs are, if not where the host says.
    endpoints: Option<Endpoints>,
    now: fn() -> i64,
    state: Mutex<State>,
    /// Which hosts' buckets are used up; kept across the rebuilds of a host's clients.
    limits: Arc<Limits>,
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
            http: LazyClient::new(TIMEOUT),
            endpoints: None,
            now: unix_now,
            state: Mutex::new(State::new()),
            limits: Arc::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    fn endpoints(&self, host: &str) -> Endpoints {
        self.endpoints.clone().unwrap_or_else(|| Endpoints::of(host))
    }

    /// The clients for `host`, built with the credential it has now.
    async fn clients(&self, host: &str) -> Result<Clients, PlatformError> {
        let Some(credential) = self.credentials.credential(host, (self.now)()).await else {
            return Err(match self.credentials.cli_unusable(host) {
                Some(reason) => PlatformError::CliUnusable {
                    host: host.to_owned(),
                    reason,
                },
                None => PlatformError::SignedOut(host.to_owned()),
            });
        };
        if let Some((built_for, clients)) = self.state().clients.get(host)
            && *built_for == credential
        {
            return Ok(clients.clone());
        }
        let http = self.http.get().map_err(PlatformError::Unreachable)?;
        let endpoints = self.endpoints(host);
        let unusable = |e: octocrab::Error| {
            PlatformError::Unexpected(format!("cannot set up the client for {host}: {}", short(&e)))
        };
        let watch = |bucket| Watch {
            limits: self.limits.clone(),
            host: host.to_owned(),
            bucket,
            now: self.now,
        };
        let clients = Clients {
            rest: client(http, &endpoints.rest, &credential, true, watch(Bucket::Core)).map_err(unusable)?,
            graphql: client(http, &endpoints.graphql, &credential, false, watch(Bucket::GraphQl)).map_err(unusable)?,
        };
        self.state()
            .clients
            .insert(host.to_owned(), (credential, clients.clone()));
        Ok(clients)
    }

    /// `result`, or why it failed in terms fit to show. A rate limit is logged where it is first
    /// seen (`limits`), not here.
    async fn settle<T>(&self, host: &str, bucket: Bucket, result: octocrab::Result<T>) -> Result<T, PlatformError> {
        let error = match result {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        let now = (self.now)();
        let failed = match error {
            octocrab::Error::GitHub { source, .. } => {
                let (status, message) = (source.status_code, source.message);
                let announced = self.limits.blocked_until(host, bucket, now);
                if status == StatusCode::TOO_MANY_REQUESTS
                    || (status == StatusCode::FORBIDDEN
                        && (announced.is_some() || message.to_ascii_lowercase().contains("rate limit")))
                {
                    return Err(self.limited(host, bucket));
                }
                refusal(host, status, &message)
            }
            octocrab::Error::Reqwest { source, .. } => unreachable_host(host, &source),
            // The transport's failures arrive wrapped.
            octocrab::Error::Service { source, .. } => {
                if let Some(Blocked { until }) = source.downcast_ref() {
                    return Err(rate_limited(host, *until, now));
                }
                match source.downcast_ref::<octocrab::Error>() {
                    Some(octocrab::Error::Reqwest { source, .. }) => unreachable_host(host, source),
                    _ => PlatformError::Unreachable(format!("cannot reach {host}")),
                }
            }
            octocrab::Error::Hyper { .. } => PlatformError::Unreachable(format!("cannot reach {host}")),
            other => PlatformError::Unexpected(format!("{host} sent something that Vet cannot use: {}", short(&other))),
        };
        tracing::warn!(host, reason = %failed, "GitHub request failed");
        Err(failed)
    }

    /// The error for a response that said the bucket is used up. The limit ends when the response
    /// said (`limits` has it); a refusal that did not say, GitHub asks to wait at least a minute for.
    fn limited(&self, host: &str, bucket: Bucket) -> PlatformError {
        let now = (self.now)();
        let until = self.limits.blocked_until(host, bucket, now).unwrap_or_else(|| {
            let until = now + SECONDARY_WAIT;
            self.limits
                .block(host, bucket, until, now, "refused as rate limited without a time");
            until
        });
        rate_limited(host, until, now)
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
        let response = self.settle(host, Bucket::GraphQl, reply).await?;
        if response
            .errors
            .iter()
            .any(|e| e.kind.as_deref() == Some("RATE_LIMITED"))
        {
            return Err(self.limited(host, Bucket::GraphQl));
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
        match self.settle(host, Bucket::Core, found).await {
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
        let newest = self.settle(host, Bucket::Core, found).await?.pop();
        let now = (self.now)();
        Ok(self.state().probed.insert(remote.clone(), newest.clone(), now) == Some(newest))
    }
}

impl<C: CredentialProvider + Sync> Platform for GitHubClient<C> {
    fn traits(&self) -> PlatformTraits {
        PlatformTraits::github()
    }

    async fn viewer(&self, remote: &RemoteKey) -> Result<Viewer, PlatformError> {
        let host = remote.host.as_str();
        let now = (self.now)();
        if let Some(viewer) = self.state().viewers.fresh(&host.to_owned(), now, VIEWER_MAX_AGE) {
            return Ok(viewer.clone());
        }
        let clients = self.clients(host).await?;
        let user = clients.rest.current().user().await;
        let user = self.settle(host, Bucket::Core, user).await?;
        let viewer = Viewer {
            login: user.login,
            teams: self.teams(host, &clients.rest).await?,
        };
        self.state().viewers.insert(host.to_owned(), viewer.clone(), now);
        Ok(viewer)
    }

    async fn list_pull_requests(
        &self,
        remote: &RemoteKey,
        query: &PullRequestQuery,
    ) -> Result<PullRequestListing, PlatformError> {
        let (owner, name) = repository(remote)?;
        let search = search_string(owner, name, query);
        let now = (self.now)();
        let clients = self.clients(&remote.host).await?;
        if self
            .unchanged_since_last_asked(&remote.host, &clients, remote, owner, name)
            .await?
        {
            if let Some(found) = self
                .state()
                .lists
                .fresh(&(remote.clone(), search.clone()), now, LIST_MAX_AGE)
            {
                return Ok(found.clone());
            }
        } else {
            self.state().lists.retain(|(kept, _), _| kept != remote);
        }

        let mut found = PullRequestListing::default();
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
                found.pull_requests.push(node.into_pull_request()?);
            }
            let page = data.search.page_info;
            match page.end_cursor {
                Some(cursor) if page.has_next_page && found.pull_requests.len() < MAX_LISTED => after = Some(cursor),
                _ => {
                    found.truncated = page.has_next_page;
                    break;
                }
            }
        }
        self.state().lists.insert((remote.clone(), search), found.clone(), now);
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
            let data: RepositoryData<DetailNode> = self
                .graphql(
                    &remote.host,
                    &clients,
                    &wire::detail_query(),
                    json!({ "owner": owner, "name": name, "number": number, "after": after }),
                )
                .await?;
            let node = data.pull_request().ok_or_else(not_found)?;
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

    async fn steps(&self, remote: &RemoteKey, check: &str) -> Result<Vec<Step>, PlatformError> {
        self.job_steps(remote, check).await
    }

    async fn pipeline_action(&self, remote: &RemoteKey, action: &PipelineAction) -> Result<(), PlatformError> {
        self.change_run(remote, action).await
    }

    async fn discussion(&self, remote: &RemoteKey, number: u64) -> Result<Discussion, PlatformError> {
        self.read_discussion(remote, number).await
    }

    async fn comment(&self, remote: &RemoteKey, comment: &CommentRef) -> Result<PlatformComment, PlatformError> {
        self.read_comment(&remote.host, comment).await
    }

    async fn edit_comment(
        &self,
        remote: &RemoteKey,
        comment: &CommentRef,
        body: &str,
    ) -> Result<PlatformComment, PlatformError> {
        self.write_comment(&remote.host, comment, body).await
    }

    async fn delete_comment(&self, remote: &RemoteKey, comment: &CommentRef) -> Result<(), PlatformError> {
        self.remove_comment(&remote.host, comment).await
    }

    async fn set_thread_resolved(&self, remote: &RemoteKey, thread: &str, resolved: bool) -> Result<(), PlatformError> {
        self.resolve_thread(&remote.host, thread, resolved).await
    }

    async fn submit_review(
        &self,
        remote: &RemoteKey,
        number: u64,
        submission: &Submission,
    ) -> Result<Submitted, PlatformError> {
        self.post_review(remote, number, submission).await
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

/// Why GitHub refused a request. Every message is fit to show.
fn refusal(host: &str, status: StatusCode, message: &str) -> PlatformError {
    let reason = |what: &str| reason(host, what, message, status);
    match status {
        StatusCode::UNAUTHORIZED => PlatformError::Rejected(reason("refused the token")),
        StatusCode::FORBIDDEN => PlatformError::Forbidden(reason("does not allow the token this")),
        StatusCode::NOT_FOUND => PlatformError::NotFound(reason("does not know this, or the token cannot see it")),
        // The log of a job that ran longer ago than the repository keeps logs.
        StatusCode::GONE => PlatformError::NotFound(format!("{host} no longer has this: it was deleted or expired")),
        s if s.is_server_error() => PlatformError::Unreachable(format!("{host} answered {status}")),
        _ => PlatformError::Unexpected(reason("answered")),
    }
}
