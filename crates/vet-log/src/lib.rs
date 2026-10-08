//! Vet's one log. The backend and the shell each install the subscriber once: a daily
//! rolling file in the `logs` folder of Vet's data directory, plus stderr in a debug build.
//! `VET_LOG` sets the level, in `RUST_LOG` syntax (`info` by default; `timing=debug,info` adds
//! how long requests, language-server starts and queries took).
//!
//! Nothing at `info` or above carries a token, a credential or the content of a file.

pub mod capture;

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The environment variable that sets the level.
pub const ENV: &str = "VET_LOG";

/// The level when `VET_LOG` says nothing.
pub const DEFAULT_FILTER: &str = "info";

/// The folder of the log files in the data directory.
pub const LOGS: &str = "logs";

/// The file of a day is `vet.log.<date>`.
const FILE: &str = "vet.log";
/// How many days stay.
const KEPT_FILES: usize = 5;

/// Vet's data directory (§8) in the platform's place for application data.
pub fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("vet"))
}

/// The first line of a tool's output or a server's message: what is safe to log above `debug`,
/// as the rest may quote code.
pub fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// `text` with the user name, password or token in front of the host of every URL in it replaced
/// by `***`: `https://x-access-token:ghp_1@github.com/o/r` reads `https://***@github.com/o/r`.
/// Apply it to a message that may quote a remote's URL before it is logged.
pub fn redact_urls(text: &str) -> Cow<'_, str> {
    const MASK: &str = "***";
    let mut out = String::new();
    let mut rest = text;
    let mut redacted = false;
    while let Some(at) = rest.find("://") {
        let (head, after) = rest.split_at(at + 3);
        out.push_str(head);
        let end = after
            .find(|c: char| c.is_whitespace() || matches!(c, '/' | '?' | '#' | '"' | '\'' | '`' | '<' | '>'))
            .unwrap_or(after.len());
        let (authority, tail) = after.split_at(end);
        match authority.rfind('@') {
            Some(at) => {
                out.push_str(MASK);
                out.push_str(&authority[at..]);
                redacted = true;
            }
            None => out.push_str(authority),
        }
        rest = tail;
    }
    if !redacted {
        return Cow::Borrowed(text);
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// The filter `spec` (the value of `VET_LOG`) asks for, and why it was not used if it is invalid.
/// No spec, or an empty one, is the default.
pub fn filter(spec: Option<&str>) -> (EnvFilter, Option<String>) {
    let default = || EnvFilter::new(DEFAULT_FILTER);
    match spec.map(str::trim).filter(|s| !s.is_empty()) {
        None => (default(), None),
        Some(spec) => match EnvFilter::try_new(spec) {
            Ok(filter) => (filter, None),
            Err(e) => (
                default(),
                Some(format!(
                    "{ENV}={spec:?} is not a log filter ({e}); using {DEFAULT_FILTER}"
                )),
            ),
        },
    }
}

/// Keeps the file writer's thread alive: dropping it flushes what is buffered. Hold it until the
/// process ends.
#[must_use = "dropping the guard stops the log file from being written"]
pub struct Guard {
    _worker: Option<WorkerGuard>,
}

/// Installs the process's subscriber: the file in `dir` and, in a debug build, stderr, at the
/// level `VET_LOG` asks for. A directory that cannot be written leaves stderr only (in a release
/// build, nothing). Panics are logged as well.
pub fn init(dir: &Path) -> Guard {
    let (filter, rejected) = filter(std::env::var(ENV).ok().as_deref());
    let (file, guard, unwritable) = match appender(dir) {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            (Some(writer), Some(guard), None)
        }
        Err(why) => (None, None, Some(why)),
    };
    let file = file.map(|writer| tracing_subscriber::fmt::layer().with_ansi(false).with_writer(writer));
    let stderr = cfg!(debug_assertions).then(|| tracing_subscriber::fmt::layer().with_writer(std::io::stderr));
    let installed = tracing_subscriber::registry()
        .with(filter)
        .with(file)
        .with(stderr)
        .try_init()
        .is_ok();
    if installed {
        if let Some(rejected) = rejected {
            tracing::warn!("{rejected}");
        }
        if let Some(why) = unwritable {
            tracing::warn!(dir = %dir.display(), "no log file: {why}");
        }
        log_panics();
    }
    Guard { _worker: guard }
}

fn appender(dir: &Path) -> Result<RollingFileAppender, String> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(FILE)
        .max_log_files(KEPT_FILES)
        .build(dir)
        .map_err(|e| e.to_string())
}

fn log_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(target: "panic", "{info}");
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emitted(spec: Option<&str>) -> String {
        let capture = capture::Capture::default();
        let _guard = capture.install(filter(spec).0);
        tracing::debug!(target: "vet_test", "a debug line");
        tracing::info!(target: "vet_test", "an info line");
        tracing::warn!(target: "other", "a warning");
        // Without the timestamps, which differ between runs.
        let text = capture.text();
        text.lines()
            .map(|line| line.split_once(' ').map_or(line, |(_, rest)| rest))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_credentials_of_a_url_are_masked_wherever_the_message_quotes_one() {
        let message = "fatal: unable to access 'https://x-access-token:ghp_secret@github.com/o/r.git/': \
                       could not resolve host, then ssh://deploy@host:22/r and https://example.com/a@b";
        assert_eq!(
            redact_urls(message),
            "fatal: unable to access 'https://***@github.com/o/r.git/': \
             could not resolve host, then ssh://***@host:22/r and https://example.com/a@b"
        );
    }

    #[test]
    fn a_message_without_credentials_is_left_alone() {
        for text in [
            "",
            "no url here",
            "https://github.com/o/r",
            "user@host:path.git",
            "a :// b",
        ] {
            assert!(matches!(redact_urls(text), Cow::Borrowed(t) if t == text), "{text}");
        }
    }

    #[test]
    fn a_password_with_an_at_sign_is_masked_to_the_last_one() {
        assert_eq!(redact_urls("https://u:p@ss@host/x"), "https://***@host/x");
    }

    #[test]
    fn without_a_setting_info_and_above_is_logged() {
        let text = emitted(None);
        assert!(text.contains("an info line") && text.contains("a warning"), "{text}");
        assert!(!text.contains("a debug line"), "{text}");
        assert_eq!(emitted(Some("  ")), text);
    }

    #[test]
    fn the_setting_overrides_the_default_per_target() {
        let text = emitted(Some("vet_test=debug,error"));
        assert!(text.contains("a debug line") && text.contains("an info line"), "{text}");
        assert!(!text.contains("a warning"), "{text}");
        let quiet = emitted(Some("error"));
        assert!(quiet.is_empty(), "{quiet}");
    }

    #[test]
    fn a_setting_that_is_no_filter_gives_the_default_and_says_so() {
        let (_, rejected) = filter(Some("vet_test=loud"));
        assert!(rejected.is_some_and(|why| why.contains("loud")));
        assert_eq!(emitted(Some("vet_test=loud")), emitted(None));
    }
}
