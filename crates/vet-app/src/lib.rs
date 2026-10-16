//! Use cases (SPEC §6, §5, §11) over ports. Modules call each other directly,
//! so ordering guarantees are plain awaits: settling reconciles before it
//! declares a snapshot settled.

// Tests build their fixtures with locks of their own.
#![cfg_attr(test, allow(clippy::disallowed_methods))]

pub mod acquisition;
pub mod analysis;
pub mod base;
pub mod changeset;
pub mod editor;
pub mod fetch;
pub mod gc;
pub mod ignore;
mod mutex;
pub mod platform;
pub mod ports;
pub mod progress;
pub mod pull_request;
pub mod pull_request_feed;
pub mod recent;
pub mod registry;
pub mod review;
pub mod revisions;
pub mod search;
pub mod semantics;
pub mod servers;
pub mod session;
pub mod settle;
pub mod trust;

/// The contract of the store ports (T4): run against the fake from here and against
/// `SqliteStore` from `vet-infra`'s tests, which enable the `contract` feature for it.
#[cfg(any(test, feature = "contract"))]
pub mod store_contract;
