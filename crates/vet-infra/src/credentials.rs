//! `vet_app::platform::CredentialValidator`: asks GitHub or Azure DevOps whom a token
//! belongs to. The token goes in a request header to the host it is for and nowhere
//! else; nothing here logs a request or a response.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::{RequestBuilder, StatusCode, redirect};
use vet_app::platform::{Credential, CredentialSource, CredentialValidator, PlatformKind, ValidationError};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Azure DevOps' identity service, which knows a user across organizations.
const AZURE_DEVOPS_IDENTITY: &str = "app.vssps.visualstudio.com";

#[derive(Default)]
pub struct HttpValidator {
    /// Set up when first needed: loading the system's certificates is not free, and may fail.
    client: OnceLock<Result<reqwest::Client, String>>,
}

/// One URL that names the owner of a token, and where in its answer the name is.
struct Endpoint {
    url: String,
    user: fn(&str) -> Option<String>,
}

impl HttpValidator {
    fn client(&self) -> Result<&reqwest::Client, ValidationError> {
        self.client
            .get_or_init(|| {
                // Already installed by something else is as good as installing it now.
                let _ = rustls::crypto::ring::default_provider().install_default();
                reqwest::Client::builder()
                    .user_agent(concat!("vet/", env!("CARGO_PKG_VERSION")))
                    .timeout(TIMEOUT)
                    // Azure DevOps sends a token it does not know to its sign-in page.
                    .redirect(redirect::Policy::none())
                    .build()
                    .map_err(|e| format!("cannot set up the HTTP client: {e}"))
            })
            .as_ref()
            .map_err(|why| ValidationError::Unreachable(why.clone()))
    }

    fn request(
        &self,
        endpoint: &Endpoint,
        kind: PlatformKind,
        credential: &Credential,
    ) -> Result<RequestBuilder, ValidationError> {
        let token = credential.token.expose();
        let request = self.client()?.get(&endpoint.url);
        Ok(match (kind, credential.source) {
            (PlatformKind::GitHub, _) => request
                .bearer_auth(token)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28"),
            // A personal access token is the password of a user with no name.
            (PlatformKind::AzureDevOps, CredentialSource::Pat) => request.basic_auth("", Some(token)),
            (PlatformKind::AzureDevOps, CredentialSource::Cli) => request.bearer_auth(token),
        })
    }

    async fn ask(
        &self,
        endpoint: &Endpoint,
        host: &str,
        kind: PlatformKind,
        credential: &Credential,
    ) -> Result<String, ValidationError> {
        let response = self
            .request(endpoint, kind, credential)?
            .send()
            .await
            .map_err(|e| ValidationError::Unreachable(format!("cannot reach {host}: {}", e.without_url())))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status == StatusCode::OK {
            return (endpoint.user)(&body).ok_or_else(|| refusal(kind, host, status, &body));
        }
        let refused = status.is_redirection() || (status.is_client_error() && status != StatusCode::NOT_FOUND);
        Err(if refused {
            refusal(kind, host, status, &body)
        } else {
            ValidationError::Unreachable(format!("{host} answered {status}"))
        })
    }
}

impl CredentialValidator for HttpValidator {
    async fn user(&self, host: &str, kind: PlatformKind, credential: &Credential) -> Result<String, ValidationError> {
        let mut last = None;
        for endpoint in endpoints(host, kind) {
            match self.ask(&endpoint, host, kind, credential).await {
                Ok(user) => return Ok(user),
                Err(error) => last = Some(error),
            }
        }
        Err(last.expect("every platform has an endpoint"))
    }
}

