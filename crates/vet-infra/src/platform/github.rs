//! `vet_app::platform::Platform` for GitHub.com and GitHub Enterprise Server.
//!
//! GraphQL gives a page of pull requests with everything a queue row shows in one
//! request (the search costs about 3 points of the 5000 an hour per 50 pull requests).
//! REST serves what is cheaper there: the viewer and their teams, and a one-item
//! "anything changed in this repository?" probe. Both are conditional requests (a 304
//! is free), and an unchanged repository is answered from the last list.
//!
//! The token goes to the host's API and nowhere else: no redirect is followed, and a
//! `Link` header pointing elsewhere is not either. Nothing here logs a request or a
//! response.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use reqwest::header::HeaderMap;
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use vet_app::platform::{
    CredentialProvider, Platform, PlatformError, PlatformKind, PlatformTraits, PullRequest, PullRequestDetail,
    PullRequestQuery, QueueTab, RemoteKey, Viewer, pushes_from_force_pushes,
};

use crate::clock::unix_now;
use crate::http::http_client;
use wire::{DetailData, SearchData, TeamDocument, UserDocument};

mod wire;

#[cfg(test)]
mod tests;

const TIMEOUT: Duration = Duration::from_secs(30);

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

/// Where a host's APIs are.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Endpoints {
    graphql: String,
    rest: String,
}

impl Endpoints {
    fn of(host: &str) -> Self {
        if host == "github.com" {
            Self {
                graphql: "https://api.github.com/graphql".to_owned(),
                rest: "https://api.github.com".to_owned(),
            }
        } else {
            Self {
                graphql: format!("https://{host}/api/graphql"),
                rest: format!("https://{host}/api/v3"),
            }
        }
    }
}

/// A REST document and what it was tagged with.
#[derive(Clone)]
struct Document {
    etag: String,
    body: String,
    next: Option<String>,
}

/// How many requests a bucket of the rate limit has left until when.
#[derive(Clone, Copy)]
struct Limit {
    remaining: u64,
    resets_at: i64,
}

#[derive(Default)]
struct State {
    /// By host and resource (`core`, `graphql`).
    limits: HashMap<(String, String), Limit>,
    /// REST documents by URL.
    documents: HashMap<String, Document>,
    /// Lists by repository, then search; with the time they were fetched.
    lists: HashMap<RemoteKey, HashMap<String, (i64, Vec<PullRequest>)>>,
    viewers: HashMap<String, (i64, Viewer)>,
}

pub struct GitHubClient<C> {
    credentials: C,
    /// Set up when first needed: loading the system's certificates is not free, and may fail.
    client: OnceLock<Result<reqwest::Client, String>>,
    /// Where the APIs are, if not where the host says.
    endpoints: Option<Endpoints>,
    now: fn() -> i64,
    state: Mutex<State>,
}

struct Request<'a> {
    method: Method,
    url: &'a str,
    etag: Option<&'a str>,
    json: Option<Value>,
    /// The rate-limit bucket the request is counted in.
    resource: &'static str,
}

struct Reply {
    status: StatusCode,
    etag: Option<String>,
    next: Option<String>,
    body: String,
}

/// A REST document, from the platform or, if it was not modified, from before.
struct Fetched {
    body: String,
    next: Option<String>,
    unchanged: bool,
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
            client: OnceLock::new(),
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

    fn http(&self) -> Result<&reqwest::Client, PlatformError> {
        self.client
            .get_or_init(|| http_client(TIMEOUT))
            .as_ref()
            .map_err(|why| PlatformError::Unreachable(why.clone()))
    }

