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
use vet_app::ports::{GitError, Scan};
use vet_core::content::{BlobId, EntryKind, FileEntry, Hash, RepoPath, Snapshot, SnapshotStatus};

use vet_core::merkle;

use super::stat_cache::{Record, StatCache, StatKey, file_stamp};
use super::tree::tree_entry;
use super::{BlobCache, err, in_tool_store};
use crate::clock::unix_now;
use crate::paths;

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
    pub fn new(
        repo: &'r gix::Repository,
        blobs: &'r BlobCache,
        stats: &'r StatCache,
        cap: u64,
    ) -> Result<Self, GitError> {
        let index = repo.index_or_empty().map_err(err)?;
        let stack = repo
            .attributes_only(&index, Source::WorktreeThenIdMapping)
            .map_err(err)?
            .detach();
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
            root: repo
                .workdir()
                .ok_or_else(|| GitError("bare repository".into()))?
                .to_owned(),
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
    pub fn item(
        &mut self,
        path: &BStr,
        tracked: Option<&gix::index::Entry>,
        keep: bool,
    ) -> Result<Option<Item>, GitError> {
        let full = self.root.join(paths::to_path(path).map_err(err)?);
        let at = |e: std::io::Error| err(format!("{}: {e}", full.display()));
        let mut md = match std::fs::symlink_metadata(&full) {
            Ok(md) => md,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(at(e)),
        };
        // Windows reports other reparse points (cloud placeholders, deduplicated files, app
        // execution aliases) as symlinks that `read_link` cannot read. Like Git for Windows,
        // treat them as what they point at.
        let mut link_target = None;
        if md.is_symlink() {
            match std::fs::read_link(&full) {
                Ok(target) => link_target = Some(target),
                Err(_) => match std::fs::metadata(&full) {
                    Ok(followed) => md = followed,
                    Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
                    // A link whose target cannot be accessed: recorded, but without content.
                    Err(_) => {
                        return Ok(Some(Item {
                            entry: FileEntry {
                                blob: BlobId::of(&[]),
                                mode: 0o120000,
                                kind: EntryKind::Symlink,
                                unfiltered: false,
                            },
                            unavailable: true,
                            bytes: None,
                        }));
                    }
                },
            }
        }
        let index_mode = tracked.map(|t| t.mode);
        let kind = if link_target.is_some() {
            EntryKind::Symlink
        } else if md.is_file() {
            // Without symlink support a link is checked out as a file holding its target.
            if index_mode == Some(Mode::SYMLINK) && !self.caps.symlink {
                EntryKind::Symlink
            } else {
                EntryKind::File
            }
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
        let entry = |blob, unfiltered| FileEntry {
            blob,
            mode,
            kind,
            unfiltered,
        };

        if kind == EntryKind::Symlink {
            let bytes = match link_target {
                Some(target) => gix::path::into_bstr(target).into_owned().into(),
                None => std::fs::read(&full).map_err(at)?,
            };
            return Ok(Some(Item {
                entry: entry(BlobId::of(&bytes), false),
                unavailable: false,
                bytes: keep.then_some(bytes),
            }));
        }

        if let Some(t) = tracked.filter(|t| self.stat_matches(t, &full)) {
            let blob = self.blobs.blob_id(self.repo, t.id)?;
            let bytes = if keep {
                self.blobs.odb_content(self.repo, blob, u64::MAX)?
            } else {
                None
            };
            return Ok(Some(Item {
                entry: entry(blob, false),
                unavailable: false,
                bytes,
            }));
        }
        // Bytes are wanted for copying, so the cache cannot answer.
        let key = (!keep).then(|| StatKey::of(&md, index_mode.map(|m| m.bits())));
        let repo_path = RepoPath(path.to_vec());
        let cached = self.prev.get(&repo_path);
        if let (Some(key), Some(record)) = (&key, cached) {
            if record.matches(key) {
                let item = Item {
                    entry: record.entry.clone(),
                    unavailable: record.unavailable,
                    bytes: None,
                };
                self.next.insert(repo_path, record.clone());
                return Ok(Some(item));
            }
            self.attributes_changed |= path.ends_with(b".gitattributes");
        }

        let recorded_secs = unix_now();
        let item = self.hash_file(path, &full, md.len(), keep, entry)?;
        if let Some(key) = key {
            self.next.insert(
                repo_path,
                Record {
                    key,
                    entry: item.entry.clone(),
                    unavailable: item.unavailable,
                    recorded_secs,
                },
            );
        }
        Ok(Some(item))
    }

    fn hash_file(
        &mut self,
        path: &BStr,
        full: &Path,
        len: u64,
        keep: bool,
        entry: impl Fn(BlobId, bool) -> FileEntry,
    ) -> Result<Item, GitError> {
        let at = |e: std::io::Error| err(format!("{}: {e}", full.display()));
        if len > self.cap {
            let blob = BlobId(Hash::of_reader(std::fs::File::open(full).map_err(at)?).map_err(at)?);
            return Ok(Item {
                entry: entry(blob, false),
                unavailable: true,
                bytes: None,
            });
        }
        let raw = std::fs::read(full).map_err(at)?;
        let (converted, custom_filter) = self.convert(path, &raw)?;
        let (bytes, unfiltered) = match (converted, custom_filter) {
            (_, true) | (None, _) => (raw, custom_filter),
            (Some(converted), false) => (converted, false),
        };
        Ok(Item {
            entry: entry(BlobId::of(&bytes), unfiltered),
            unavailable: false,
            bytes: keep.then_some(bytes),
        })
    }

    /// Start a scan that reuses and refreshes the stat cache. Other uses of the
    /// worktree (diff, `read_blob`) neither read nor change it.
    pub fn use_stat_cache(&mut self) {
        let git_dir = self.repo.git_dir();
        let config = [git_dir.join("config"), git_dir.join("info/attributes")]
            .iter()
            .map(|p| file_stamp(p))
            .collect();
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
        let rela = paths::to_path(path).map_err(err)?;
        let platform = self.attrs.at_path(path, None, &self.repo.objects).map_err(err)?;
        let mut custom_filter = false;
        let (index, repo) = (&self.index, self.repo);
        let outcome = self
            .filter
            .convert_to_git(
                raw,
                rela,
                &mut |_, attrs| {
                    platform.matching_attributes(attrs);
                    custom_filter = attrs.iter_selected().any(|m| {
                        m.assignment.name.as_str() == "filter"
                            && matches!(m.assignment.state, gix::attrs::StateRef::Value(_))
                    });
                },
                &mut |buf| {
                    let Some(entry) = index.entry_by_path(path) else {
                        return Ok(None);
                    };
                    Ok(repo
                        .objects
                        .try_find(&entry.id, buf)?
                        .filter(|o| o.kind == gix::objs::Kind::Blob)
                        .map(|_| ()))
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

/// Collects the dirwalk's entries, but never enters or reports a tool store.
#[derive(Default)]
struct SkipToolStores(gix::dir::walk::delegate::Collect);

impl gix::dir::walk::Delegate for SkipToolStores {
    fn emit(
        &mut self,
        entry: gix::dir::EntryRef<'_>,
        collapsed_directory_status: Option<gix::dir::entry::Status>,
    ) -> gix::dir::walk::Action {
        if in_tool_store(&entry.rela_path) {
            return std::ops::ControlFlow::Continue(());
        }
        self.0.emit(entry, collapsed_directory_status)
    }

    fn can_recurse(
        &mut self,
        entry: gix::dir::EntryRef<'_>,
        for_deletion: Option<gix::dir::walk::ForDeletionMode>,
        worktree_root_is_repository: bool,
    ) -> bool {
        !in_tool_store(&entry.rela_path) && self.0.can_recurse(entry, for_deletion, worktree_root_is_repository)
    }
}

/// The files git neither tracks nor ignores, tool stores left out. The dirwalk
/// only knows an index entry is tracked if it is flagged up to date, so it may
/// report tracked files too; callers skip those.
fn untracked(repo: &gix::Repository, index: &gix::worktree::Index) -> Result<Vec<BString>, GitError> {
    let mut delegate = SkipToolStores::default();
    let options = repo.dirwalk_options().map_err(err)?;
    repo.dirwalk(
        index,
        Vec::<BString>::new(),
        &AtomicBool::new(false),
        options,
        &mut delegate,
    )
    .map_err(err)?;
    Ok(delegate
        .0
        .into_entries_by_path()
        .into_iter()
        .filter(|(found, _)| {
            found.status == gix::dir::entry::Status::Untracked
                && matches!(
                    found.disk_kind,
                    Some(gix::dir::entry::Kind::File | gix::dir::entry::Kind::Symlink)
                )
        })
        .map(|(found, _)| found.rela_path)
        .collect())
}

/// Every file of the tree as git sees it, without reading any: what the index
/// holds and what is untracked and not ignored.
pub(crate) fn list(repo: &gix::Repository) -> Result<Vec<RepoPath>, GitError> {
    let index = repo.index_or_empty().map_err(err)?;
    let tracked = index.entries().iter().map(|e| e.path(&index).to_vec());
    let mut files: Vec<RepoPath> = tracked
        .chain(untracked(repo, &index)?.into_iter().map(|p| p.to_vec()))
        .map(RepoPath)
        .collect();
    files.sort();
    files.dedup();
    Ok(files)
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
            let id = if tracked.mode == Mode::COMMIT && !tracked.flags.contains(Flags::SKIP_WORKTREE) {
                submodule_head(repo, path).unwrap_or(tracked.id)
            } else {
                tracked.id
            };
            entries.insert(key, tree_entry(repo, blobs, id, tracked.mode.bits())?);
        } else {
            work.push((path.to_owned(), Some(tracked.clone())));
        }
    }
    let tracked_paths: BTreeSet<_> = work.iter().map(|(p, _)| p.clone()).collect();

    for path in untracked(repo, &index)? {
        if !tracked_paths.contains(&path) && !entries.contains_key(&RepoPath(path.to_vec())) {
            work.push((path, None));
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

    let id = merkle::snapshot_id(&entries);
    let snapshot = Snapshot { id, entries };
    let status = SnapshotStatus {
        content_unavailable: unavailable,
        ..SnapshotStatus::of(&snapshot)
    };
    Ok(Scan { snapshot, status })
}

/// The commit a checked-out submodule is at; `None` if it is not checked out.
fn submodule_head(repo: &gix::Repository, path: &BStr) -> Option<gix::ObjectId> {
    let dir = repo.workdir()?.join(paths::to_path(path).ok()?);
    Some(gix::open(dir).ok()?.head_id().ok()?.detach())
}
