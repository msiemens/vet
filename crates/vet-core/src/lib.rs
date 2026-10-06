//! Pure domain model: content-addressed snapshots and the review model (I1, I3).
//!
//! Nothing here knows about git refs, commit hashes, the filesystem, or time.

pub mod completion;
pub mod content;
pub mod filter;
pub mod inline;
pub mod marking;
pub mod merkle;
pub mod platform;
pub mod prepared;
pub mod remap;
pub mod resolve;
pub mod review;
mod text;
