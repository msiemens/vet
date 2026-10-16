//! Dirty-tree scan (§4.1, §4.3): index first, hashing only entries whose stat
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
use vet_core::content::{BlobId, EntryKind, FileEntry, Hash, RepoPath, Snapshot, SnapshotStatus, in_tool_store};

use vet_core::merkle;

use super::stat_cache::{Record, StatCache, StatKey, file_stamp};
use super::tree::tree_entry;
use super::{BlobCache, chained, err};
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
    pub(super) fn new(
        repo: &'r gix::Repository,
        blobs: &'r BlobCache,
        stats: &'r StatCache,
        cap: u64,
    ) -> Result<Self, GitError> {
        let index = repo.index_or_empty().map_err(chained)?;
        let stack = repo
            .attributes_only(&index, Source::WorktreeThenIdMapping)
            .map_err(chained)?
            .detach();
        let (mut filter, attrs) = gix::filter::Pipeline::new(repo, stack).map_err(chained)?.into_parts();
        let options = filter.options_mut();
        // Custom drivers are never run (§4.3).
        options.drivers.clear();
        options.crlf_roundtrip_check = CrlfRoundTripCheck::Skip;
        Ok(Self {
            repo,
            blobs,
            index,
            filter,
            attrs,
            caps: repo.filesystem_options().map_err(chained)?,
            stat: repo.stat_options().map_err(chained)?,
            root: repo
                .workdir()
                .ok_or_else(|| GitError::failed("bare repository"))?
                .to_owned(),
            cap,
            stats,
            prev: HashMap::new(),
            next: HashMap::new(),
            attributes_changed: false,
        })
    }

    pub(super) fn item_by_path(&mut self, path: &BStr, keep: bool) -> Result<Option<Item>, GitError> {
        let tracked = self.index.entry_by_path(path).cloned();
        self.item(path, tracked.as_ref(), keep)
    }

    /// The item at `path`, or `None` if it is gone or cannot be tracked.
    pub(super) fn item(
        &mut self,
        path: &BStr,
        tracked: Option<&gix::index::Entry>,
        keep: bool,
    ) -> Result<Option<Item>, GitError> {
        let full = self.root.join(paths::to_path(path).map_err(chained)?);
        let mut md = match std::fs::symlink_metadata(&full) {
            Ok(md) => md,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return self.unreadable(&full, tracked, &e),
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
                None => match std::fs::read(&full) {
                    Ok(bytes) => bytes,
                    Err(e) => return self.unreadable(&full, tracked, &e),
                },
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
        let Some(item) = self.hash_file(path, &full, md.len(), keep, tracked, entry)? else {
            return Ok(None);
        };
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

    /// An entry the working tree cannot be read: a tracked file keeps the content
    /// git last recorded, so it is not treated as deleted; anything else is left
    /// out. Both the path and the error are reported. An entry that is gone is
    /// treated like a deletion: left out, and nothing to report.
    fn unreadable(
        &self,
        full: &Path,
        tracked: Option<&gix::index::Entry>,
        error: &std::io::Error,
    ) -> Result<Option<Item>, GitError> {
        if error.kind() == ErrorKind::NotFound {
            return Ok(None);
        }
        tracing::warn!(path = %full.display(), error = %error, "skipping an entry the working tree cannot be read");
        let Some(tracked) = tracked else {
            return Ok(None);
        };
        let blob = self.blobs.blob_id(self.repo, tracked.id)?;
        Ok(Some(Item {
            entry: FileEntry {
                blob,
                mode: tracked.mode.bits(),
                kind: if tracked.mode == Mode::SYMLINK {
                    EntryKind::Symlink
                } else {
                    EntryKind::File
                },
                unfiltered: false,
            },
            unavailable: true,
            bytes: None,
        }))
    }

    fn hash_file(
        &mut self,
        path: &BStr,
        full: &Path,
        len: u64,
        keep: bool,
        tracked: Option<&gix::index::Entry>,
        entry: impl Fn(BlobId, bool) -> FileEntry,
    ) -> Result<Option<Item>, GitError> {
        if len > self.cap {
            let file = match std::fs::File::open(full) {
                Ok(file) => file,
                Err(e) => return self.unreadable(full, tracked, &e),
            };
            let blob = match Hash::of_reader(file) {
                Ok(hash) => BlobId(hash),
                Err(e) => return self.unreadable(full, tracked, &e),
            };
            return Ok(Some(Item {
                entry: entry(blob, false),
                unavailable: true,
                bytes: None,
            }));
        }
        let raw = match std::fs::read(full) {
            Ok(raw) => raw,
            Err(e) => return self.unreadable(full, tracked, &e),
        };
        let (converted, custom_filter) = self.convert(path, &raw)?;
        let (bytes, unfiltered) = match (converted, custom_filter) {
            (_, true) | (None, _) => (raw, custom_filter),
            (Some(converted), false) => (converted, false),
        };
        Ok(Some(Item {
            entry: entry(BlobId::of(&bytes), unfiltered),
            unavailable: false,
            bytes: keep.then_some(bytes),
        }))
    }

    /// Start a scan that reuses and refreshes the stat cache. Other uses of the
    /// worktree (diff, `read_blob`) neither read nor change it.
    pub(super) fn use_stat_cache(&mut self) {
        let git_dir = self.repo.git_dir();
        let config = [git_dir.join("config"), git_dir.join("info/attributes")]
            .iter()
            .map(|p| file_stamp(p))
            .collect();
        self.prev = self.stats.take(config);
    }

    /// Drop cached entries; they may predate a change of conversion rules.
    pub(super) fn forget_cached(&mut self) {
        self.prev.clear();
    }

    /// Hand the records of this scan back to the cache.
    pub(super) fn finish(&mut self) {
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
        let rela = paths::to_path(path).map_err(chained)?;
        let platform = self.attrs.at_path(path, None, &self.repo.objects).map_err(chained)?;
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
                read.read_to_end(&mut out).map_err(chained)?;
                Some(out)
            }
        };
        Ok((converted, custom_filter))
    }
}

