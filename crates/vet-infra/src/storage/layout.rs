//! Where a project's files are in the data directory (§9):
//!
//! ```text
//! <data>/projects/<pk>/
//!   root       the repository root, as text
//!   sources/   the project's kept source trees
//! ```
//!
//! `pk` is the first [`PK_HEX`] hex digits of the hash of the root, so a project's
//! directory is found from its root alone.

use std::path::{Path, PathBuf};

use vet_core::content::Hash;

pub const PROJECTS: &str = "projects";
pub const SOURCES: &str = "sources";
pub const ROOT: &str = "root";

/// Hex digits of the root's hash in a project's directory name.
pub const PK_HEX: usize = 16;

/// The directory of the project whose repository root is `root`.
pub fn project_dir(data: &Path, root: &str) -> PathBuf {
    let hex = Hash::of(root.as_bytes()).to_hex();
    data.join(PROJECTS).join(&hex[..PK_HEX])
}
