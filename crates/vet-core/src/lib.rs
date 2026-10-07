//! Pure domain model: content-addressed snapshots and the review model (I1, I3).
//!
//! Nothing here knows about git refs, commit hashes, the filesystem, or time.

pub mod classify;
pub mod completion;
pub mod content;
pub mod filter;
pub mod hunks;
pub mod inline;
pub mod interdiff;
pub mod marking;
pub mod merge;
pub mod merkle;
pub mod patch_id;
pub mod prepared;
pub mod remap;
pub mod resolve;
pub mod review;
pub mod text;
pub mod version_diff;
