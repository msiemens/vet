//! What every call to Azure DevOps needs: the credential as the API crate wants it, the
//! transport that follows no redirect, and what to make of a failure.

use std::sync::Arc;

use azure_core::credentials::{AccessToken, Secret, TokenCredential, TokenRequestOptions};
use azure_core::error::ErrorKind;
use azure_core::http::headers::HeaderName;
use azure_core::http::policies::{Policy, PolicyResult, RetryHeaders, RetryPolicy};
use azure_core::http::{Context, Method, Request, RetryOptions, StatusCode, Transport, Url, check_success};
use azure_core::time::{Duration, OffsetDateTime};
use azure_devops_rust_api::{Credential as ApiCredential, build, git, ims, policy};
use serde::Deserialize;
use vet_app::platform::{Credential, CredentialSource, PlatformError};

use super::limits::{Gating, Signals, Watch};
use crate::clock::unix_now;
use crate::platform::limits::Blocked;

/// Where the organizations are.
pub(crate) const API_ENDPOINT: &str = "https://dev.azure.com";

/// How many characters of a failed response's body are logged.
const BODY_EXCERPT: usize = 200;

/// A bearer token that was obtained elsewhere (`az account get-access-token`).
#[derive(Debug)]
struct Bearer(Secret);

#[async_trait::async_trait]
impl TokenCredential for Bearer {
    async fn get_token(
        &self,
        _scopes: &[&str],
        _options: Option<TokenRequestOptions<'_>>,
    ) -> azure_core::Result<AccessToken> {
        // The expiry is the token's own business; whoever hands it out renews it.
        Ok(AccessToken::new(
            self.0.clone(),
            OffsetDateTime::now_utc() + Duration::hours(1),
        ))
    }
}

/// A credential and a transport, from which the clients of the services are made.
pub(crate) struct Access {
    credential: ApiCredential,
    transport: Transport,
    /// For the requests the API crates have no client for.
    http: reqwest::Client,
    token: String,
    source: CredentialSource,
    watch: Watch,
}

impl Access {
    /// An access that shares no rate-limit state with the others: for one request that checks a
    /// credential.
    pub(crate) fn unshared(credential: &Credential, http: &reqwest::Client, account: &str) -> Self {
        let watch = Watch {
            gate: Arc::default(),
            account: account.to_owned(),
            now: unix_now,
        };
        Self::new(credential, http, watch)
    }

    /// A personal access token goes as the password of a user with no name, a CLI token
    /// as a bearer token. `watch` holds requests back while the account's limit is used up.
    pub(super) fn new(credential: &Credential, http: &reqwest::Client, watch: Watch) -> Self {
        let token = credential.token.expose().to_owned();
        let api_credential = match credential.source {
            CredentialSource::Pat => ApiCredential::from_pat(token.clone()),
            CredentialSource::Cli => ApiCredential::from_token_credential(Arc::new(Bearer(Secret::new(token.clone())))),
        };
        Self {
            credential: api_credential,
            transport: Transport::new(Arc::new(http.clone())),
            http: http.clone(),
            token,
            source: credential.source,
            watch,
        }
    }

    fn gating(&self) -> Vec<Arc<dyn Policy>> {
        vec![Arc::new(Gating(self.watch.clone()))]
    }

    /// Pull requests, their threads, statuses and iterations.
    pub(crate) fn git(&self, endpoint: &str) -> Result<git::Client, PlatformError> {
        Ok(git::ClientBuilder::new(self.credential.clone())
            .endpoint(url(endpoint)?)
            .transport(self.transport.clone())
            .retry(retries())
            .per_call_policies(requests(self.source))
            .per_try_policies(self.gating())
            .build())
    }

    /// Builds, their timelines and logs.
    pub(crate) fn builds(&self, endpoint: &str) -> Result<build::Client, PlatformError> {
        Ok(build::ClientBuilder::new(self.credential.clone())
            .endpoint(url(endpoint)?)
            .transport(self.transport.clone())
            .retry(retries())
            .per_call_policies(requests(self.source))
            .per_try_policies(self.gating())
            .build())
    }

