//! With `VET_TIMING=1`, how long requests, language-server starts and
//! language-server queries take and when each began, one stderr line each.

use std::sync::OnceLock;
use std::time::Instant;

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("VET_TIMING").is_some_and(|v| v == "1"))
}

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// Marks the start of the process, which the times in the log are relative to.
/// Called first thing in `main`.
pub fn process_started() {
    PROCESS_START.get_or_init(Instant::now);
}

/// Prints `what took N ms (at +T s)` when timing is enabled: `started` is when
/// the timed thing began, `T` how long after the process started.
pub fn log(what: impl FnOnce() -> String, started: Instant) {
    if enabled() {
        let at = started.saturating_duration_since(*PROCESS_START.get_or_init(Instant::now));
        eprintln!(
            "vet timing: {} took {} ms (at +{:.1} s)",
            what(),
            started.elapsed().as_millis(),
            at.as_secs_f64()
        );
    }
}
