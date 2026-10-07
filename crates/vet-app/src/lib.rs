//! Use cases (SPEC §6, §5, §11) over ports. Modules call each other directly,
//! so ordering guarantees are plain awaits: settling reconciles before it
//! declares a snapshot settled.

pub mod acquisition;
pub mod base;
pub mod changeset;
pub mod editor;
pub mod gc;
pub mod ignore;
pub mod limits;
pub mod platform;
pub mod ports;
pub mod progress;
pub mod recent;
pub mod registry;
pub mod review;
pub mod semantics;
pub mod servers;
pub mod session;
pub mod settle;
pub mod trust;
