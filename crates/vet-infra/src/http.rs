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

/// The TLS provider that `reqwest` and `gix` need. Already installed by something else is as good
/// as installing it now.
pub(crate) fn install_tls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub(crate) fn http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    install_tls_provider();
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

    /// A client that is already set up and ignores the system's proxy settings, for a fake server.
    #[cfg(test)]
    pub(crate) fn for_tests(timeout: Duration) -> Self {
        install_tls_provider();
        Self {
            timeout,
            client: OnceLock::from(client_builder(timeout).no_proxy().build().unwrap()),
        }
    }
}
