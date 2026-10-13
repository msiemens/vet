//! `vet_app::platform::CredentialValidator`: asks GitHub or Azure DevOps whom a token
//! belongs to. The token goes in a request header to the host it is for and nowhere
//! else; a refusal is logged with its host and the platform's reason, never a token, a
//! request or a response.

use std::time::Duration;

use reqwest::StatusCode;
use vet_app::platform::{Credential, CredentialValidator, Owner, PlatformError, PlatformKind, ValidationError};

use crate::clock::unix_now;
use crate::http::LazyClient;
use crate::platform::azure_devops::access::{API_ENDPOINT, Access};
use crate::platform::time::parse_token_expiry;
use crate::platform::{github, message};

const TIMEOUT: Duration = Duration::from_secs(10);

pub struct HttpValidator {
    http: LazyClient,
    /// Where the APIs are, if not where the host says.
    #[cfg(test)]
    endpoint: Option<String>,
}

impl Default for HttpValidator {
    fn default() -> Self {
        Self {
            http: LazyClient::new(TIMEOUT),
            #[cfg(test)]
            endpoint: None,
        }
    }
}

impl HttpValidator {
    fn http(&self) -> Result<&reqwest::Client, ValidationError> {
        self.http.get().map_err(ValidationError::Unreachable)
    }

    fn endpoint(&self, default: impl FnOnce() -> String) -> String {
        #[cfg(test)]
        if let Some(endpoint) = &self.endpoint {
            return endpoint.clone();
        }
        default()
    }

    /// The user, and the token's end from the header GitHub adds for a token that expires.
    async fn github_user(&self, host: &str, credential: &Credential) -> Result<Owner, ValidationError> {
        let base = self.endpoint(|| github::Endpoints::of(host).rest);
        let unusable = |_| ValidationError::Unreachable(format!("cannot set up the client for {host}"));
        let crab = github::checking_client(self.http()?, &base, credential, host).map_err(unusable)?;
        let answer = match crab._get("/user").await {
            Ok(response) => octocrab::map_github_error(response).await,
            Err(e) => Err(e),
        };
        match answer {
            Ok(response) => {
                let expires_at = response
                    .headers()
                    .get("github-authentication-token-expiration")
                    .and_then(|v| v.to_str().ok())
                    .and_then(parse_token_expiry);
                let body = crab
                    .body_to_string(response)
                    .await
                    .map_err(|_| ValidationError::Unreachable(format!("{host} answered in a way Vet cannot read")))?;
                let login = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|user| Some(user.get("login")?.as_str()?.to_owned()))
                    .ok_or_else(|| ValidationError::Unreachable(format!("{host} answered without a user")))?;
                Ok(Owner {
                    user: login,
                    expires_at,
                })
            }
            Err(octocrab::Error::GitHub { source, .. }) => {
                let status = source.status_code;
                // A token the host does not know is refused, a host that is not GitHub is
                // not found.
                let refused = status.is_redirection() || (status.is_client_error() && status != StatusCode::NOT_FOUND);
                Err(if refused {
                    refusal(host, status, &source.message)
                } else {
                    ValidationError::Unreachable(format!("{host} answered {status}"))
                })
            }
            Err(_) => Err(ValidationError::Unreachable(format!("cannot reach {host}"))),
        }
    }

    /// Azure DevOps tells a token nothing of its own end: only an Entra token may list them.
    async fn azure_devops_user(&self, host: &str, credential: &Credential) -> Result<Owner, ValidationError> {
        let Some(organization) = host.strip_prefix("dev.azure.com/") else {
            return Err(ValidationError::Rejected(format!(
                "{host} is not the account of an Azure DevOps organization, e.g. dev.azure.com/contoso"
            )));
        };
        let endpoint = self.endpoint(|| API_ENDPOINT.to_owned());
        let me = Access::unshared(credential, self.http()?, host)
            .me(&endpoint, organization, host, unix_now())
            .await
            .map_err(|e| match e {
                PlatformError::Rejected(why) | PlatformError::Forbidden(why) => ValidationError::Rejected(why),
                other => ValidationError::Unreachable(other.to_string()),
            })?;
        me.display_name
            .or(me.email_address)
            .map(Owner::named)
            .ok_or_else(|| ValidationError::Rejected(format!("{host} refused the token: the profile has no name")))
    }
}

