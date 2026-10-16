//! Snapshot diff (§4.1, §5): line hunks for text, one item per non-text change,
//! renames detected by gitoxide's rewrite tracker.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::convert::Infallible;

use gix::ObjectId;
use gix::bstr::ByteSlice;
use gix::diff::rewrites::{Tracker, tracker::ChangeKind};
use gix::diff::tree::visit::Action;
use gix::objs::{
    Find, FindHeader, Kind,
    tree::{EntryKind as GitKind, EntryMode},
};
use vet_app::ports::{GitError, TextDiff};
use vet_core::classify::{Change, changes, classify, text_blobs};
use vet_core::content::{BlobId, BlobSource, EntryKind, FileDiff, RenameMap, RepoPath, Snapshot, SnapshotDiff};
use vet_core::hunks::line_hunks;

use super::scan::Worktree;
use super::{BlobCache, err};

/// `contents` holds what the content store had; the rest is looked up in git
/// or the worktree. Anything unavailable ends up as a `ContentUnavailable` item.
/// The content read comes with the diff, so that it is read once.
pub(crate) fn diff(
    repo: &gix::Repository,
    wt: &mut Worktree,
    blobs: &BlobCache,
    cap: u64,
    mut contents: BTreeMap<BlobId, Vec<u8>>,
    old: &Snapshot,
    new: &Snapshot,
) -> Result<TextDiff, GitError> {
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
    changes.sort_by(|a, b| a.path().cmp(b.path()));

    let mut out = SnapshotDiff {
        renames: RenameMap(renames.into_iter().collect()),
        ..Default::default()
    };
    for change in &changes {
        let classified = classify(change, &contents);
        out.non_text.extend(classified.non_text);
        if let Some(text) = classified.text {
            out.files.push(FileDiff {
                old_path: text.old.map(|(p, _)| p.clone()),
                new_path: text.new.map(|(p, _)| p.clone()),
                old_blob: text.old.map(|(_, b)| b),
                new_blob: text.new.map(|(_, b)| b),
                hunks: line_hunks(text.before, text.after),
            });
        }
    }
    let texts: BTreeSet<BlobId> = (out.files.iter())
        .flat_map(|f| [f.old_blob, f.new_blob])
        .flatten()
        .collect();
    contents.retain(|blob, _| texts.contains(blob));
    Ok(TextDiff {
        diff: out,
        texts: contents,
    })
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

/// Serves blob contents the rewrite tracker asks for. What it does not hold
/// (worktree-only blobs) falls back to the object database, which is where
/// the attribute stack finds the repository's `.gitattributes` files.
struct Contents<'a> {
    held: HashMap<ObjectId, &'a [u8]>,
    repo: &'a gix::Repository,
}

impl Find for Contents<'_> {
    fn try_find<'a>(&self, id: &gix::oid, buffer: &'a mut Vec<u8>) -> gix::ExnResult<Option<gix::objs::Data<'a>>> {
        let Some(bytes) = self.held.get(id) else {
            return self.repo.objects.try_find(id, buffer);
        };
        buffer.clear();
        buffer.extend_from_slice(bytes);
        Ok(Some(gix::objs::Data::new(buffer, Kind::Blob, gix::hash::Kind::Sha1)))
    }
}

impl FindHeader for Contents<'_> {
    fn try_header(&self, id: &gix::oid) -> gix::ExnResult<Option<gix::objs::Header>> {
        match self.held.get(id) {
            Some(bytes) => Ok(Some(gix::objs::Header {
                kind: Kind::Blob,
                size: bytes.len() as u64,
            })),
            None => self.repo.objects.try_header(id),
        }
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
            &Contents {
                held: git_contents,
                repo,
            },
            |_| Ok::<(), Infallible>(()),
        )
        .map_err(err)?;
    Ok(found)
}