    /// The evaluations of a pull request's policies.
    pub(crate) fn policies(&self, endpoint: &str) -> Result<policy::Client, PlatformError> {
        Ok(policy::ClientBuilder::new(self.credential.clone())
            .endpoint(url(endpoint)?)
            .transport(self.transport.clone())
            .retry(retries())
            .per_call_policies(requests(self.source))
            .per_try_policies(self.gating())
            .build())
    }

    /// Identities and the groups they are members of.
    pub(crate) fn identities(&self, endpoint: &str) -> Result<ims::Client, PlatformError> {
        Ok(ims::ClientBuilder::new(self.credential.clone())
            .endpoint(url(endpoint)?)
            .transport(self.transport.clone())
            .retry(retries())
            .per_call_policies(requests(self.source))
            .per_try_policies(self.gating())
            .build())
    }

    /// The signed-in user, as the organization `organization` (of the account `account`)
    /// knows them. Unlike the global profile, the organization's own endpoint takes a token
    /// that is scoped to the organization. A host that answers with anything but a user,
    /// its sign-in page for a token it does not know, has refused the token.
    pub(crate) async fn me(&self, api: &str, organization: &str, account: &str, now: i64) -> Result<Me, PlatformError> {
        let address = url(&format!(
            "{}/{organization}/_apis/connectionData",
            api.trim_end_matches('/')
        ))?;
        let endpoint = endpoint(Method::Get, &address);
        let request = self.http.get(address);
        let request = match self.source {
            CredentialSource::Pat => request.basic_auth("", Some(&self.token)),
            CredentialSource::Cli => request.bearer_auth(&self.token),
        };
        let auth = scheme(self.source);
        if let Err(Blocked { until }) = self.watch.check() {
            return Err(rate_limited(account, until, now));
        }
        let response = request.send().await.map_err(|_| {
            let error = PlatformError::Unreachable(format!("cannot reach {account}"));
            log_failure(account, Some((&endpoint, auth)), None, "", &error);
            error
        })?;
        let status = response.status().as_u16();
        let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok());
        let signals = Signals {
            status,
            retry_after: header("retry-after"),
            remaining: header("x-ratelimit-remaining"),
            reset: header("x-ratelimit-reset"),
            resource: header("x-ratelimit-resource"),
        };
        self.watch.observe(&signals);
        let limit_end = signals.block_end(now).map(|(until, _)| until);
        let body = response.text().await.unwrap_or_default();
        let fail = |error: PlatformError| {
            log_failure(
                account,
                Some((&endpoint, auth)),
                Some(status),
                &excerpt(body.as_bytes()),
                &error,
            );
            error
        };
        if !(200..300).contains(&status) {
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|b| b.get("message")?.as_str().map(str::to_owned))
                .unwrap_or_default();
            return Err(fail(from_status(account, status, &message, limit_end, now)));
        }
        let me = serde_json::from_str::<ConnectionData>(&body)
            .ok()
            .and_then(|data| data.authenticated_user)
            .map(|user| Me {
                id: user.id,
                display_name: user.custom_display_name.or(user.provider_display_name),
                email_address: user
                    .properties
                    .and_then(|p| p.pointer("/Account/$value")?.as_str().map(str::to_owned)),
            })
            .ok_or_else(|| {
                fail(PlatformError::Rejected(format!(
                    "{account} refused the token: it answered with something but a user"
                )))
            })?;
        Ok(me)
    }
}

/// Who the signed-in user is, as far as the organization's `connectionData` says.
#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
pub(crate) struct Me {
    pub id: Option<String>,
    pub display_name: Option<String>,
    pub email_address: Option<String>,
}

#[derive(Deserialize)]
struct ConnectionData {
    #[serde(rename = "authenticatedUser")]
    authenticated_user: Option<ConnectedUser>,
}

