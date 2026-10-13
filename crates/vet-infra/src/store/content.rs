//! Content-addressed files: written to a temp file, fsynced, renamed into place, and the
//! directory fsynced. Many blobs are put as a batch: the directories are synced once each,
//! after every blob of the batch was renamed into them.
//!
//! On Apple systems `fsync` hands the data to the drive without flushing the drive's cache,
//! and `F_FULLFSYNC` flushes that whole cache. So each file and directory gets a plain
//! `fsync`, and a batch ends with one `F_FULLFSYNC` ([`ContentStore::durable`]) before its
//! blobs count as held. Elsewhere `fsync` (`sync_all`) is durable by itself.
//!
//! A blob read back is checked against its hash: one that does not match (a write torn by a
//! crash the scheme above did not cover, a disk error) is removed and reported missing, so
//! the reader falls back to git or the worktree and the next batch writes it again.

use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use vet_core::content::{BlobId, Hash};

use crate::sync::lock;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) struct ContentStore {
    root: PathBuf,
    /// Blobs renamed into place that are not durable yet. A crash could still lose them, so
    /// they do not count as held.
    unsynced: Mutex<HashSet<BlobId>>,
    /// How many times the drive's cache was flushed.
    #[cfg(test)]
    pub full_flushes: AtomicUsize,
}

/// Blobs renamed into place by [`ContentStore::place`]. They are durable once `dirs` are
/// synced and [`ContentStore::durable`] was told.
#[derive(Default)]
pub(super) struct Placed {
    /// The id of every blob given, in order.
    pub ids: Vec<BlobId>,
    /// The blobs that were written; the others were held already.
    written: Vec<BlobId>,
    /// The directories the written blobs were renamed into.
    pub dirs: BTreeSet<PathBuf>,
}

impl Placed {
    /// One batch of what `self` and `other` placed.
    pub(super) fn merge(mut self, other: Placed) -> Self {
        self.ids.extend(other.ids);
        self.written.extend(other.written);
        self.dirs.extend(other.dirs);
        self
    }
}

impl ContentStore {
    pub(super) fn open(root: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(root.join("tmp"))?;
        Ok(Self {
            root,
            unsynced: Mutex::default(),
            #[cfg(test)]
            full_flushes: AtomicUsize::new(0),
        })
    }

    fn path(&self, id: BlobId) -> PathBuf {
        let hex = id.0.to_hex();
        self.root.join(&hex[..2]).join(&hex[2..])
    }

