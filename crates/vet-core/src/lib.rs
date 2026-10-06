//! Pure domain model: content-addressed snapshots and the review model (I1, I3).
//!
//! Nothing here knows about git refs, commit hashes, the filesystem, or time.

pub mod content;
pub mod platform;
pub mod review;
