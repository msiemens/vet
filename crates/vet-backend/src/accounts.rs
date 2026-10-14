//! Requests about the credentials for hosting platforms, which belong to the process
//! and not to a session.

use vet_app::changeset::Source;
use vet_app::fetch::Credentials;
use vet_app::platform::{Credential, normalize_host};
use vet_infra::clock::unix_now;
use vet_mapping as mapping;
use vet_protocol::{Event, Response, Secret};

use crate::error::Result;
use crate::events::Connection;
use crate::sync::sync_in_background;
use crate::{Orchestrator, PlatformAccounts};

/// The accounts the process holds, as the credentials a fetch uses.
pub(crate) struct Held<'a>(pub(crate) &'a PlatformAccounts);

impl Credentials for Held<'_> {
    async fn credential(&self, host: &str) -> Option<Credential> {
        self.0.credential(host, unix_now()).await
    }
}

impl Orchestrator {
    /// The accounts, for a connection that from now on hears the events of the process.
    pub(crate) async fn accounts(&self, from: &Connection) -> Result<Response> {
        self.process_subscribers.subscribe(from.clone());
        let accounts = self.accounts.accounts(unix_now()).await;
        Ok(Response::Accounts {
            accounts: mapping::account::accounts(&accounts),
        })
    }

    /// A token handed back unchecked that is held already changes nothing, and is not announced:
    /// the shell hands back what its keychain holds whenever a platform refuses it.
    pub(crate) async fn set_credentials(&self, host: &str, token: &Secret, validate: bool) -> Result<Response> {
        let unchanged = !validate && self.accounts.holds(host, &token.0);
        let account = self
            .accounts
            .set_credentials(host, &token.0, validate, unix_now())
            .await?;
        if !unchanged {
            self.accounts_changed(&account.host).await;
        }
        Ok(Response::Account(mapping::account::account(&account)))
    }

    pub(crate) async fn remove_credentials(&self, host: &str) -> Result<Response> {
        let accounts = self.accounts.remove_credentials(host, unix_now()).await?;
        if let Ok(host) = normalize_host(host) {
            self.accounts_changed(&host).await;
        }
        Ok(Response::Accounts {
            accounts: mapping::account::accounts(&accounts),
        })
    }

    /// The credential of `host` changed: the connections of the process are told, and the open
    /// pull requests of the host are synced again with it.
    async fn accounts_changed(&self, host: &str) {
        self.process_subscribers.broadcast(Event::AccountsChanged);
        for open in self.projects.all().await {
            let Some(booted) = open.booted_now() else {
                continue;
            };
            if let Source::PullRequest { remote, .. } = booted.session.source().await
                && remote.account() == host
            {
                sync_in_background(open.clone());
            }
        }
    }
}