/// Where to ask, best first. A token may lack the scope of one endpoint and have the other's.
fn endpoints(host: &str, kind: PlatformKind) -> Vec<Endpoint> {
    match kind {
        PlatformKind::GitHub => vec![Endpoint {
            url: github_user_url(host),
            user: github_login,
        }],
        PlatformKind::AzureDevOps => {
            let mut endpoints = Vec::new();
            if host != "dev.azure.com" {
                // An organization's own address.
                endpoints.push(Endpoint {
                    url: format!("https://{host}/_apis/connectionData?api-version=7.1-preview"),
                    user: azure_devops_connection_user,
                });
            }
            endpoints.push(Endpoint {
                url: format!("https://{AZURE_DEVOPS_IDENTITY}/_apis/connectionData?api-version=7.1-preview"),
                user: azure_devops_connection_user,
            });
            endpoints.push(Endpoint {
                url: format!("https://{AZURE_DEVOPS_IDENTITY}/_apis/profile/profiles/me?api-version=7.1"),
                user: azure_devops_profile_user,
            });
            endpoints
        }
    }
}

fn github_user_url(host: &str) -> String {
    if host == "github.com" {
        "https://api.github.com/user".to_owned()
    } else {
        format!("https://{host}/api/v3/user")
    }
}

fn github_login(body: &str) -> Option<String> {
    let user: serde_json::Value = serde_json::from_str(body).ok()?;
    Some(user.get("login")?.as_str()?.to_owned())
}

/// The signed-in user of an Azure DevOps `connectionData`; an anonymous caller has none.
fn azure_devops_connection_user(body: &str) -> Option<String> {
    let data: serde_json::Value = serde_json::from_str(body).ok()?;
    let user = data.get("authenticatedUser")?;
    let name = user
        .get("providerDisplayName")
        .or_else(|| user.get("customDisplayName"))?
        .as_str()?;
    (user.get("isActive").and_then(serde_json::Value::as_bool) != Some(false)).then(|| name.to_owned())
}

fn azure_devops_profile_user(body: &str) -> Option<String> {
    let profile: serde_json::Value = serde_json::from_str(body).ok()?;
    Some(profile.get("displayName")?.as_str()?.to_owned())
}

fn refusal(kind: PlatformKind, host: &str, status: StatusCode, body: &str) -> ValidationError {
    let reason = match kind {
        PlatformKind::GitHub => serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|b| b.get("message")?.as_str().map(str::to_owned)),
        PlatformKind::AzureDevOps => None,
    };
    ValidationError::Rejected(match reason {
        Some(reason) => format!("{host} refused the token: {reason} ({status})"),
        None => format!("{host} refused the token ({status})"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_github_user_is_its_login() {
        let body = r#"{"login":"octocat","id":1,"type":"User","site_admin":false}"#;
        assert_eq!(github_login(body).as_deref(), Some("octocat"));
        assert_eq!(github_login(r#"{"message":"Bad credentials"}"#), None);
    }

    #[test]
    fn an_enterprise_host_is_asked_at_its_api_path() {
        assert_eq!(github_user_url("github.com"), "https://api.github.com/user");
        assert_eq!(
            github_user_url("ghe.corp.example"),
            "https://ghe.corp.example/api/v3/user"
        );
    }

    #[test]
    fn the_azure_devops_user_is_the_authenticated_one() {
        let signed_in = r#"{"authenticatedUser":{"id":"a1","providerDisplayName":"Ada Lovelace","isActive":true},"authorizedUser":{}}"#;
        assert_eq!(azure_devops_connection_user(signed_in).as_deref(), Some("Ada Lovelace"));
        assert_eq!(azure_devops_connection_user(r#"{"instanceId":"x"}"#), None);
        assert_eq!(azure_devops_connection_user("<html>Sign in</html>"), None);
        let profile = r#"{"displayName":"Ada Lovelace","publicAlias":"1f","emailAddress":"ada@example.com"}"#;
        assert_eq!(azure_devops_profile_user(profile).as_deref(), Some("Ada Lovelace"));
    }

    #[test]
    fn a_refusal_carries_the_platforms_reason() {
        let ValidationError::Rejected(why) = refusal(
            PlatformKind::GitHub,
            "github.com",
            StatusCode::UNAUTHORIZED,
            r#"{"message":"Bad credentials"}"#,
        ) else {
            panic!("not a rejection")
        };
        assert_eq!(why, "github.com refused the token: Bad credentials (401 Unauthorized)");
    }
}
