//! Path to `FileEntry` cache keyed by file stat, so reconciliation (§6.2) hashes
//! only entries whose stat changed. In memory for now.

use std::collections::HashMap;
use std::fs::Metadata;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use vet_core::content::{FileEntry, RepoPath};

use crate::clock::unix_secs;

/// What identifies a file's content without reading it. `index_mode` is part of
/// it because the recorded mode depends on the index when the filesystem cannot
/// represent it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct StatKey {
    size: u64,
    mtime: (i64, u32),
    ctime: (i64, u32),
    ino: u64,
    index_mode: Option<u32>,
}

impl StatKey {
    pub(super) fn of(md: &Metadata, index_mode: Option<u32>) -> Self {
        #[cfg(unix)]
        let (ctime, ino) = { ((md.ctime(), md.ctime_nsec() as u32), md.ino()) };
        #[cfg(not(unix))]
        let (ctime, ino) = ((0, 0), 0);
        Self {
            size: md.len(),
            mtime: md.modified().map_or((0, 0), stamp),
            ctime,
            ino,
            index_mode,
        }
    }
}

fn stamp(t: SystemTime) -> (i64, u32) {
    (
        unix_secs(t),
        t.duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()),
    )
}

#[derive(Clone)]
pub(crate) struct Record {
    pub key: StatKey,
    pub entry: FileEntry,
    pub unavailable: bool,
    /// When the content was read.
    pub recorded_secs: i64,
}

impl Record {
    /// Racy like git's index: a file modified in the same second the content was
    /// read could keep an identical stat, so its mtime must be older.
    pub(super) fn matches(&self, key: &StatKey) -> bool {
        self.key == *key && key.mtime.0 < self.recorded_secs
    }
}

#[derive(Default)]
pub(crate) struct StatCache {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Stat of the files that change how content is converted.
    config: Vec<Option<(u64, (i64, u32))>>,
    records: HashMap<RepoPath, Record>,
}

impl StatCache {
    /// Take the records for a scan. They are dropped if the git configuration
    /// changed since they were made.
    pub(super) fn take(&self, config: Vec<Option<(u64, (i64, u32))>>) -> HashMap<RepoPath, Record> {
        let mut state = self.state.lock().expect("not poisoned");
        if state.config != config {
            state.records.clear();
            state.config = config;
        }
        std::mem::take(&mut state.records)
    }

    /// Replace the records with those of the finished scan, which also drops
    /// paths that no longer exist.
    pub(super) fn put(&self, records: HashMap<RepoPath, Record>) {
        self.state.lock().expect("not poisoned").records = records;
    }
}

/// Stat of one config file, for the cache's invalidation.
pub(crate) fn file_stamp(path: &std::path::Path) -> Option<(u64, (i64, u32))> {
    let md = std::fs::metadata(path).ok()?;
    Some((md.len(), md.modified().map_or((0, 0), stamp)))
}

#[cfg(test)]
mod tests;