#[derive(Deserialize)]
struct ConnectedUser {
    id: Option<String>,
    #[serde(rename = "providerDisplayName")]
    provider_display_name: Option<String>,
    #[serde(rename = "customDisplayName")]
    custom_display_name: Option<String>,
    /// The sign-in address is in `Account.$value`.
    properties: Option<serde_json::Value>,
}

fn url(endpoint: &str) -> Result<Url, PlatformError> {
    Url::parse(endpoint).map_err(|_| PlatformError::Unexpected(format!("{endpoint} is not an address")))
}

/// A request that failed on the network or with a server error is tried again twice. A
/// request the platform asks to slow down is not: its `Retry-After` can be minutes, and
/// whoever called is told when to come back instead of waiting for it.
#[derive(Debug)]
struct Retries;

#[async_trait::async_trait]
impl RetryPolicy for Retries {
    fn is_expired(&self, _since_start: Duration, retries: u32) -> bool {
        retries >= 2
    }

    fn retry_headers(&self) -> Option<&RetryHeaders> {
        None
    }

    fn retry_status_codes(&self) -> &[StatusCode] {
        &[
            StatusCode::RequestTimeout,
            StatusCode::InternalServerError,
            StatusCode::BadGateway,
            StatusCode::ServiceUnavailable,
            StatusCode::GatewayTimeout,
        ]
    }

    fn sleep_duration(&self, retries: u32) -> Duration {
        Duration::milliseconds(500 << retries.saturating_sub(1))
    }
}

fn retries() -> RetryOptions {
    RetryOptions::custom(Arc::new(Retries))
}

/// The method, host, path and query of a request, which name what it asked for. The
/// credential never is in the address.
fn endpoint(method: Method, url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
    match url.port() {
        Some(port) => format!("{method} {host}:{port}{}{query}", url.path()),
        None => format!("{method} {host}{}{query}", url.path()),
    }
}

/// How the credential goes with a request: the name of the scheme, never the value.
fn scheme(source: CredentialSource) -> &'static str {
    match source {
        CredentialSource::Pat => "Basic",
        CredentialSource::Cli => "Bearer",
    }
}

/// The first part of a response body, for the log.
fn excerpt(body: &[u8]) -> String {
    String::from_utf8_lossy(body)
        .chars()
        .take(BODY_EXCERPT)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// The request an error came from, carried by the error to whoever makes sense of it.
#[derive(Debug)]
struct FailedRequest {
    endpoint: String,
    auth: &'static str,
}

impl std::fmt::Display for FailedRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.endpoint)
    }
}

impl std::error::Error for FailedRequest {}

/// Sends a request to the path the API names, turns an answer that is not a success into an
/// error once the retries are done, and has every error say which request it was.
#[derive(Debug)]
struct Requests {
    auth: &'static str,
}

#[async_trait::async_trait]
impl Policy for Requests {
    async fn send(&self, ctx: &Context, request: &mut Request, next: &[Arc<dyn Policy>]) -> PolicyResult {
        // The API crates join the endpoint, whose address ends in a slash, and the path with
        // another one.
        if let Some(path) = request.url().path().strip_prefix('/').filter(|p| p.starts_with('/')) {
            let path = path.to_owned();
            request.url_mut().set_path(&path);
        }
        let endpoint = endpoint(request.method(), request.url());
        let answer = match next[0].send(ctx, request, &next[1..]).await {
            Ok(response) => check_success(response, None).await,
            failed => failed,
        };
        answer.map_err(|error| {
            if blocked_until(&error).is_some() {
                return error;
            }
            let message = error.to_string();
            let failed = FailedRequest {
                endpoint,
                auth: self.auth,
            };
            azure_core::Error::with_error(error.kind().clone(), failed, message)
        })
    }
}

fn requests(source: CredentialSource) -> Vec<Arc<dyn Policy>> {
    vec![Arc::new(Requests { auth: scheme(source) })]
}

