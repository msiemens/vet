//! Dirty-tree scan (§6.1, §6.3): index first, hashing only entries whose stat
//! is uncertain, and only built-in content conversions.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use gix::bstr::{BStr, BString, ByteSlice};
use gix::filter::plumbing::pipeline::{CrlfRoundTripCheck, convert::ToGitOutcome};
use gix::index::entry::{Flags, Mode, Stat};
use gix::objs::Find;
use gix::worktree::stack::state::attributes::Source;
use unicode_normalization::UnicodeNormalization;
use vet_app::ports::{GitError, Scan};
use vet_core::content::{BlobId, EntryKind, FileEntry, Hash, RepoPath, Snapshot, SnapshotStatus};

use vet_core::merkle;

use super::stat_cache::{Record, StatCache, StatKey, file_stamp, now_secs};
use super::{BlobCache, err};

/// One worktree item, converted the way git would store it.
pub(crate) struct Item {
    pub entry: FileEntry,
    /// Over the capture cap: hashed, but not read into memory.
    pub unavailable: bool,
    /// Present only when asked for and available.
    pub bytes: Option<Vec<u8>>,
}

pub(crate) struct Worktree<'r> {
    repo: &'r gix::Repository,
    blobs: &'r BlobCache,
    index: gix::worktree::Index,
    filter: gix::filter::plumbing::Pipeline,
    attrs: gix::worktree::Stack,
    caps: gix::fs::Capabilities,
    stat: gix::index::entry::stat::Options,
    root: PathBuf,
    cap: u64,
    stats: &'r StatCache,
    prev: HashMap<RepoPath, Record>,
    next: HashMap<RepoPath, Record>,
    /// A `.gitattributes` file differs from the cached one.
    pub attributes_changed: bool,
}

impl<'r> Worktree<'r> {
    pub fn new(repo: &'r gix::Repository, blobs: &'r BlobCache, stats: &'r StatCache, cap: u64) -> Result<Self, GitError> {
        let index = repo.index_or_empty().map_err(err)?;
        let stack = repo.attributes_only(&index, Source::WorktreeThenIdMapping).map_err(err)?.detach();
        let (mut filter, attrs) = gix::filter::Pipeline::new(repo, stack).map_err(err)?.into_parts();
        let options = filter.options_mut();
        // Custom drivers are never run (§6.3).
        options.drivers.clear();
        options.crlf_roundtrip_check = CrlfRoundTripCheck::Skip;
        Ok(Self {
            repo,
            blobs,
            index,
            filter,
            attrs,
            caps: repo.filesystem_options().map_err(err)?,
            stat: repo.stat_options().map_err(err)?,
            root: repo.workdir().ok_or_else(|| GitError("bare repository".into()))?.to_owned(),
            cap,
            stats,
            prev: HashMap::new(),
            next: HashMap::new(),
            attributes_changed: false,
        })
    }

    pub fn item_by_path(&mut self, path: &BStr, keep: bool) -> Result<Option<Item>, GitError> {
        let tracked = self.index.entry_by_path(path).cloned();
        self.item(path, tracked.as_ref(), keep)
    }

    /// The item at `path`, or `None` if it is gone or cannot be tracked.
    pub fn item(&mut self, path: &BStr, tracked: Option<&gix::index::Entry>, keep: bool) -> Result<Option<Item>, GitError> {
        let full = self.root.join(gix::path::from_bstr(path));
        let md = match std::fs::symlink_metadata(&full) {
            Ok(md) => md,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(err(e)),
        };
        let index_mode = tracked.map(|t| t.mode);
        let kind = if md.is_symlink() {
            EntryKind::Symlink
        } else if md.is_file() {
            // Without symlink support a link is checked out as a file holding its target.
            if index_mode == Some(Mode::SYMLINK) && !self.caps.symlink { EntryKind::Symlink } else { EntryKind::File }
        } else {
            return Ok(None);
        };
        let mode = match kind {
            EntryKind::Symlink => 0o120000,
            _ if !self.caps.executable_bit => match index_mode {
                Some(m) if m == Mode::FILE || m == Mode::FILE_EXECUTABLE => m.bits(),
                _ => 0o100644,
            },
            _ if gix::fs::is_executable(&md) => 0o100755,
            _ => 0o100644,
        };
        let entry = |blob, unfiltered| FileEntry { blob, mode, kind, unfiltered };

        if kind == EntryKind::Symlink {
            let bytes = if md.is_symlink() {
                gix::path::into_bstr(std::fs::read_link(&full).map_err(err)?).into_owned().into()
            } else {
                std::fs::read(&full).map_err(err)?
            };
            return Ok(Some(Item { entry: entry(BlobId::of(&bytes), false), unavailable: false, bytes: keep.then_some(bytes) }));
        }

        if let Some(t) = tracked.filter(|t| self.stat_matches(t, &full)) {
            let blob = self.blobs.blob_id(self.repo, t.id)?;
            let bytes = if keep { self.blobs.odb_content(self.repo, blob, u64::MAX)? } else { None };
            return Ok(Some(Item { entry: entry(blob, false), unavailable: false, bytes }));
        }
        // Bytes are wanted for copying, so the cache cannot answer.
        let key = (!keep).then(|| StatKey::of(&md, index_mode.map(|m| m.bits())));
        let repo_path = RepoPath(path.to_vec());
        let cached = self.prev.get(&repo_path);
        if let (Some(key), Some(record)) = (&key, cached) {
            if record.matches(key) {
                let item = Item { entry: record.entry.clone(), unavailable: record.unavailable, bytes: None };
                self.next.insert(repo_path, record.clone());
                return Ok(Some(item));
            }
            self.attributes_changed |= path.ends_with(b".gitattributes");
        }

        let recorded_secs = now_secs();
        let item = self.hash_file(path, &full, md.len(), keep, entry)?;
        if let Some(key) = key {
            self.next.insert(
                repo_path,
                Record { key, entry: item.entry.clone(), unavailable: item.unavailable, recorded_secs },
            );
        }
        Ok(Some(item))
    }

