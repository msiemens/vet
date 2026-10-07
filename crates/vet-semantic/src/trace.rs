//! Where the semantic layer reports how long its phases took, for a host that
//! asks (`VET_TIMING`). Nothing is measured for a host that does not install a sink.
//! The individual queries are timed by the language-server layer, which knows
//! what a query waited for.

use std::sync::OnceLock;
use std::time::Instant;

/// Receives a finished phase: what it was and when it began.
pub type Sink = fn(String, Instant);

static SINK: OnceLock<Sink> = OnceLock::new();

/// The first sink installed stays.
pub fn install(sink: Sink) {
    let _ = SINK.set(sink);
}

/// Reports `what` as having taken the time since `started`. `what` is built
/// only if there is a sink.
pub fn log(what: impl FnOnce() -> String, started: Instant) {
    if let Some(sink) = SINK.get() {
        sink(what(), started);
    }
}