impl CredentialValidator for HttpValidator {
    async fn user(&self, host: &str, kind: PlatformKind, credential: &Credential) -> Result<Owner, ValidationError> {
        let answer = match kind {
            PlatformKind::GitHub => self.github_user(host, credential).await,
            PlatformKind::AzureDevOps => self.azure_devops_user(host, credential).await,
        };
        if let Err(error) = &answer {
            let (ValidationError::Rejected(why) | ValidationError::Unreachable(why)) = error;
            tracing::warn!(host, ?kind, reason = %why, "a credential could not be validated");
        }
        answer
    }
}

fn refusal(host: &str, status: StatusCode, reason: &str) -> ValidationError {
    ValidationError::Rejected(message::reason(host, "refused the token", reason, status))
}

#[cfg(test)]
mod tests {
    use vet_app::platform::{CredentialSource, Token};

    use super::*;
    use crate::platform::fake::{Canned, Fake};

    fn credential(source: CredentialSource) -> Credential {
        Credential {
            token: Token::new("secret").unwrap(),
            source,
        }
    }

    fn validator(fake: &Fake) -> HttpValidator {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = crate::http::client_builder(TIMEOUT).no_proxy().build().unwrap();
        HttpValidator {
            http: LazyClient::ready(TIMEOUT, http),
            endpoint: Some(fake.base()),
        }
    }