/// Why Azure DevOps refused a request. Every message is fit to show. `host` is the account's.
pub(crate) fn failure(host: &str, error: &azure_core::Error, now: i64) -> PlatformError {
    // The limit was logged when it began.
    if let Some(until) = blocked_until(error) {
        return rate_limited(host, until, now);
    }
    let mut body = String::new();
    let (status, failed) = match error.kind() {
        ErrorKind::HttpResponse {
            status, raw_response, ..
        } => {
            let status = u16::from(*status);
            if let Some(raw) = raw_response {
                body = excerpt(raw.body().as_ref());
            }
            let message = raw_response
                .as_ref()
                .and_then(|r| r.body().json::<serde_json::Value>().ok())
                .and_then(|b| b.get("message")?.as_str().map(str::to_owned))
                .unwrap_or_default();
            let header = |name: &'static str| {
                raw_response
                    .as_ref()
                    .and_then(|r| r.headers().get_optional_str(&HeaderName::from_static(name)))
            };
            let limit_end = Signals {
                status,
                retry_after: header("retry-after"),
                remaining: header("x-ratelimit-remaining"),
                reset: header("x-ratelimit-reset"),
                resource: None,
            }
            .block_end(now)
            .map(|(until, _)| until);
            (Some(status), from_status(host, status, &message, limit_end, now))
        }
        ErrorKind::Io | ErrorKind::Connection => (None, PlatformError::Unreachable(format!("cannot reach {host}"))),
        ErrorKind::DataConversion => (
            None,
            PlatformError::Unexpected(format!("{host} sent a document that Vet cannot read")),
        ),
        _ => (None, PlatformError::Unexpected(format!("{host} could not be asked"))),
    };
    let request = error.downcast_ref::<FailedRequest>();
    log_failure(
        host,
        request.map(|r| (r.endpoint.as_str(), r.auth)),
        status,
        &body,
        &failed,
    );
    failed
}

/// What an HTTP status, and the message the platform gave with it, mean for the caller.
/// `limit_end` is when the limit its signals announce ends.
fn from_status(host: &str, status: u16, message: &str, limit_end: Option<i64>, now: i64) -> PlatformError {
    let reason = |what: &str| {
        if message.is_empty() {
            format!("{host} {what} ({status})")
        } else {
            format!("{host} {what}: {message} ({status})")
        }
    };
    match status {
        // A redirect goes to the sign-in page.
        300..=399 | 401 => PlatformError::Rejected(reason("refused the token")),
        403 => PlatformError::Forbidden(reason("does not allow the token this")),
        404 => PlatformError::NotFound(reason("does not know this, or the token cannot see it")),
        429 => rate_limited(host, limit_end.unwrap_or(now), now),
        500..=599 => PlatformError::Unreachable(format!("{host} answered {status}")),
        _ => PlatformError::Unexpected(reason("answered")),
    }
}

/// When the limit ends that kept the request from being sent, if that is why `error` is one. The
/// libraries wrap errors on their way out, so the chain is searched.
fn blocked_until(error: &(dyn std::error::Error + 'static)) -> Option<i64> {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(Blocked { until }) = error.downcast_ref() {
            return Some(*until);
        }
        current = error.source();
    }
    None
}

/// The limit on `host` ends at `until`.
fn rate_limited(host: &str, until: i64, now: i64) -> PlatformError {
    PlatformError::RateLimited {
        message: format!("{host} asks to slow down; try again in {}", in_words(until - now)),
        retry_at: Some(until),
    }
}

/// Records a failed request: the method, address and the scheme of its credential (never the
/// credential), the status and the start of the answer.
pub(crate) fn log_failure(
    account: &str,
    request: Option<(&str, &str)>,
    status: Option<u16>,
    body: &str,
    error: &PlatformError,
) {
    let (endpoint, auth) = request.unwrap_or(("-", "-"));
    tracing::warn!(
        account,
        endpoint,
        auth,
        status = status.unwrap_or(0),
        body,
        reason = %error,
        "Azure DevOps request failed"
    );
}

fn in_words(seconds: i64) -> String {
    match u64::try_from(seconds).unwrap_or(0).div_ceil(60).max(1) {
        1 => "a minute".to_owned(),
        minutes => format!("{minutes} minutes"),
    }
}
