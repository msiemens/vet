//! A credential for a hosting platform, and the port that asks the platform about it.

use std::fmt;

use super::host::PlatformKind;

/// A token. Its `Debug` shows nothing, so holding one in a logged structure is safe.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// `None` for a blank token.
    pub fn new(token: &str) -> Option<Self> {
        let token = token.trim();
        (!token.is_empty()).then(|| Self(token.to_owned()))
    }

    /// The token itself, for the one place that sends it to the platform.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// Where a credential comes from, which also says how the platform wants it presented.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CredentialSource {
    /// A signed-in `gh` or `az`: asked on demand, never stored.
    Cli,
    /// A personal access token the shell holds in the OS keychain.
    Pat,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Credential {
    pub token: Token,
    pub source: CredentialSource,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ValidationError {
    /// The platform answered that the credential is no good. Fit to show.
    Rejected(String),
    /// The platform could not be asked. Fit to show.
    Unreachable(String),
}

/// Asks a platform whom a credential belongs to; implemented over HTTP by `vet-infra`.
pub trait CredentialValidator {
    /// The platform's name for the credential's owner.
    fn user(
        &self,
        host: &str,
        kind: PlatformKind,
        credential: &Credential,
    ) -> impl Future<Output = Result<String, ValidationError>> + Send;
}
