//! Snapshot diff (§5, §6.1): line hunks for text, one item per non-text change,
//! renames detected by gitoxide's rewrite tracker.

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;

use gix::ObjectId;
use gix::bstr::ByteSlice;
use gix::diff::blob::{Algorithm, Diff, InternedInput, sources::byte_lines};
use gix::diff::rewrites::{Tracker, tracker::ChangeKind};
use gix::diff::tree::visit::Action;
use gix::objs::{
    Find, FindHeader, Kind,
    tree::{EntryKind as GitKind, EntryMode},
};
use vet_app::ports::GitError;
use vet_core::content::{
    BlobId, BlobSource, EntryKind, FileDiff, FileEntry, Hunk, LineRange, NonTextChange, NonTextKind, RenameMap,
    RepoPath, Snapshot, SnapshotDiff,
};

use super::scan::Worktree;
use super::{BlobCache, err};

type Side = Option<(RepoPath, FileEntry)>;

pub(crate) struct Change {
    pub old: Side,
    pub new: Side,
}

/// Entries that differ between the snapshots, by path.
pub(crate) fn changes(old: &Snapshot, new: &Snapshot) -> Vec<Change> {
    let side = |snap: &Snapshot, path: &RepoPath| snap.entries.get(path).map(|e| (path.clone(), e.clone()));
    let mut paths: Vec<&RepoPath> = old.entries.keys().chain(new.entries.keys()).collect();
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .filter(|p| old.entries.get(*p) != new.entries.get(*p))
        .map(|p| Change {
            old: side(old, p),
            new: side(new, p),
        })
        .collect()
}

/// Blobs whose text a diff of `changes` needs.
pub(crate) fn text_blobs(changes: &[Change]) -> Vec<(RepoPath, BlobId)> {
    changes
        .iter()
        .flat_map(|c| [&c.old, &c.new])
        .flatten()
        .filter(|(_, e)| e.kind == EntryKind::File)
        .map(|(p, e)| (p.clone(), e.blob))
        .collect()
}

/// `contents` holds what the content store had; the rest is looked up in git
/// or the worktree. Anything unavailable ends up as a `ContentUnavailable` item.
pub(crate) fn diff(
    repo: &gix::Repository,
    wt: &mut Worktree,
    blobs: &BlobCache,
    cap: u64,
    mut contents: BTreeMap<BlobId, Vec<u8>>,
    old: &Snapshot,
    new: &Snapshot,
) -> Result<SnapshotDiff, GitError> {
    let mut changes = changes(old, new);
    for (path, blob) in text_blobs(&changes) {
        if contents.blob(blob).is_none()
            && let Some(bytes) = local_blob(repo, wt, blobs, cap, &path, blob)?
        {
            contents.insert(blob, bytes);
        }
    }

    let renames = detect_renames(repo, &changes, &contents)?;
    for (from, to) in &renames {
        let old_side = take(&mut changes, |c| {
            c.new.is_none() && c.old.as_ref().is_some_and(|(p, _)| p == from)
        });
        let new_side = take(&mut changes, |c| {
            c.old.is_none() && c.new.as_ref().is_some_and(|(p, _)| p == to)
        });
        changes.push(Change {
            old: old_side.and_then(|c| c.old),
            new: new_side.and_then(|c| c.new),
        });
    }
    changes.sort_by(|a, b| key(a).cmp(key(b)));

    let mut out = SnapshotDiff {
        renames: RenameMap(renames.into_iter().collect()),
        ..Default::default()
    };
    for change in changes {
        classify(change, &contents, &mut out);
    }
    Ok(out)
}

fn key(c: &Change) -> &RepoPath {
    &c.new.as_ref().or(c.old.as_ref()).expect("a change has a side").0
}

fn take(changes: &mut Vec<Change>, f: impl Fn(&Change) -> bool) -> Option<Change> {
    changes.iter().position(f).map(|i| changes.remove(i))
}

/// Git first, then the worktree if the file still hashes to `id`.
fn local_blob(
    repo: &gix::Repository,
    wt: &mut Worktree,
    blobs: &BlobCache,
    cap: u64,
    path: &RepoPath,
    blob: BlobId,
) -> Result<Option<Vec<u8>>, GitError> {
    if let Some(bytes) = blobs.odb_content(repo, blob, cap)? {
        return Ok(Some(bytes));
    }
    let item = wt.item_by_path(path.0.as_bstr(), true)?;
    Ok(item.filter(|i| i.entry.blob == blob).and_then(|i| i.bytes))
}

#[derive(Clone)]
struct Ch {
    id: ObjectId,
    kind: ChangeKind,
    mode: EntryMode,
}

impl gix::diff::rewrites::tracker::Change for Ch {
    fn id(&self) -> &gix::oid {
        &self.id
    }
    fn relation(&self) -> Option<gix::diff::tree::visit::Relation> {
        None
    }
    fn kind(&self) -> ChangeKind {
        self.kind
    }
    fn entry_mode(&self) -> EntryMode {
        self.mode
    }
    fn id_and_entry_mode(&self) -> (&gix::oid, EntryMode) {
        (&self.id, self.mode)
    }
}

/// Serves blob contents the rewrite tracker asks for.
struct Contents<'a>(HashMap<ObjectId, &'a [u8]>);

