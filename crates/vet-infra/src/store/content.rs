//! Content-addressed files: written to a temp file, fsynced, renamed into place, and the
//! directory fsynced. Many blobs are put as a batch: the directories are synced once each,
//! after every blob of the batch was renamed into them.

use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use vet_core::content::{BlobId, Hash};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct ContentStore {
    root: PathBuf,
    /// Blobs renamed into place whose directory is not synced yet. A crash could still lose
    /// them, so they do not count as held.
    unsynced: Mutex<HashSet<BlobId>>,
}

/// Blobs renamed into place by [`ContentStore::place`]. They are durable once `dirs` are
/// synced and [`ContentStore::durable`] was told.
#[derive(Default)]
pub struct Placed {
    /// The id of every blob given, in order.
    pub ids: Vec<BlobId>,
    /// The blobs that were written; the others were held already.
    written: Vec<BlobId>,
    /// The directories the written blobs were renamed into.
    pub dirs: BTreeSet<PathBuf>,
}

impl Placed {
    /// One batch of what `self` and `other` placed.
    pub fn merge(mut self, other: Placed) -> Self {
        self.ids.extend(other.ids);
        self.written.extend(other.written);
        self.dirs.extend(other.dirs);
        self
    }
}

impl ContentStore {
    pub fn open(root: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(root.join("tmp"))?;
        Ok(Self {
            root,
            unsynced: Mutex::default(),
        })
    }

    fn path(&self, id: BlobId) -> PathBuf {
        let hex = id.0.to_hex();
        self.root.join(&hex[..2]).join(&hex[2..])
    }

    fn unsynced(&self) -> std::sync::MutexGuard<'_, HashSet<BlobId>> {
        self.unsynced.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether `id` is in place and survives a crash.
    fn holds(&self, id: BlobId) -> bool {
        self.path(id).exists() && !self.unsynced().contains(&id)
    }

    /// Of `ids`, those the store does not hold durably.
    pub fn missing(&self, ids: &[BlobId]) -> Vec<BlobId> {
        ids.iter().copied().filter(|id| !self.holds(*id)).collect()
    }

    /// Writes each of `blobs` the store does not hold to a temp file, fsyncs it and renames it
    /// into place. The directories are left to sync: until [`ContentStore::durable`] is told
    /// of them, the written blobs are [`missing`](ContentStore::missing).
    pub fn place(&self, blobs: &[Vec<u8>]) -> io::Result<Placed> {
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
            file.sync_all()?;
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

    /// Records that the directories of `placed` are synced: its blobs are held.
    pub fn durable(&self, placed: &Placed) {
        let mut unsynced = self.unsynced();
        for id in &placed.written {
            unsynced.remove(id);
        }
    }

    pub fn get(&self, id: BlobId) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.path(id)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Delete files not in `live` that are older than `grace`, including
    /// abandoned temp files. Returns the number of blobs deleted.
    pub fn sweep(&self, live: &HashSet<BlobId>, now: SystemTime, grace: Duration) -> io::Result<usize> {
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

/// Makes the entries of `dir` survive a crash.
#[cfg(unix)]
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
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
        store.durable(&placed);
        placed.ids
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
        store.durable(&placed);
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

        store.durable(&placed);
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
