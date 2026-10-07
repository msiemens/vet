//! What every call to Azure DevOps needs: the credential as the API crate wants it, the
//! transport that follows no redirect, and what to make of a failure.

use std::sync::Arc;

use azure_core::credentials::{AccessToken, Secret, TokenCredential, TokenRequestOptions};
use azure_core::error::ErrorKind;
use azure_core::http::headers::HeaderName;
use azure_core::http::{ExponentialRetryOptions, RetryOptions, Transport, Url};
use azure_core::time::{Duration, OffsetDateTime};
use azure_devops_rust_api::{Credential as ApiCredential, profile};
use serde::Deserialize;
use vet_app::platform::{Credential, CredentialSource, PlatformError};

/// Where the identity of a user across organizations is.
pub(crate) const PROFILE_ENDPOINT: &str = "https://app.vssps.visualstudio.com";

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
}

impl Access {
    /// A personal access token goes as the password of a user with no name, a CLI token
    /// as a bearer token.
    pub(crate) fn new(credential: &Credential, http: &reqwest::Client) -> Self {
        let token = credential.token.expose().to_owned();
        let credential = match credential.source {
            CredentialSource::Pat => ApiCredential::from_pat(token),
            CredentialSource::Cli => ApiCredential::from_token_credential(Arc::new(Bearer(Secret::new(token)))),
        };
        Self {
            credential,
            transport: Transport::new(Arc::new(http.clone())),
        }
    }

    fn profiles(&self, endpoint: &str) -> Result<profile::Client, PlatformError> {
        Ok(profile::ClientBuilder::new(self.credential.clone())
            .endpoint(url(endpoint)?)
            .transport(self.transport.clone())
            .retry(retries())
            .build())
    }

    /// The signed-in user. A host that answers with anything but a profile, its sign-in
    /// page for a token it does not know, has refused the token.
    pub(crate) async fn me(&self, endpoint: &str, host: &str, now: i64) -> Result<Me, PlatformError> {
        let reply = self
            .profiles(endpoint)?
            .profiles_client()
            .get("me")
            .details(true)
            .send()
            .await
            .map_err(|e| failure(host, &e, now))?;
        let me: Me = reply.into_raw_response().body().json().map_err(|_| {
            PlatformError::Rejected(format!(
                "{host} refused the token: it answered with something but a profile"
            ))
        })?;
        Ok(me)
    }
}

/// What the profile service knows of a user. The API crate's model has none of it.
#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
pub(crate) struct Me {
    pub id: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(rename = "emailAddress")]
    pub email_address: Option<String>,
}

fn url(endpoint: &str) -> Result<Url, PlatformError> {
    Url::parse(endpoint).map_err(|_| PlatformError::Unexpected(format!("{endpoint} is not an address")))
}

/// Failed requests are tried again twice, as the platform's `Retry-After` says.
fn retries() -> RetryOptions {
    RetryOptions::exponential(ExponentialRetryOptions {
        initial_delay: Duration::milliseconds(500),
        max_retries: 2,
        max_total_elapsed: Duration::seconds(30),
        max_delay: Duration::seconds(10),
    })
}

/// Why Azure DevOps refused a request. Every message is fit to show.
pub(crate) fn failure(host: &str, error: &azure_core::Error, now: i64) -> PlatformError {
    match error.kind() {
        ErrorKind::HttpResponse {
            status, raw_response, ..
        } => {
            let status = u16::from(*status);
            let message = raw_response
                .as_ref()
                .and_then(|r| r.body().json::<serde_json::Value>().ok())
                .and_then(|b| b.get("message")?.as_str().map(str::to_owned))
                .unwrap_or_default();
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
                429 => {
                    let wait = raw_response
                        .as_ref()
                        .and_then(|r| r.headers().get_optional_str(&HeaderName::from_static("retry-after")))
                        .and_then(|s| s.parse::<i64>().ok());
                    let retry_at = wait.map(|secs| now + secs);
                    let when = wait.map_or_else(
                        || "; try again in a minute".to_owned(),
                        |secs| format!("; try again in {}", in_words(secs)),
                    );
                    PlatformError::RateLimited {
                        message: format!("{host} asks to slow down{when}"),
                        retry_at,
                    }
                }
                500..=599 => PlatformError::Unreachable(format!("{host} answered {status}")),
                _ => PlatformError::Unexpected(reason("answered")),
            }
        }
        ErrorKind::Io | ErrorKind::Connection => PlatformError::Unreachable(format!("cannot reach {host}")),
        ErrorKind::DataConversion => PlatformError::Unexpected(format!("{host} sent a document that Vet cannot read")),
        _ => PlatformError::Unexpected(format!("{host} could not be asked")),
    }
}

fn in_words(seconds: i64) -> String {
    match u64::try_from(seconds).unwrap_or(0).div_ceil(60).max(1) {
        1 => "a minute".to_owned(),
        minutes => format!("{minutes} minutes"),
    }
}