/// Collects the dirwalk's entries, but never enters or reports a tool store, and
/// never enters a directory named in `skip`. The walk itself is the only reader of
/// a directory: when it cannot read one, the failure names it through `last_dir`
/// and the walk is run again with that directory skipped. `before_read` is called
/// with each directory about to be entered and each entry about to be reported;
/// production uses a no-op, tests use it to change the tree under the walk.
struct SkipToolStores<H> {
    collect: gix::dir::walk::delegate::Collect,
    /// Relative paths of directories no retry may enter.
    skip: BTreeSet<BString>,
    /// The directory recursion was last allowed into, and so the one whose reading
    /// the walk failed on if it failed.
    last_dir: Option<BString>,
    before_read: H,
}

impl<H: FnMut(&BStr)> SkipToolStores<H> {
    fn new(skip: BTreeSet<BString>, before_read: H) -> Self {
        Self {
            collect: gix::dir::walk::delegate::Collect::default(),
            skip,
            last_dir: None,
            before_read,
        }
    }

    /// Lets `before_read` see a path just before the walk reads or reports it.
    fn note(&mut self, rela_path: &BStr) {
        (self.before_read)(rela_path);
    }
}

impl<H: FnMut(&BStr)> gix::dir::walk::Delegate for SkipToolStores<H> {
    fn emit(
        &mut self,
        entry: gix::dir::EntryRef<'_>,
        collapsed_directory_status: Option<gix::dir::entry::Status>,
    ) -> gix::dir::walk::Action {
        if in_tool_store(&entry.rela_path) {
            return std::ops::ControlFlow::Continue(());
        }
        self.note(entry.rela_path.as_ref());
        self.collect.emit(entry, collapsed_directory_status)
    }

    fn can_recurse(
        &mut self,
        entry: gix::dir::EntryRef<'_>,
        for_deletion: Option<gix::dir::walk::ForDeletionMode>,
        worktree_root_is_repository: bool,
    ) -> bool {
        if in_tool_store(&entry.rela_path) {
            return false;
        }
        let rela_path = entry.rela_path.clone().into_owned();
        if !self
            .collect
            .can_recurse(entry, for_deletion, worktree_root_is_repository)
        {
            return false;
        }
        if self.skip.contains(&rela_path) {
            return false;
        }
        // gix-dir 0.30 reads a directory as the first thing it does after `can_recurse`
        // lets it in: `can_recurse` (walk/readdir.rs:77) is followed by `recursive`
        // (walk/readdir.rs:85), whose first act is `read_dir` (walk/readdir.rs:41); the
        // root is the same (walk/function.rs:76 then :110). Neither the classification
        // before `can_recurse` (walk/readdir.rs:67) nor collapsing a directory
        // (walk/readdir.rs:288-419) reads a directory or asks again. So this is the
        // directory the walk was reading when it fails.
        self.note(rela_path.as_bstr());
        self.last_dir = Some(rela_path);
        true
    }
}

