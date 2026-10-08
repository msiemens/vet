//! The semantic layer (SPEC §7): a server pool, queries, and derived analyses.
//!
//! It owns the traits it consumes (`Materializer`, `ServerLauncher`,
//! `LanguageServer`); `vet-app` and `vet-infra` implement them. It never sees
//! how a directory was produced (I1) and reuses an answer only under a key that
//! names everything it depends on (I2, `reuse`). It has no async runtime: IO
//! happens behind the ports.

pub mod derived;
pub mod join;
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
