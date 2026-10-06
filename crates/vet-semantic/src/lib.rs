//! The semantic layer (SPEC §7): a server pool, queries, and derived analyses.
//!
//! It owns the traits it consumes (`Materializer`, `ServerLauncher`,
//! `LanguageServer`); `vet-app` and `vet-infra` implement them. It never sees
//! how a directory was produced (I1) and never persists an answer (I2).

pub mod derived;
pub mod policy;
pub mod pool;
pub mod ports;
