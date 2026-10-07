//! Hosting platforms: which platform a host is and the credentials used against it.

mod accounts;
mod cli;
mod credentials;
mod host;

pub use accounts::{Account, AccountError, AccountState, Accounts};
pub use credentials::{Credential, CredentialSource, CredentialValidator, Token, ValidationError};
pub use host::{PlatformKind, normalize_host};
