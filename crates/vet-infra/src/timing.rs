//! With `VET_TIMING=1`, how long requests, language-server starts and
//! language-server queries take, one stderr line each.

use std::sync::OnceLock;
use std::time::Duration;

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("VET_TIMING").is_some_and(|v| v == "1"))
}

/// Prints `what took N ms` when timing is enabled.
pub fn log(what: impl FnOnce() -> String, took: Duration) {
    if enabled() {
        eprintln!("vet timing: {} took {} ms", what(), took.as_millis());
    }
}