impl Find for Contents<'_> {
    fn try_find<'a>(&self, id: &gix::oid, buffer: &'a mut Vec<u8>) -> gix::ExnResult<Option<gix::objs::Data<'a>>> {
        Ok(self.0.get(id).map(|bytes| {
            buffer.clear();
            buffer.extend_from_slice(bytes);
            gix::objs::Data::new(buffer, Kind::Blob, gix::hash::Kind::Sha1)
        }))
    }
}

impl FindHeader for Contents<'_> {
    fn try_header(&self, id: &gix::oid) -> gix::ExnResult<Option<gix::objs::Header>> {
        Ok(self.0.get(id).map(|bytes| gix::objs::Header {
            kind: Kind::Blob,
            size: bytes.len() as u64,
        }))
    }
}

fn detect_renames(
    repo: &gix::Repository,
    changes: &[Change],
    contents: &BTreeMap<BlobId, Vec<u8>>,
) -> Result<Vec<(RepoPath, RepoPath)>, GitError> {
    let mut tracker = Tracker::new(gix::diff::Rewrites::default());
    let mut git_contents = HashMap::new();
    for change in changes {
        let (kind, (path, entry)) = match (&change.old, &change.new) {
            (Some(old), None) => (ChangeKind::Deletion, old),
            (None, Some(new)) => (ChangeKind::Addition, new),
            _ => continue,
        };
        let Some(bytes) = contents.blob(entry.blob).filter(|_| entry.kind == EntryKind::File) else {
            continue;
        };
        // The tracker compares git object ids.
        let id = gix::objs::compute_hash(repo.object_hash(), Kind::Blob, bytes).map_err(err)?;
        git_contents.insert(id, bytes);
        let mode = if entry.mode & 0o111 != 0 {
            GitKind::BlobExecutable
        } else {
            GitKind::Blob
        };
        tracker.try_push_change(
            Ch {
                id,
                kind,
                mode: mode.into(),
            },
            path.0.as_bstr(),
        );
    }

    let mut found = Vec::new();
    let mut cache = repo.diff_resource_cache_for_tree_diff().map_err(err)?;
    tracker
        .emit(
            |dest, source| {
                if let Some(source) = source {
                    found.push((RepoPath(source.location.to_vec()), RepoPath(dest.location.to_vec())));
                }
                Action::Continue(())
            },
            &mut cache,
            &Contents(git_contents),
            |_| Ok::<(), Infallible>(()),
        )
        .map_err(err)?;
    Ok(found)
}

fn classify(change: Change, contents: &BTreeMap<BlobId, Vec<u8>>, out: &mut SnapshotDiff) {
    let kind_of = |s: &Side| s.as_ref().map(|(_, e)| e.kind);
    let item = |kind, change: &Change| NonTextChange {
        path: key(change).clone(),
        from: (kind == NonTextKind::Rename).then(|| key_old(change).clone()),
        kind,
        before: change.old.as_ref().map(|(_, e)| e.clone()),
        after: change.new.as_ref().map(|(_, e)| e.clone()),
    };
    let (old_kind, new_kind) = (kind_of(&change.old), kind_of(&change.new));

    if old_kind == Some(EntryKind::Submodule) || new_kind == Some(EntryKind::Submodule) {
        out.non_text.push(item(NonTextKind::Submodule, &change));
        return;
    }
    if old_kind == Some(EntryKind::Symlink) || new_kind == Some(EntryKind::Symlink) {
        out.non_text.push(item(NonTextKind::Symlink, &change));
        return;
    }

    let (old_blob, new_blob) = (
        change.old.as_ref().map(|(_, e)| e.blob),
        change.new.as_ref().map(|(_, e)| e.blob),
    );
    if let (Some((_, o)), Some((_, n))) = (&change.old, &change.new) {
        if o.mode != n.mode {
            out.non_text.push(item(NonTextKind::Mode, &change));
        }
        if o.blob == n.blob {
            if key_old(&change) != key(&change) {
                out.non_text.push(item(NonTextKind::Rename, &change));
            }
            return;
        }
    }

    let text = |blob: Option<BlobId>| match blob {
        None => Some(&[][..]),
        Some(b) => contents.blob(b),
    };
    let (Some(before), Some(after)) = (text(old_blob), text(new_blob)) else {
        out.non_text.push(item(NonTextKind::ContentUnavailable, &change));
        return;
    };
    if is_binary(before) || is_binary(after) {
        out.non_text.push(item(NonTextKind::Binary, &change));
        return;
    }
    out.files.push(FileDiff {
        old_path: change.old.as_ref().map(|(p, _)| p.clone()),
        new_path: change.new.as_ref().map(|(p, _)| p.clone()),
        old_blob,
        new_blob,
        hunks: line_hunks(before, after),
    });
}

fn key_old(c: &Change) -> &RepoPath {
    &c.old.as_ref().expect("both sides").0
}

/// Git's heuristic: a NUL in the first 8000 bytes.
fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(8000)].contains(&0)
}

/// 0-based line hunks, histogram algorithm.
pub fn line_hunks(old: &[u8], new: &[u8]) -> Vec<Hunk> {
    let input = InternedInput::new(byte_lines(old), byte_lines(new));
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    let range = |r: std::ops::Range<u32>| LineRange {
        start: r.start,
        end: r.end,
    };
    diff.hunks()
        .map(|h| Hunk {
            old: range(h.before),
            new: range(h.after),
        })
        .collect()
}
