//! Azure DevOps' rate limits, as its responses announce them
//! (<https://learn.microsoft.com/en-us/azure/devops/integrate/concepts/rate-limits>).
//!
//! Before it delays a user's requests, a response carries `Retry-After` (seconds to wait),
//! `X-RateLimit-Remaining` (throughput units left before delays start, 0 once requests are delayed
//! or blocked) and `X-RateLimit-Reset` (UTC epoch seconds at which the tracked usage is back to
//! zero). `X-RateLimit-Resource` names the threshold for a human. A blocked request gets 429.
//! Delayed requests still succeed, so a count of zero alone holds nothing back: `Retry-After` does,
//! on any status, and a 429 also holds until `X-RateLimit-Reset` when the count is zero. Either
//! holds back every request to the account until it ends: the pipeline fails it at once with
//! [`Blocked`], and the limit is logged once for its window.

use std::sync::Arc;

use azure_core::error::ErrorKind;
use azure_core::http::headers::HeaderName;
use azure_core::http::policies::{Policy, PolicyResult};
use azure_core::http::{Context, Request};

use crate::platform::limits::{Blocked, Gate};

/// How long a 429 without a time lasts.
pub(super) const UNSAID_WAIT: i64 = 60;

/// What a response says of the limits.
#[derive(Default, Debug)]
pub(super) struct Signals<'a> {
    pub(super) status: u16,
    pub(super) retry_after: Option<&'a str>,
    pub(super) remaining: Option<&'a str>,
    pub(super) reset: Option<&'a str>,
    pub(super) resource: Option<&'a str>,
}

/// A number of seconds or an epoch, which may carry decimals; rounded up.
fn seconds(text: Option<&str>) -> Option<i64> {
    let value = text?.trim().parse::<f64>().ok().filter(|v| v.is_finite())?;
    #[allow(clippy::cast_possible_truncation)]
    Some(value.ceil() as i64)
}

/// What the responses of one account say of its limits, and a gate that keeps requests back while
/// they are used up.
#[derive(Clone)]
pub(super) struct Watch {
    pub(super) gate: Arc<Gate<String>>,
    pub(super) account: String,
    pub(super) now: fn() -> i64,
}

impl Watch {
    /// Why the request may not be sent, if the account's limit is used up.
    pub(super) fn check(&self) -> Result<(), Blocked> {
        match self.gate.blocked_until(&self.account, (self.now)()) {
            Some(until) => Err(Blocked { until }),
            None => Ok(()),
        }
    }

    pub(super) fn observe(&self, signals: &Signals<'_>) {
        let now = (self.now)();
        let wait = seconds(signals.retry_after).map(|secs| now + secs.max(0));
        let refused = signals.status == 429;
        let used_up = seconds(signals.remaining) == Some(0);
        let reset = seconds(signals.reset).filter(|_| refused && used_up);
        let unsaid = refused.then_some(now + UNSAID_WAIT);
        let (until, why) = match (wait, reset) {
            (Some(wait), reset) if reset.is_none_or(|r| wait >= r) => (wait, "Retry-After"),
            (_, Some(reset)) => (reset, "X-RateLimit-Reset"),
            _ => match unsaid {
                Some(until) => (until, "a 429 without a time"),
                None => return,
            },
        };
        if until > now && self.gate.block(&self.account, until, now) {
            tracing::warn!(
                account = %self.account,
                resource = signals.resource.unwrap_or("-"),
                resets_in_seconds = until - now,
                why,
                "Azure DevOps' rate limit is used up; nothing is sent to it until it resets"
            );
        }
    }
}

/// Holds requests back while the account's limit is used up, and reads each response, before a
/// retry, for what it says of the limit.
#[derive(Debug)]
pub(super) struct Gating(pub(super) Watch);

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watch").field("account", &self.account).finish()
    }
}

#[async_trait::async_trait]
impl Policy for Gating {
    async fn send(&self, ctx: &Context, request: &mut Request, next: &[Arc<dyn Policy>]) -> PolicyResult {
        if let Err(blocked) = self.0.check() {
            let message = blocked.to_string();
            return Err(azure_core::Error::with_error(ErrorKind::Other, blocked, message));
        }
        let response = next[0].send(ctx, request, &next[1..]).await?;
        let header = |name: &'static str| response.headers().get_optional_str(&HeaderName::from_static(name));
        self.0.observe(&Signals {
            status: u16::from(response.status()),
            retry_after: header("retry-after"),
            remaining: header("x-ratelimit-remaining"),
            reset: header("x-ratelimit-reset"),
            resource: header("x-ratelimit-resource"),
        });
        Ok(response)
    }
}
