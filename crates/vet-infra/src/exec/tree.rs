//! A run's whole process tree, so that Stop and the backend exiting end the
//! grandchildren too (a `pnpm dev` is a shell around node around a server).
//!
//! `prepare` before the spawn, `Tree::attach` right after. Dropping the `Tree`
//! kills whatever is left.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::{Tree, prepare};
#[cfg(windows)]
pub use windows::{Tree, prepare};