/// The files git neither tracks nor ignores, tool stores left out. The dirwalk only
/// knows an index entry is tracked if it is flagged up to date, so it may report
/// tracked files too; callers skip those. A directory the walk cannot read — its
/// entries or the ignore file inside it — is left out rather than treated as if it
/// held nothing: the walk is run again with it skipped, since gix aborts on the first
/// thing it cannot read and offers no way to continue past it.
fn untracked<H: FnMut(&BStr)>(
    repo: &gix::Repository,
    index: &gix::worktree::Index,
    mut before_read: H,
) -> Result<Vec<BString>, GitError> {
    let root = repo
        .workdir()
        .ok_or_else(|| GitError::failed("bare repository"))?
        .to_owned();
    let mut skip = BTreeSet::new();
    loop {
        // One walk; the delegate borrows `before_read` only for this pass.
        let (last_dir, error) = {
            let mut delegate = SkipToolStores::new(skip.clone(), &mut before_read);
            let options = repo.dirwalk_options().map_err(chained)?;
            let walked = repo.dirwalk(
                index,
                Vec::<BString>::new(),
                &AtomicBool::new(false),
                options,
                &mut delegate,
            );
            match walked {
                Ok(_) => {
                    return Ok(delegate
                        .collect
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
                        .collect());
                }
                Err(error) => (delegate.last_dir, error),
            }
        };
        // Only a failure to read a directory is worth another pass; anything
        // else is the scan's own problem.
        if error.downcast_any_ref::<std::io::Error>().is_none() {
            return Err(chained(error));
        }
        // A root that cannot be read stays fatal.
        let Some(path) = last_dir else {
            return Err(chained(error));
        };
        if path.is_empty() || skip.contains(&path) {
            return Err(chained(error));
        }
        // `last_dir` names the directory the walk was reading when it failed, but it
        // could be a wrong guess. The walk reads a directory's entries and the ignore
        // file inside it; only a failure on one of those is really about this directory,
        // and skipping a readable directory would hide the untracked files inside it.
        let full = paths::to_path(path.as_bstr()).map_or_else(|_| root.clone(), |rela| root.join(rela));
        let unreadable = if std::fs::read_dir(&full).is_err() {
            full.clone()
        } else {
            let ignore = full.join(".gitignore");
            if !ignore_unreadable(&ignore) {
                return Err(chained(error));
            }
            ignore
        };
        skip.insert(path);
        if !error.classify().is_not_found() {
            tracing::warn!(
                directory = %full.display(),
                unreadable = %unreadable.display(),
                error = %super::chain_text(&error),
                "skipping a directory the working tree cannot be read"
            );
        }
    }
}

/// Whether gix would fail to read `path` as an ignore file. A file that is not there, or a
/// directory in its place, is not a failure.
fn ignore_unreadable(path: &Path) -> bool {
    match std::fs::read(path) {
        Ok(_) => false,
        Err(error) => error.kind() != ErrorKind::NotFound && !names_a_directory(&error),
    }
}

/// Whether the OS reports the path as a directory rather than a readable file.
fn names_a_directory(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::IsADirectory | ErrorKind::NotADirectory)
        || cfg!(windows) && error.kind() == ErrorKind::PermissionDenied
}

/// Every file of the tree as git sees it, without reading any: what the index
/// holds and what is untracked and not ignored.
pub(crate) fn list(repo: &gix::Repository) -> Result<Vec<RepoPath>, GitError> {
    let index = repo.index_or_empty().map_err(chained)?;
    let tracked = index.entries().iter().map(|e| e.path(&index).to_vec());
    let mut files: Vec<RepoPath> = tracked
        .chain(untracked(repo, &index, |_| {})?.into_iter().map(|p| p.to_vec()))
        .map(RepoPath)
        .collect();
    files.sort();
    files.dedup();
    Ok(files)
}

pub(crate) fn scan(repo: &gix::Repository, blobs: &BlobCache, stats: &StatCache, cap: u64) -> Result<Scan, GitError> {
    scan_with(repo, blobs, stats, cap, |_| {})
}

/// [`scan`], with `before_read` handed to the walk so a test can change the tree under it.
fn scan_with<H: FnMut(&BStr)>(
    repo: &gix::Repository,
    blobs: &BlobCache,
    stats: &StatCache,
    cap: u64,
    mut before_read: H,
) -> Result<Scan, GitError> {
    let mut wt = Worktree::new(repo, blobs, stats, cap)?;
    wt.use_stat_cache();
    let index = repo.index_or_empty().map_err(chained)?;
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

    for path in untracked(repo, &index, &mut before_read)? {
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

#[cfg(test)]
mod tests;
