//! The words the platform adapters use for what went wrong.

use std::fmt::Display;

use vet_app::platform::PlatformError;

/// `host` did `what`, as `message` (if the platform gave one) says: `github.com refused the token:
/// Bad credentials (401)`.
pub(crate) fn reason(host: &str, what: &str, message: &str, status: impl Display) -> String {
    if message.is_empty() {
        format!("{host} {what} ({status})")
    } else {
        format!("{host} {what}: {message} ({status})")
    }
}

/// The limit on `host` ends at `until`.
pub(crate) fn rate_limited(host: &str, until: i64, now: i64) -> PlatformError {
    PlatformError::RateLimited {
        message: format!("{host}'s rate limit is used up; it resets in {}", in_words(until - now)),
        retry_at: Some(until),
    }
}

/// A span of seconds rounded up to whole minutes, or hours from two on.
fn in_words(seconds: i64) -> String {
    let minutes = u64::try_from(seconds).unwrap_or(0).div_ceil(60).max(1);
    match minutes {
        1 => "a minute".to_owned(),
        2..=119 => format!("{minutes} minutes"),
        _ => format!("{} hours", minutes.div_ceil(60)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_round_up_to_a_minute_then_to_minutes_then_to_hours() {
        assert_eq!(in_words(-5), "a minute");
        assert_eq!(in_words(1), "a minute");
        assert_eq!(in_words(61), "2 minutes");
        assert_eq!(in_words(119 * 60), "119 minutes");
        assert_eq!(in_words(120 * 60), "2 hours");
        assert_eq!(in_words(121 * 60), "3 hours");
    }

    #[test]
    fn a_reason_names_the_platforms_message_only_when_there_is_one() {
        assert_eq!(reason("h", "refused", "", 401), "h refused (401)");
        assert_eq!(reason("h", "refused", "no", 401), "h refused: no (401)");
    }
}