    fn hash_file(&mut self, path: &BStr, full: &Path, len: u64, keep: bool, entry: impl Fn(BlobId, bool) -> FileEntry) -> Result<Item, GitError> {
        if len > self.cap {
            let blob = BlobId(Hash::of_reader(std::fs::File::open(full).map_err(err)?).map_err(err)?);
            return Ok(Item { entry: entry(blob, false), unavailable: true, bytes: None });
        }
        let raw = std::fs::read(full).map_err(err)?;
        let (converted, custom_filter) = self.convert(path, &raw)?;
        let (bytes, unfiltered) = match (converted, custom_filter) {
            (_, true) | (None, _) => (raw, custom_filter),
            (Some(converted), false) => (converted, false),
        };
        Ok(Item { entry: entry(BlobId::of(&bytes), unfiltered), unavailable: false, bytes: keep.then_some(bytes) })
    }

    /// Start a scan that reuses and refreshes the stat cache. Other uses of the
    /// worktree (diff, `read_blob`) neither read nor change it.
    pub fn use_stat_cache(&mut self) {
        let git_dir = self.repo.git_dir();
        let config = [git_dir.join("config"), git_dir.join("info/attributes")].iter().map(|p| file_stamp(p)).collect();
        self.prev = self.stats.take(config);
    }

    /// Drop cached entries; they may predate a change of conversion rules.
    pub fn forget_cached(&mut self) {
        self.prev.clear();
    }

    /// Hand the records of this scan back to the cache.
    pub fn finish(&mut self) {
        self.stats.put(std::mem::take(&mut self.next));
    }

    fn stat_matches(&self, tracked: &gix::index::Entry, full: &Path) -> bool {
        let md = gix::index::fs::Metadata::from_path_no_follow(full);
        md.ok().and_then(|md| Stat::from_fs(&md).ok()).is_some_and(|fs| {
            tracked.stat.matches(&fs, self.stat) && !tracked.stat.is_racy(self.index.timestamp(), self.stat)
        })
    }

    /// Built-in conversions (`eol`, `text`, `autocrlf`, `ident`), plus whether
    /// the path names a custom `filter` driver. `None` if nothing changed.
    fn convert(&mut self, path: &BStr, raw: &[u8]) -> Result<(Option<Vec<u8>>, bool), GitError> {
        let rela = gix::path::from_bstr(path);
        let platform = self.attrs.at_path(path, None, &self.repo.objects).map_err(err)?;
        let mut custom_filter = false;
        let (index, repo) = (&self.index, self.repo);
        let outcome = self
            .filter
            .convert_to_git(
                raw,
                &rela,
                &mut |_, attrs| {
                    platform.matching_attributes(attrs);
                    custom_filter = attrs.iter_selected().any(|m| {
                        m.assignment.name.as_str() == "filter" && matches!(m.assignment.state, gix::attrs::StateRef::Value(_))
                    });
                },
                &mut |buf| {
                    let Some(entry) = index.entry_by_path(path) else { return Ok(None) };
                    Ok(repo.objects.try_find(&entry.id, buf)?.filter(|o| o.kind == gix::objs::Kind::Blob).map(|_| ()))
                },
            )
            .map_err(err)?;
        let converted = match outcome {
            ToGitOutcome::Unchanged(_) => None,
            ToGitOutcome::Buffer(buf) => Some(buf.to_vec()),
            ToGitOutcome::Process(mut read) => {
                let mut out = Vec::new();
                read.read_to_end(&mut out).map_err(err)?;
                Some(out)
            }
        };
        Ok((converted, custom_filter))
    }
}

