//! The crate's integration tests, one binary: every file under `tests/` would otherwise
//! link the whole dependency tree on its own.

mod acceptance;
mod caches;
mod cancel;
mod checks;
mod common;
mod fetch;
mod first_mark;
mod github;
mod load;
mod progress;
mod python;
mod recent;
mod restore;
mod semantic;
mod session;