    /// A user as GitHub documents one: `octocrab` needs every URL.
    fn github_user_json(login: &str) -> String {
        let url = format!("https://api.github.com/users/{login}");
        format!(
            r#"{{"login":"{login}","id":1,"node_id":"U_1","avatar_url":"{url}","gravatar_id":"","url":"{url}",
"html_url":"{url}","followers_url":"{url}","following_url":"{url}","gists_url":"{url}","starred_url":"{url}",
"subscriptions_url":"{url}","organizations_url":"{url}","repos_url":"{url}","events_url":"{url}",
"received_events_url":"{url}","type":"User","site_admin":false,"name":null,"patch_url":null}}"#
        )
    }

    #[tokio::test]
    async fn the_github_user_is_its_login_and_the_token_goes_as_a_bearer() {
        let fake = Fake::start(|_, _| Canned::json(&github_user_json("octocat")));
        let owner = validator(&fake)
            .user("github.com", PlatformKind::GitHub, &credential(CredentialSource::Pat))
            .await
            .unwrap();
        assert_eq!(owner, Owner::named("octocat"));
        let requests = fake.requests();
        assert_eq!(requests[0].path, "/user");
        assert_eq!(requests[0].header("authorization"), Some("Bearer secret"));
    }

    #[tokio::test]
    async fn a_github_token_that_expires_says_when_in_utc_or_with_an_offset() {
        for (header, expected) in [
            ("2026-11-01 12:00:00 UTC", 1_793_534_400),
            ("2024-02-29 10:30:00 +0200", 1_709_195_400),
            ("2024-02-29 01:30:00 -0700", 1_709_195_400),
        ] {
            let fake = Fake::start(move |_, _| {
                Canned::json(&github_user_json("octocat")).with("GitHub-Authentication-Token-Expiration", header)
            });
            let owner = validator(&fake)
                .user("github.com", PlatformKind::GitHub, &credential(CredentialSource::Pat))
                .await
                .unwrap();
            assert_eq!(owner.expires_at, Some(expected), "{header}");
        }
    }

    #[tokio::test]
    async fn a_refused_github_token_carries_the_platforms_reason_and_a_missing_api_is_unreachable() {
        let fake = Fake::start(|_, _| Canned::status(401, r#"{"message":"Bad credentials"}"#));
        let error = validator(&fake)
            .user("github.com", PlatformKind::GitHub, &credential(CredentialSource::Pat))
            .await
            .unwrap_err();
        assert_eq!(
            error,
            ValidationError::Rejected("github.com refused the token: Bad credentials (401 Unauthorized)".to_owned())
        );

        let fake = Fake::start(|_, _| Canned::status(404, r#"{"message":"Not Found"}"#));
        let error = validator(&fake)
            .user(
                "ghe.corp.example",
                PlatformKind::GitHub,
                &credential(CredentialSource::Pat),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, ValidationError::Unreachable(_)), "{error:?}");
    }

    #[tokio::test]
    async fn a_refused_token_is_logged_with_its_host_and_status_and_without_the_token() {
        let capture = vet_log::capture::Capture::default();
        let _logging = capture.install(vet_log::filter(None).0);
        let fake = Fake::start(|_, _| Canned::status(401, r#"{"message":"Bad credentials"}"#));
        let github = validator(&fake)
            .user("github.com", PlatformKind::GitHub, &credential(CredentialSource::Pat))
            .await;
        assert!(github.is_err());
        let fake = Fake::start(|_, _| Canned::status(401, ""));
        let azure = validator(&fake)
            .user(
                "dev.azure.com/contoso",
                PlatformKind::AzureDevOps,
                &credential(CredentialSource::Pat),
            )
            .await;
        assert!(azure.is_err());

        let log = capture.text();
        assert!(
            log.contains("WARN") && log.contains("github.com") && log.contains("401"),
            "{log}"
        );
        assert!(log.contains("dev.azure.com/contoso"), "{log}");
        // The token itself, and its base64 as the Basic header carries it.
        assert!(!log.contains("secret") && !log.contains("OnNlY3JldA"), "{log}");
    }

    #[tokio::test]
    async fn the_azure_devops_user_is_asked_of_the_organization_and_a_personal_token_has_no_user_name() {
        // A token of one organization is refused by the global profile, which is not asked.
        let fake = Fake::start(|seen, _| {
            if seen.path == "/contoso/_apis/connectionData" {
                Canned::json(
                    r#"{"authenticatedUser":{"id":"a1","providerDisplayName":"Ada Lovelace",
"properties":{"Account":{"$type":"System.String","$value":"ada@example.com"}}}}"#,
                )
            } else {
                Canned::status(401, r#"{"message":"VS30063: You are not authorized"}"#)
            }
        });
        let user = validator(&fake)
            .user(
                "dev.azure.com/contoso",
                PlatformKind::AzureDevOps,
                &credential(CredentialSource::Pat),
            )
            .await
            .unwrap();
        assert_eq!(user, Owner::named("Ada Lovelace"));
        let request = &fake.requests()[0];
        assert_eq!(request.path, "/contoso/_apis/connectionData");
        // base64 of ":secret".
        assert_eq!(request.header("authorization"), Some("Basic OnNlY3JldA=="));

        let user = validator(&fake)
            .user(
                "dev.azure.com/contoso",
                PlatformKind::AzureDevOps,
                &credential(CredentialSource::Cli),
            )
            .await
            .unwrap();
        assert_eq!(user, Owner::named("Ada Lovelace"));
        assert_eq!(fake.requests()[1].header("authorization"), Some("Bearer secret"));
    }

    #[tokio::test]
    async fn azure_devops_refuses_by_status_by_redirect_and_by_its_sign_in_page() {
        for (status, body) in [
            (401, ""),
            (302, ""),
            // A token it does not know is answered with the sign-in page and a success status.
            (203, "<html>Sign in</html>"),
        ] {
            let fake = Fake::start(move |_, _| {
                Canned::status(status, body).with("Location", "https://spsprodweu1.vssps.visualstudio.com/_signin")
            });
            let error = validator(&fake)
                .user(
                    "dev.azure.com/contoso",
                    PlatformKind::AzureDevOps,
                    &credential(CredentialSource::Pat),
                )
                .await
                .unwrap_err();
            assert!(matches!(error, ValidationError::Rejected(_)), "{status}: {error:?}");
        }
    }

    #[tokio::test]
    async fn a_refused_azure_devops_token_carries_the_platforms_reason() {
        let fake = Fake::start(|_, _| Canned::status(401, r#"{"message":"TF400813: The user is not authorized"}"#));
        let error = validator(&fake)
            .user(
                "dev.azure.com/contoso",
                PlatformKind::AzureDevOps,
                &credential(CredentialSource::Pat),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error,
            ValidationError::Rejected(
                "dev.azure.com/contoso refused the token: TF400813: The user is not authorized (401)".to_owned()
            )
        );
    }

    #[tokio::test]
    async fn an_azure_devops_account_without_an_organization_is_not_asked() {
        let fake = Fake::start(|_, _| Canned::status(200, "{}"));
        let error = validator(&fake)
            .user(
                "dev.azure.com",
                PlatformKind::AzureDevOps,
                &credential(CredentialSource::Pat),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, ValidationError::Rejected(_)), "{error:?}");
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn an_azure_devops_server_error_is_not_a_refusal() {
        let fake = Fake::start(|_, _| Canned::status(500, ""));
        let error = validator(&fake)
            .user(
                "dev.azure.com/contoso",
                PlatformKind::AzureDevOps,
                &credential(CredentialSource::Pat),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, ValidationError::Unreachable(_)), "{error:?}");
    }
}