pub(crate) fn scan(repo: &gix::Repository, blobs: &BlobCache, stats: &StatCache, cap: u64) -> Result<Scan, GitError> {
    let mut wt = Worktree::new(repo, blobs, stats, cap)?;
    wt.use_stat_cache();
    let index = repo.index_or_empty().map_err(err)?;
    let mut entries = BTreeMap::new();
    let mut unavailable = Vec::new();
    let mut work: Vec<(BString, Option<gix::index::Entry>)> = Vec::new();

    for tracked in index.entries() {
        let path = tracked.path(&index);
        let key = RepoPath(path.to_vec());
        if entries.contains_key(&key) {
            continue; // further conflict stages
        }
        if tracked.mode == Mode::COMMIT || tracked.flags.contains(Flags::SKIP_WORKTREE) {
            entries.insert(key, tree_entry(repo, blobs, tracked.id, tracked.mode.bits())?);
        } else {
            work.push((path.to_owned(), Some(tracked.clone())));
        }
    }
    let tracked_paths: BTreeSet<_> = work.iter().map(|(p, _)| p.clone()).collect();

    // The dirwalk only knows an index entry is tracked if it is flagged up to
    // date, so it may report tracked files too; those are skipped here.
    let mut delegate = gix::dir::walk::delegate::Collect::default();
    let options = repo.dirwalk_options().map_err(err)?;
    repo.dirwalk(&index, Vec::<BString>::new(), &AtomicBool::new(false), options, &mut delegate).map_err(err)?;
    for (found, _) in delegate.into_entries_by_path() {
        if found.status != gix::dir::entry::Status::Untracked {
            continue;
        }
        let is_file = matches!(found.disk_kind, Some(gix::dir::entry::Kind::File | gix::dir::entry::Kind::Symlink));
        if is_file && !tracked_paths.contains(&found.rela_path) && !entries.contains_key(&RepoPath(found.rela_path.to_vec())) {
            work.push((found.rela_path, None));
        }
    }

    // Attribute files first: if one changed, cached entries may have been
    // converted under other rules and must not be reused.
    let is_attributes = |p: &BString| p.ends_with(b".gitattributes");
    work.sort_by_key(|(p, _)| !is_attributes(p));
    let attribute_files = work.iter().take_while(|(p, _)| is_attributes(p)).count();
    for (i, (path, tracked)) in work.iter().enumerate() {
        if i == attribute_files && wt.attributes_changed {
            wt.forget_cached();
        }
        if let Some(item) = wt.item(path.as_bstr(), tracked.as_ref(), false)? {
            let key = RepoPath(path.to_vec());
            if item.unavailable {
                unavailable.push(key.clone());
            }
            entries.insert(key, item.entry);
        }
    }
    wt.finish();

    let status = SnapshotStatus { path_collisions: collisions(entries.keys()), content_unavailable: unavailable };
    let id = merkle::snapshot_id(&entries);
    Ok(Scan { snapshot: Snapshot { id, entries }, status })
}

/// Paths that differ only in case or Unicode normalization.
fn collisions<'a>(paths: impl Iterator<Item = &'a RepoPath>) -> Vec<RepoPath> {
    let mut groups: BTreeMap<Vec<u8>, Vec<&RepoPath>> = BTreeMap::new();
    for path in paths {
        let key = match std::str::from_utf8(&path.0) {
            Ok(s) => s.nfc().collect::<String>().to_lowercase().into_bytes(),
            Err(_) => path.0.clone(),
        };
        groups.entry(key).or_default().push(path);
    }
    let colliding: BTreeSet<_> = groups.into_values().filter(|g| g.len() > 1).flatten().cloned().collect();
    colliding.into_iter().collect()
}

/// The entry for `oid` as recorded in a tree or index, with no worktree
/// involved. A submodule's blob is the hash of its pinned commit id.
pub(crate) fn tree_entry(repo: &gix::Repository, blobs: &BlobCache, oid: gix::ObjectId, mode: u32) -> Result<FileEntry, GitError> {
    let (kind, blob) = match mode & 0o170000 {
        0o160000 => (EntryKind::Submodule, BlobId::of(oid.as_bytes())),
        0o120000 => (EntryKind::Symlink, blobs.blob_id(repo, oid)?),
        _ => (EntryKind::File, blobs.blob_id(repo, oid)?),
    };
    Ok(FileEntry { blob, mode, kind, unfiltered: false })
}
