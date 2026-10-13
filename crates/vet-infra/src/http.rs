//! The HTTP client the platform adapters share.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::{ClientBuilder, redirect};

/// A client with Vet's user agent that follows no redirect: a request carries a token,
/// and a platform that answers with a redirect (a renamed repository, a sign-in page)
/// is told to the caller instead of followed.
pub(crate) fn client_builder(timeout: Duration) -> ClientBuilder {
    reqwest::Client::builder()
        .user_agent(concat!("vet/", env!("CARGO_PKG_VERSION")))
        .timeout(timeout)
        .redirect(redirect::Policy::none())
}

pub(crate) fn http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    // Already installed by something else is as good as installing it now.
    let _ = rustls::crypto::ring::default_provider().install_default();
    client_builder(timeout)
        .build()
        .map_err(|e| format!("cannot set up the HTTP client: {e}"))
}

/// An adapter's client, set up when first needed: loading the system's certificates is not free,
/// and may fail. A failed setup is tried again by the next request.
pub(crate) struct LazyClient {
    timeout: Duration,
    client: OnceLock<reqwest::Client>,
}

impl LazyClient {
    pub(crate) fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            client: OnceLock::new(),
        }
    }

    pub(crate) fn get(&self) -> Result<&reqwest::Client, String> {
        if let Some(client) = self.client.get() {
            return Ok(client);
        }
        let client = http_client(self.timeout)?;
        Ok(self.client.get_or_init(|| client))
    }

    #[cfg(test)]
    pub(crate) fn ready(timeout: Duration, client: reqwest::Client) -> Self {
        Self {
            timeout,
            client: OnceLock::from(client),
        }
    }
}