    async fn send(&self, host: &str, request: Request<'_>) -> Result<Reply, PlatformError> {
        let now = (self.now)();
        let credential = self
            .credentials
            .credential(host, now)
            .await
            .ok_or_else(|| PlatformError::SignedOut(host.to_owned()))?;
        let bucket = (host.to_owned(), request.resource.to_owned());
        if let Some(limit) = self.state().limits.get(&bucket).copied()
            && limit.remaining == 0
            && limit.resets_at > now
        {
            return Err(rate_limited(host, Some(limit.resets_at), now));
        }
        let mut builder = self
            .http()?
            .request(request.method, request.url)
            .bearer_auth(credential.token.expose())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(etag) = request.etag {
            builder = builder.header("If-None-Match", etag);
        }
        if let Some(json) = &request.json {
            builder = builder.json(json);
        }
        let response = builder
            .send()
            .await
            .map_err(|e| PlatformError::Unreachable(format!("cannot reach {host}: {}", e.without_url())))?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.text().await.unwrap_or_default();

        let limit = rate_limit(&headers);
        if let Some(limit) = limit {
            self.state().limits.insert(bucket, limit);
        }
        if status.is_success() || status == StatusCode::NOT_MODIFIED {
            return Ok(Reply {
                status,
                etag: header(&headers, "etag").map(str::to_owned),
                next: header(&headers, "link").and_then(next_link),
                body,
            });
        }
        let retry_at = header(&headers, "retry-after")
            .and_then(|s| s.parse::<i64>().ok())
            .map(|secs| now + secs)
            .or(limit.map(|l| l.resets_at));
        let exhausted = limit.is_some_and(|l| l.remaining == 0) || header(&headers, "retry-after").is_some();
        Err(failure(host, status, exhausted, retry_at, &body, now))
    }

