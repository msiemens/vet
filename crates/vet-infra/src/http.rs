//! The HTTP client the platform adapters share.

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
