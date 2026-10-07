//! Content-addressed files: written to a temp file, fsynced, renamed into place.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use vet_core::content::{BlobId, Hash};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct ContentStore {
    root: PathBuf,
}

impl ContentStore {
    pub fn open(root: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(root.join("tmp"))?;
        Ok(Self { root })
    }

    fn path(&self, id: BlobId) -> PathBuf {
        let hex = id.0.to_hex();
        self.root.join(&hex[..2]).join(&hex[2..])
    }

    pub fn put(&self, bytes: &[u8]) -> io::Result<BlobId> {
        self.put_with(BlobId::of(bytes), bytes, |_| {})
    }

    /// `durable_tmp` runs after the temp file is fsynced and before the rename.
    fn put_with(&self, id: BlobId, bytes: &[u8], durable_tmp: impl FnOnce(&Path)) -> io::Result<BlobId> {
        let dest = self.path(id);
        if dest.exists() {
            return Ok(id);
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
        let dir = dest.parent().expect("blob path has a parent");
        fs::create_dir_all(dir)?;
        fs::rename(&tmp, &dest)?;
        sync_dir(dir)?;
        Ok(id)
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

#[cfg(unix)]
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

// Directories cannot be opened for fsync on Windows; NTFS journals the rename.
#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> BlobId {
        BlobId(Hash([n; 32]))
    }

    #[test]
    fn blob_is_durable_in_tmp_before_it_is_visible() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(dir.path().to_path_buf()).unwrap();
        store
            .put_with(id(1), b"hello", |tmp| {
                assert_eq!(fs::read(tmp).unwrap(), b"hello");
                assert_eq!(store.get(id(1)).unwrap(), None);
            })
            .unwrap();
        assert_eq!(store.get(id(1)).unwrap().as_deref(), Some(&b"hello"[..]));
        assert_eq!(fs::read_dir(dir.path().join("tmp")).unwrap().count(), 0);
    }

    #[test]
    fn sweep_keeps_live_and_young_and_removes_abandoned_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(dir.path().to_path_buf()).unwrap();
        store.put_with(id(1), b"live", |_| {}).unwrap();
        store.put_with(id(2), b"dead", |_| {}).unwrap();
        fs::write(dir.path().join("tmp").join("crashed"), b"partial").unwrap();
        let live = HashSet::from([id(1)]);

        let young = store
            .sweep(&live, SystemTime::now(), Duration::from_secs(3600))
            .unwrap();
        assert_eq!(young, 0);
        assert!(store.get(id(2)).unwrap().is_some());

        let deleted = store.sweep(&live, SystemTime::now(), Duration::ZERO).unwrap();
        assert_eq!(deleted, 1);
        assert!(store.get(id(1)).unwrap().is_some());
        assert_eq!(store.get(id(2)).unwrap(), None);
        assert_eq!(fs::read_dir(dir.path().join("tmp")).unwrap().count(), 0);
    }
}