    async fn graphql<T: DeserializeOwned>(
        &self,
        host: &str,
        query: &str,
        variables: Value,
    ) -> Result<T, PlatformError> {
        let url = self.endpoints(host).graphql;
        let reply = self
            .send(
                host,
                Request {
                    method: Method::POST,
                    url: &url,
                    etag: None,
                    json: Some(json!({ "query": query, "variables": variables })),
                    resource: "graphql",
                },
            )
            .await?;
        let response: GraphQlResponse<T> = serde_json::from_str(&reply.body).map_err(unreadable)?;
        if response
            .errors
            .iter()
            .any(|e| e.kind.as_deref() == Some("RATE_LIMITED"))
        {
            let resets_at = self
                .state()
                .limits
                .get(&(host.to_owned(), "graphql".to_owned()))
                .map(|l| l.resets_at);
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

    /// A REST document by URL, as a conditional request against what was fetched before.
    /// `url` is on the host's API.
    async fn rest(&self, host: &str, url: &str) -> Result<Fetched, PlatformError> {
        let rest = self.endpoints(host).rest;
        if !url.starts_with(&rest) {
            return Err(PlatformError::Unexpected(format!(
                "{host} pointed to {url}, which is not its API"
            )));
        }
        let cached = self.state().documents.get(url).cloned();
        let reply = self
            .send(
                host,
                Request {
                    method: Method::GET,
                    url,
                    etag: cached.as_ref().map(|d| d.etag.as_str()),
                    json: None,
                    resource: "core",
                },
            )
            .await?;
        if reply.status == StatusCode::NOT_MODIFIED {
            let cached = cached.ok_or_else(|| {
                PlatformError::Unexpected(format!("{host} answered 304 to a request that was not conditional"))
            })?;
            return Ok(Fetched {
                body: cached.body,
                next: cached.next,
                unchanged: true,
            });
        }
        if let Some(etag) = reply.etag {
            self.state().documents.insert(
                url.to_owned(),
                Document {
                    etag,
                    body: reply.body.clone(),
                    next: reply.next.clone(),
                },
            );
        }
        Ok(Fetched {
            body: reply.body,
            next: reply.next,
            unchanged: false,
        })
    }

    /// The names of the viewer's teams. Without the scope to list them there are none
    /// to name.
    async fn teams(&self, host: &str) -> Result<Vec<String>, PlatformError> {
        let mut url = format!("{}/user/teams?per_page=100", self.endpoints(host).rest);
        let mut teams = Vec::new();
        loop {
            let page = match self.rest(host, &url).await {
                Ok(page) => page,
                Err(PlatformError::Forbidden(_) | PlatformError::NotFound(_)) => return Ok(Vec::new()),
                Err(e) => return Err(e),
            };
            let documents: Vec<TeamDocument> = serde_json::from_str(&page.body).map_err(unreadable)?;
            teams.extend(documents.iter().map(TeamDocument::name));
            match page.next {
                Some(next) => url = next,
                None => return Ok(teams),
            }
        }
    }

    /// Whether the repository's pull requests are as they were when this was last asked:
    /// the most recently updated one (open or not) is as tagged before.
    async fn unchanged_since_last_asked(
        &self,
        remote: &RemoteKey,
        owner: &str,
        name: &str,
    ) -> Result<bool, PlatformError> {
        let url = format!(
            "{}/repos/{owner}/{name}/pulls?state=all&sort=updated&direction=desc&per_page=1",
            self.endpoints(&remote.host).rest
        );
        Ok(self.rest(&remote.host, &url).await?.unchanged)
    }
}

impl<C: CredentialProvider + Sync> Platform for GitHubClient<C> {
    fn traits(&self) -> PlatformTraits {
        PlatformTraits::github()
    }

    async fn viewer(&self, host: &str) -> Result<Viewer, PlatformError> {
        let now = (self.now)();
        if let Some((at, viewer)) = self.state().viewers.get(host)
            && now - at < VIEWER_MAX_AGE
        {
            return Ok(viewer.clone());
        }
        let user = self.rest(host, &format!("{}/user", self.endpoints(host).rest)).await?;
        let user: UserDocument = serde_json::from_str(&user.body).map_err(unreadable)?;
        let viewer = Viewer {
            login: user.login,
            teams: self.teams(host).await?,
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
        if self.unchanged_since_last_asked(remote, owner, name).await? {
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
        let mut detail = None;
        let mut force_pushes = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let data: DetailData = self
                .graphql(
                    &remote.host,
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

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

fn rate_limit(headers: &HeaderMap) -> Option<Limit> {
    Some(Limit {
        remaining: header(headers, "x-ratelimit-remaining")?.parse().ok()?,
        resets_at: header(headers, "x-ratelimit-reset")?.parse().ok()?,
    })
}

/// The URL of the `next` page of a `Link` header.
fn next_link(link: &str) -> Option<String> {
    link.split(',').find_map(|part| {
        let (url, params) = part.split_once(';')?;
        params
            .contains("rel=\"next\"")
            .then(|| url.trim().trim_start_matches('<').trim_end_matches('>').to_owned())
    })
}

fn unreadable(error: serde_json::Error) -> PlatformError {
    PlatformError::Unexpected(format!("GitHub sent a document that Vet cannot read: {error}"))
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
fn failure(
    host: &str,
    status: StatusCode,
    exhausted: bool,
    retry_at: Option<i64>,
    body: &str,
    now: i64,
) -> PlatformError {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|b| b.get("message")?.as_str().map(str::to_owned))
        .unwrap_or_default();
    let secondary = message.to_ascii_lowercase().contains("rate limit");
    let reason = |what: &str| {
        if message.is_empty() {
            format!("{host} {what} ({status})")
        } else {
            format!("{host} {what}: {message} ({status})")
        }
    };
    match status {
        StatusCode::TOO_MANY_REQUESTS => rate_limited(host, retry_at, now),
        StatusCode::FORBIDDEN if exhausted || secondary => rate_limited(host, retry_at, now),
        StatusCode::UNAUTHORIZED => PlatformError::Rejected(reason("refused the token")),
        StatusCode::FORBIDDEN => PlatformError::Forbidden(reason("does not allow the token this")),
        StatusCode::NOT_FOUND => PlatformError::NotFound(reason("does not know this, or the token cannot see it")),
        s if s.is_server_error() => PlatformError::Unreachable(format!("{host} answered {status}")),
        _ => PlatformError::Unexpected(reason("answered")),
    }
}
