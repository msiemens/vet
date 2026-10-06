//! Use cases (SPEC §6, §5, §9, §11) over ports. Modules call each other directly,
//! so ordering guarantees are plain awaits: execution reconciles before it
//! launches, and settling reconciles before it declares a snapshot settled.

pub mod acquisition;
pub mod execution;
pub mod ignore;
pub mod limits;
pub mod pin;
pub mod ports;
pub mod review;
pub mod semantics;
pub mod servers;
pub mod session;
pub mod settle;
pub mod sync;
