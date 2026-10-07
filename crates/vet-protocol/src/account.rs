//! Accounts on hosting platforms: per host and app-wide, not per session.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ts_rs::TS;

/// A token or password. It is sent as a plain string; its `Debug` shows nothing, so a
/// request or event that carries one cannot leak it into a log.
#[derive(Clone, PartialEq, Eq, TS)]
#[ts(export, type = "string")]
pub struct Secret(pub String);

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PlatformKind {
    #[serde(rename = "github")]
    GitHub,
    AzureDevOps,
}

/// Where the backend gets a host's credential from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CredentialSource {
    /// A signed-in `gh` or `az` command line tool, asked on demand: the token is never stored.
    Cli,
    /// A personal access token the shell keeps in the OS keychain.
    Pat,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum AccountState {
    /// The platform accepted the credential.
    SignedIn,
    /// The platform refused the credential.
    Rejected {
        /// Fit to show.
        message: String,
    },
    /// The platform could not be asked, so the credential is neither confirmed nor refused.
    Unverified {
        /// Fit to show.
        message: String,
    },
}

/// A credential the backend holds for a host.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Account {
    /// The platform's host name, e.g. `github.com` or `dev.azure.com`.
    pub host: String,
    pub kind: PlatformKind,
    pub source: CredentialSource,
    /// The platform's name for the credential's owner, once the platform has been asked.
    pub user: Option<String>,
    pub state: AccountState,
    /// This is the credential the backend uses for the host: a host's CLI credential
    /// comes before its personal access token.
    pub in_use: bool,
}

#[cfg(test)]
mod tests {
    use crate::{Request, Secret};

    #[test]
    fn a_request_that_carries_a_token_does_not_show_it() {
        let request = Request::SetCredentials {
            host: "github.com".into(),
            token: Secret("ghp_supersecret".into()),
            validate: true,
        };
        let shown = format!("{request:?}");
        assert!(
            shown.contains("github.com") && !shown.contains("supersecret"),
            "{shown}"
        );
    }

    #[test]
    fn a_token_travels_as_a_plain_string() {
        let json = r#"{"type":"setCredentials","host":"github.com","token":"ghp_x","validate":false}"#;
        let request: Request = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&request).unwrap(), json);
    }
}
