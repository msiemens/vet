//! The semantic layer (SPEC §6): a server pool, queries, and derived analyses.
//!
//! It owns the traits it consumes (`Materializer`, `ServerLauncher`,
//! `LanguageServer`); `vet-app` and `vet-infra` implement them. It never sees
//! how a directory was produced (`ARCHITECTURE.md`) and reuses an answer only under a key that
//! names everything it depends on (I1, `reuse`). It has no async runtime: IO
//! happens behind the ports.

// Tests build their fixtures with locks of their own.
#![cfg_attr(test, allow(clippy::disallowed_methods))]

pub mod derived;
pub mod gate;
pub mod join;
mod mutex;
pub mod policy;
pub mod pool;
pub mod ports;
pub mod query;
pub mod reuse;
pub mod signature;
pub mod symbols;
pub mod trace;

#[cfg(test)]
mod testutil;
