//! Accounts as the protocol shows them.

use vet_app::platform::{Account, AccountState, CredentialSource, PlatformKind};
use vet_protocol as p;

pub fn account(account: &Account) -> p::Account {
    p::Account {
        host: account.host.clone(),
        kind: match account.kind {
            PlatformKind::GitHub => p::PlatformKind::GitHub,
            PlatformKind::AzureDevOps => p::PlatformKind::AzureDevOps,
        },
        source: match account.source {
            CredentialSource::Cli => p::CredentialSource::Cli,
            CredentialSource::Pat => p::CredentialSource::Pat,
        },
        user: account.user.clone(),
        state: match &account.state {
            AccountState::SignedIn => p::AccountState::SignedIn,
            AccountState::Rejected(message) => p::AccountState::Rejected {
                message: message.clone(),
            },
            AccountState::Unverified(message) => p::AccountState::Unverified {
                message: message.clone(),
            },
            AccountState::Unusable(message) => p::AccountState::Unusable {
                message: message.clone(),
            },
        },
        in_use: account.in_use,
        expires_at: account.expires_at.and_then(|at| u32::try_from(at).ok()),
    }
}

pub fn accounts(accounts: &[Account]) -> Vec<p::Account> {
    accounts.iter().map(account).collect()
}
