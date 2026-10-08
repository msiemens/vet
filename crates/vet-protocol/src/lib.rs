//! Shell <-> backend protocol (SPEC §4.1): one protocol over one loopback
//! WebSocket in every topology, JSON text frames.
//!
//! Ids and counts are `u32` so they map to plain TS `number`s.

mod account;
mod changeset;
mod checks;
mod discussion;
mod envelope;
mod event;
mod file;
mod lens;
mod progress;
mod project;
mod recent;
mod request;
mod response;
mod review;
mod semantic;
mod server;
mod text;
mod thread;
mod version;

pub use account::*;
pub use changeset::*;
pub use checks::*;
pub use discussion::*;
pub use envelope::*;
pub use event::*;
pub use file::*;
pub use lens::*;
pub use progress::*;
pub use project::*;
pub use recent::*;
pub use request::*;
pub use response::*;
pub use review::*;
pub use semantic::*;
pub use server::*;
pub use text::*;
pub use thread::*;
pub use version::*;
