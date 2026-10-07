//! Requests about the credentials for hosting platforms, which belong to the process
//! and not to a session.

use vet_infra::clock::unix_now;
use vet_mapping as mapping;
use vet_protocol::{Response, Secret};

use crate::events::Connection;
use crate::{Orchestrator, message};

impl Orchestrator {
    /// The accounts, for a connection that from now on hears when a credential is needed.
    pub(crate) async fn accounts(&self, from: &Connection) -> Result<Response, String> {
        self.process_subscribers.subscribe(from.clone());
        let accounts = self.accounts.accounts(unix_now()).await;
        Ok(Response::Accounts {
            accounts: mapping::account::accounts(&accounts),
        })
    }

    pub(crate) async fn set_credentials(&self, host: &str, token: &Secret, validate: bool) -> Result<Response, String> {
        let account = self
            .accounts
            .set_credentials(host, &token.0, validate, unix_now())
            .await
            .map_err(message)?;
        Ok(Response::Account(mapping::account::account(&account)))
    }

    pub(crate) async fn remove_credentials(&self, host: &str) -> Result<Response, String> {
        let accounts = self
            .accounts
            .remove_credentials(host, unix_now())
            .await
            .map_err(message)?;
        Ok(Response::Accounts {
            accounts: mapping::account::accounts(&accounts),
        })
    }
}
