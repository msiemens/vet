//! GitHub's rate limits, as its responses announce them
//! (<https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api>).
//!
//! Every response carries `x-ratelimit-remaining` and `x-ratelimit-reset` (UTC epoch seconds). A
//! used-up bucket (remaining 0) answers nothing until the reset. A secondary limit refuses with 403
//! or 429 and says how long to wait in `retry-after` (seconds); without that header, and without a
//! used-up bucket, GitHub asks for at least a minute. Sending requests meanwhile risks a ban, so
//! once a limit is used up no request to that host and bucket is sent until it ends: the transport
//! fails it at once with [`Blocked`], and the limit is logged once for its window.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};

use http::{HeaderMap, Request, Response, StatusCode};
use octocrab::OctoBody;
use tower::{BoxError, Layer, Service};

use super::Bucket;

/// How long a secondary limit lasts when GitHub does not say: "at least one minute".
pub(super) const SECONDARY_WAIT: i64 = 60;

/// Until when each host's bucket is used up.
#[derive(Default)]
pub(super) struct Limits {
    until: Mutex<HashMap<(String, Bucket), i64>>,
}

impl Limits {
    /// When the bucket's limit ends, if it is used up at `now`.
    pub(super) fn blocked_until(&self, host: &str, bucket: Bucket, now: i64) -> Option<i64> {
        let until = self.until.lock().unwrap_or_else(PoisonError::into_inner);
        until.get(&(host.to_owned(), bucket)).copied().filter(|&at| at > now)
    }

    /// The bucket is used up until `until`. The first report of a limit is logged, the ones for the
    /// same window are not.
    pub(super) fn block(&self, host: &str, bucket: Bucket, until: i64, now: i64, why: &str) {
        let mut windows = self.until.lock().unwrap_or_else(PoisonError::into_inner);
        let current = windows.entry((host.to_owned(), bucket)).or_insert(0);
        let begins = *current <= now;
        *current = (*current).max(until);
        if begins {
            tracing::warn!(
                host,
                bucket = ?bucket,
                resets_in_seconds = until - now,
                why,
                "GitHub's rate limit is used up; nothing is sent to it until it resets"
            );
        }
    }
}

/// A request that was not sent: the bucket it counts against is used up until `until`.
#[derive(Debug)]
pub(super) struct Blocked {
    pub(super) until: i64,
}

impl fmt::Display for Blocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the rate limit is used up until {}", self.until)
    }
}

impl std::error::Error for Blocked {}

/// What the responses of one client (one host's REST or GraphQL API) say of its bucket, and a gate
/// that keeps requests back while it is used up.
#[derive(Clone)]
pub(super) struct Watch {
    pub(super) limits: Arc<Limits>,
    pub(super) host: String,
    pub(super) bucket: Bucket,
    pub(super) now: fn() -> i64,
}

impl Watch {
    fn observe(&self, status: StatusCode, headers: &HeaderMap) {
        let now = (self.now)();
        let number = |name: &str| headers.get(name)?.to_str().ok()?.trim().parse::<i64>().ok();
        let refused = status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS;
        let used_up = number("x-ratelimit-remaining") == Some(0);
        let reset = number("x-ratelimit-reset").filter(|_| used_up);
        let wait = number("retry-after").filter(|_| refused).map(|secs| now + secs.max(0));
        let unsaid = (status == StatusCode::TOO_MANY_REQUESTS).then_some(now + SECONDARY_WAIT);
        let (until, why) = match (wait, reset) {
            (Some(wait), reset) if reset.is_none_or(|r| wait >= r) => (wait, "retry-after"),
            (_, Some(reset)) => (reset, "x-ratelimit-reset"),
            _ => match unsaid {
                Some(until) => (until, "a secondary limit without a time"),
                None => return,
            },
        };
        if until > now {
            self.limits.block(&self.host, self.bucket, until, now, why);
        }
    }
}

/// Wraps a transport in the [`Watch`].
pub(super) struct WatchLayer(pub(super) Watch);

impl<S> Layer<S> for WatchLayer {
    type Service = Watching<S>;

    fn layer(&self, inner: S) -> Watching<S> {
        Watching {
            inner,
            watch: self.0.clone(),
        }
    }
}

#[derive(Clone)]
pub(super) struct Watching<S> {
    inner: S,
    watch: Watch,
}

impl<S, B> Service<Request<OctoBody>> for Watching<S>
where
    S: Service<Request<OctoBody>, Response = Response<B>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Into<BoxError>,
    B: Send + 'static,
{
    type Response = Response<B>;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Response<B>, BoxError>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), BoxError>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, request: Request<OctoBody>) -> Self::Future {
        let watch = self.watch.clone();
        if let Some(until) = watch.limits.blocked_until(&watch.host, watch.bucket, (watch.now)()) {
            return Box::pin(async move { Err(Blocked { until }.into()) });
        }
        // The clone is the service that was polled ready; the original answers this call.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        Box::pin(async move {
            let response = inner.call(request).await.map_err(Into::into)?;
            watch.observe(response.status(), response.headers());
            Ok(response)
        })
    }
}
