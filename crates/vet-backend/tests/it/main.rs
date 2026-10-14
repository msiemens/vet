//! The crate's integration tests, one binary: every file under `tests/` would otherwise
//! link the whole dependency tree on its own.

mod acceptance;
mod caches;
mod cancel;
mod checks;
mod common;
mod fetch;
mod github;
mod load;
mod progress;
mod python;
mod recent;
mod restart;
mod restore;
mod review_screen;
mod search;
mod semantic;
mod session;
mod trust;
