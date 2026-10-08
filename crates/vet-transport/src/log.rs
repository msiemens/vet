//! What the transport logs: a span per request, and the entries the UI sends for the log.

use std::fmt::{self, Write};
use std::time::{Duration, Instant};

use tracing::Level;
use vet_protocol::{LogLevel, Request};

/// The longest message and context of a UI entry that is kept, in characters.
const MAX_MESSAGE: usize = 4000;
const MAX_CONTEXT: usize = 200;

/// At most this many UI entries per window are logged on a connection; the UI limits itself
/// well below, so this only stops a UI that does not.
const BUDGET: u32 = 100;
const WINDOW: Duration = Duration::from_secs(10);

/// The variant name of a request (`Hover`), which is all that is safe to log of one: a request
/// carries comments, paths and tokens. Formatting stops after the name.
pub(crate) fn kind(request: &Request) -> String {
    struct Name(String);
    impl Write for Name {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            match s.find(|c: char| !c.is_alphanumeric()) {
                Some(end) => {
                    self.0.push_str(&s[..end]);
                    Err(fmt::Error)
                }
                None => {
                    self.0.push_str(s);
                    Ok(())
                }
            }
        }
    }
    let mut name = Name(String::new());
    let _ = write!(name, "{request:?}");
    name.0
}

fn truncated(text: &str, max: usize) -> &str {
    text.char_indices().nth(max).map_or(text, |(end, _)| &text[..end])
}

/// Logs an entry of the UI under the target `ui`.
pub(crate) fn from_ui(level: LogLevel, message: &str, context: &str) {
    let (message, context) = (truncated(message, MAX_MESSAGE), truncated(context, MAX_CONTEXT));
    match level {
        LogLevel::Error => tracing::event!(target: "ui", Level::ERROR, %context, "{message}"),
        LogLevel::Warn => tracing::event!(target: "ui", Level::WARN, %context, "{message}"),
        LogLevel::Info => tracing::event!(target: "ui", Level::INFO, %context, "{message}"),
    }
}

/// How many UI entries a connection may still log in the window it is in.
pub(crate) struct UiBudget {
    since: Instant,
    used: u32,
    dropped: u32,
}

impl UiBudget {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            since: now,
            used: 0,
            dropped: 0,
        }
    }

    /// Whether the entry arriving at `now` is logged. A window that turned entries away says so
    /// when it ends.
    pub(crate) fn admit(&mut self, now: Instant) -> bool {
        if now.duration_since(self.since) >= WINDOW {
            if self.dropped > 0 {
                tracing::warn!(dropped = self.dropped, "the UI sent more log entries than are kept");
            }
            *self = Self::new(now);
        }
        if self.used < BUDGET {
            self.used += 1;
            true
        } else {
            self.dropped += 1;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_named_without_its_content() {
        let hover = Request::Hover {
            project: 1,
            revision: 2,
            side: vet_protocol::Side::Target,
            path: "secret/path.rs".into(),
            line: 0,
            col: 0,
        };
        assert_eq!(kind(&hover), "Hover");
        assert_eq!(kind(&Request::Accounts), "Accounts");
        let credentials = Request::SetCredentials {
            host: "github.com".into(),
            token: vet_protocol::Secret("hunter2".into()),
            validate: true,
        };
        assert_eq!(kind(&credentials), "SetCredentials");
    }

    #[test]
    fn a_ui_that_floods_the_log_is_cut_off_until_the_window_ends() {
        let start = Instant::now();
        let mut budget = UiBudget::new(start);
        let admitted = (0..BUDGET + 50).filter(|_| budget.admit(start)).count();
        assert_eq!(admitted, BUDGET as usize);
        assert!(!budget.admit(start + WINDOW / 2));
        assert!(budget.admit(start + WINDOW));
    }

    #[test]
    fn long_entries_are_cut_at_a_character() {
        assert_eq!(truncated("héllo", 2), "hé");
        assert_eq!(truncated("hi", 5), "hi");
    }
}
