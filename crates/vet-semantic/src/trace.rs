//! How long things took (requests, language-server starts and queries, restores,
//! materializations), logged at `debug` under the target `timing`: `VET_LOG=timing=debug,info`.
//! The individual queries are timed by the language-server layer, which knows what a query
//! waited for.

use std::time::Instant;

use tracing::Level;

/// Logs `what` as having taken the time since `started`. `what` is built only if the target is
/// at `debug`.
pub fn log(what: impl FnOnce() -> String, started: Instant) {
    if tracing::enabled!(target: "timing", Level::DEBUG) {
        let took_ms = started.elapsed().as_millis() as u64;
        tracing::debug!(target: "timing", took_ms, "{}", what());
    }
}