    fn unsynced(&self) -> std::sync::MutexGuard<'_, HashSet<BlobId>> {
        lock(&self.unsynced)
    }

    /// Whether `id` is in place and survives a crash.
    fn holds(&self, id: BlobId) -> bool {
        self.path(id).exists() && !self.unsynced().contains(&id)
    }

    /// Of `ids`, those the store does not hold durably.
    pub(super) fn missing(&self, ids: &[BlobId]) -> Vec<BlobId> {
        ids.iter().copied().filter(|id| !self.holds(*id)).collect()
    }

    /// Writes each of `blobs` the store does not hold to a temp file, fsyncs it and renames it
    /// into place. The directories are left to sync: until [`ContentStore::durable`] is told
    /// of them, the written blobs are [`missing`](ContentStore::missing).
    pub(super) fn place(&self, blobs: &[Vec<u8>]) -> io::Result<Placed> {
        self.place_with(blobs, |_| {})
    }

    /// `durable_tmp` runs after each temp file is fsynced and before it is renamed.
    fn place_with(&self, blobs: &[Vec<u8>], mut durable_tmp: impl FnMut(&Path)) -> io::Result<Placed> {
        let mut placed = Placed::default();
        for bytes in blobs {
            let id = BlobId::of(bytes);
            placed.ids.push(id);
            if self.holds(id) || placed.written.contains(&id) {
                continue;
            }
            let tmp = self.root.join("tmp").join(format!(
                "{}-{}",
                std::process::id(),
                TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file = File::create(&tmp)?;
            file.write_all(bytes)?;
            sync_file(&file)?;
            drop(file);
            durable_tmp(&tmp);
            let dest = self.path(id);
            let dir = dest.parent().expect("blob path has a parent");
            fs::create_dir_all(dir)?;
            self.unsynced().insert(id);
            fs::rename(&tmp, &dest)?;
            placed.written.push(id);
            placed.dirs.insert(dir.to_owned());
        }
        Ok(placed)
    }

    /// Called once the directories of `placed` are synced: makes what the batch wrote reach
    /// the drive's storage, then counts its blobs as held.
    pub(super) fn durable(&self, placed: &Placed) -> io::Result<()> {
        if placed.written.is_empty() {
            return Ok(());
        }
        self.flush()?;
        let mut unsynced = self.unsynced();
        for id in &placed.written {
            unsynced.remove(id);
        }
        Ok(())
    }

    /// Flushes the drive's cache, with everything synced before it.
    #[cfg(target_vendor = "apple")]
    fn flush(&self) -> io::Result<()> {
        rustix::fs::fcntl_fullfsync(File::open(&self.root)?)?;
        #[cfg(test)]
        self.full_flushes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    #[cfg(not(target_vendor = "apple"))]
    fn flush(&self) -> io::Result<()> {
        Ok(())
    }

    /// The blob `id`; `None` if it is not in place or its content does not hash to `id`,
    /// which is removed so that the next batch writes it again.
    pub(super) fn get(&self, id: BlobId) -> io::Result<Option<Vec<u8>>> {
        let path = self.path(id);
        match fs::read(&path) {
            Ok(bytes) if BlobId::of(&bytes) == id => Ok(Some(bytes)),
            Ok(_) => {
                tracing::warn!(blob = %id.0.to_hex(), "a stored blob does not match its hash; removing it");
                match fs::remove_file(&path) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                    _ => Ok(None),
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Delete files not in `live` that are older than `grace`, including
    /// abandoned temp files. Returns the number of blobs deleted.
    pub(super) fn sweep(&self, live: &HashSet<BlobId>, now: SystemTime, grace: Duration) -> io::Result<usize> {
        let old = |path: &Path| -> io::Result<bool> {
            let modified = fs::metadata(path)?.modified()?;
            Ok(now.duration_since(modified).unwrap_or_default() >= grace)
        };
        for entry in fs::read_dir(self.root.join("tmp"))? {
            let path = entry?.path();
            if old(&path)? {
                fs::remove_file(path)?;
            }
        }
        let mut deleted = 0;
        for dir in fs::read_dir(&self.root)? {
            let dir = dir?.path();
            let Some(prefix) = dir.file_name().and_then(|n| n.to_str()).filter(|n| n.len() == 2) else {
                continue;
            };
            for file in fs::read_dir(&dir)? {
                let path = file?.path();
                let Some(id) = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|rest| Hash::from_hex(&format!("{prefix}{rest}")))
                    .map(BlobId)
                else {
                    continue;
                };
                if !live.contains(&id) && old(&path)? {
                    fs::remove_file(path)?;
                    deleted += 1;
                }
            }
        }
        Ok(deleted)
    }
}

/// Makes `file`'s content survive a crash; on Apple systems once the drive's cache is
/// flushed too.
#[cfg(target_vendor = "apple")]
fn sync_file(file: &File) -> io::Result<()> {
    Ok(rustix::fs::fsync(file)?)
}

#[cfg(not(target_vendor = "apple"))]
fn sync_file(file: &File) -> io::Result<()> {
    file.sync_all()
}

/// Makes the entries of `dir` survive a crash; on Apple systems once the drive's cache is
/// flushed too.
#[cfg(unix)]
pub(super) fn sync_dir(dir: &Path) -> io::Result<()> {
    sync_file(&File::open(dir)?)
}

// Directories cannot be opened for fsync on Windows; NTFS journals the rename.
#[cfg(not(unix))]
pub fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Puts `blobs` as a batch, one directory sync after another.
    fn put(store: &ContentStore, blobs: &[&[u8]]) -> Vec<BlobId> {
        let placed = store
            .place(&blobs.iter().map(|b| b.to_vec()).collect::<Vec<_>>())
            .unwrap();
        for dir in &placed.dirs {
            sync_dir(dir).unwrap();
        }
        store.durable(&placed).unwrap();
        placed.ids
    }

    #[test]
    fn a_blob_that_does_not_match_its_hash_is_missing_and_written_again() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(dir.path().to_path_buf()).unwrap();
        let id = put(&store, &[b"whole"])[0];
        fs::write(store.path(id), b"who").unwrap();

        assert_eq!(store.get(id).unwrap(), None);
        assert_eq!(store.missing(&[id]), [id]);
        put(&store, &[b"whole"]);
        assert_eq!(store.get(id).unwrap().as_deref(), Some(&b"whole"[..]));
    }

    #[test]
    fn a_batch_writes_every_blob_and_skips_those_held_already() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(dir.path().to_path_buf()).unwrap();
        put(&store, &[b"one"]);

        let placed = store
            .place(&[b"one".to_vec(), b"two".to_vec(), b"three".to_vec(), b"two".to_vec()])
            .unwrap();
        assert_eq!(
            placed.ids,
            [b"one".as_slice(), b"two", b"three", b"two"].map(BlobId::of).to_vec()
        );
        assert_eq!(placed.written, [b"two".as_slice(), b"three"].map(BlobId::of).to_vec());
        store.durable(&placed).unwrap();
        for blob in [b"one".as_slice(), b"two", b"three"] {
            assert_eq!(store.get(BlobId::of(blob)).unwrap().as_deref(), Some(blob));
        }
        assert_eq!(fs::read_dir(dir.path().join("tmp")).unwrap().count(), 0);
    }

    #[test]
    fn a_blob_is_durable_in_tmp_before_it_is_visible_and_held_only_once_its_directory_is_synced() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(dir.path().to_path_buf()).unwrap();
        let id = BlobId::of(b"hello");
        let placed = store
            .place_with(&[b"hello".to_vec()], |tmp| {
                assert_eq!(fs::read(tmp).unwrap(), b"hello");
                assert_eq!(store.get(id).unwrap(), None);
            })
            .unwrap();
        assert_eq!(store.get(id).unwrap().as_deref(), Some(&b"hello"[..]));
        assert_eq!(store.missing(&[id]), [id], "renamed, but its directory is not synced");
        // A batch that comes meanwhile writes it again rather than rely on it.
        assert_eq!(store.place(&[b"hello".to_vec()]).unwrap().written, [id]);

        store.durable(&placed).unwrap();
        assert_eq!(store.missing(&[id]), []);
        assert!(store.place(&[b"hello".to_vec()]).unwrap().written.is_empty());
    }

    #[test]
    fn sweep_keeps_live_and_young_and_removes_abandoned_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(dir.path().to_path_buf()).unwrap();
        let [live, dead] = [b"live".as_slice(), b"dead"].map(BlobId::of);
        put(&store, &[b"live", b"dead"]);
        fs::write(dir.path().join("tmp").join("crashed"), b"partial").unwrap();
        let live_set = HashSet::from([live]);

        let young = store
            .sweep(&live_set, SystemTime::now(), Duration::from_secs(3600))
            .unwrap();
        assert_eq!(young, 0);
        assert!(store.get(dead).unwrap().is_some());

        let deleted = store.sweep(&live_set, SystemTime::now(), Duration::ZERO).unwrap();
        assert_eq!(deleted, 1);
        assert!(store.get(live).unwrap().is_some());
        assert_eq!(store.get(dead).unwrap(), None);
        assert_eq!(fs::read_dir(dir.path().join("tmp")).unwrap().count(), 0);
    }
}
