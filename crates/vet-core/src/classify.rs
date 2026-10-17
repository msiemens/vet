//! Which changes of a snapshot diff have no text to review (§5.2): mode, symlink,
//! submodule, binary, content unavailable, pure rename and empty file. This is
//! pure logic over the entries of both sides and the blobs' bytes; finding
//! renames and the line hunks of the text files are the diff engine's.

use crate::content::{
    BlobId, BlobSource, EntryKind, FileEntry, NonTextChange, NonTextKind, RepoPath, Snapshot, is_binary,
};

type Side = Option<(RepoPath, FileEntry)>;

/// One path's entry on each side; a rename pairs the old path with the new one.
#[derive(Clone, Debug)]
pub struct Change {
    pub old: Side,
    pub new: Side,
}

impl Change {
    /// The new path, or the old one of a deletion.
    pub fn path(&self) -> &RepoPath {
        &self.new.as_ref().or(self.old.as_ref()).expect("a change has a side").0
    }
}

/// Entries that differ between the snapshots, by path.
pub fn changes(old: &Snapshot, new: &Snapshot) -> Vec<Change> {
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
pub fn text_blobs(changes: &[Change]) -> Vec<(RepoPath, BlobId)> {
    changes
        .iter()
        .flat_map(|c| [&c.old, &c.new])
        .flatten()
        .filter(|(_, e)| e.kind == EntryKind::File)
        .map(|(p, e)| (p.clone(), e.blob))
        .collect()
}

/// A change whose two sides are text and differ.
pub struct TextChange<'a> {
    pub old: Option<(&'a RepoPath, BlobId)>,
    pub new: Option<(&'a RepoPath, BlobId)>,
    /// The texts; empty for a side the file does not exist on.
    pub before: &'a [u8],
    pub after: &'a [u8],
}

/// What a change is: items without text, and, if the file has text to compare, that.
pub struct Classified<'a> {
    pub non_text: Vec<NonTextChange>,
    pub text: Option<TextChange<'a>>,
}

/// Sorts `change` into its non-text items and its text diff. A mode change of a
/// file whose text changed too is both. `blobs` has the content of every blob
/// that is available.
pub fn classify<'a>(change: &'a Change, blobs: &'a impl BlobSource) -> Classified<'a> {
    let item = |kind| NonTextChange {
        path: change.path().clone(),
        from: (kind == NonTextKind::Rename).then(|| change.old.as_ref().expect("a rename has both sides").0.clone()),
        kind,
        before: change.old.as_ref().map(|(_, e)| e.clone()),
        after: change.new.as_ref().map(|(_, e)| e.clone()),
    };
    let only = |kind| Classified {
        non_text: vec![item(kind)],
        text: None,
    };
    let kind_of = |s: &Side| s.as_ref().map(|(_, e)| e.kind);
    let (old_kind, new_kind) = (kind_of(&change.old), kind_of(&change.new));

    if old_kind == Some(EntryKind::Submodule) || new_kind == Some(EntryKind::Submodule) {
        return only(NonTextKind::Submodule);
    }
    if old_kind == Some(EntryKind::Symlink) || new_kind == Some(EntryKind::Symlink) {
        return only(NonTextKind::Symlink);
    }

    let mut non_text = Vec::new();
    if let (Some((old_path, o)), Some((new_path, n))) = (&change.old, &change.new) {
        if o.mode != n.mode {
            non_text.push(item(NonTextKind::Mode));
        }
        if o.blob == n.blob {
            if old_path != new_path {
                non_text.push(item(NonTextKind::Rename));
            }
            return Classified { non_text, text: None };
        }
    }

    let old = change.old.as_ref().map(|(p, e)| (p, e.blob));
    let new = change.new.as_ref().map(|(p, e)| (p, e.blob));
    if [old, new]
        .into_iter()
        .flatten()
        .any(|(_, blob)| blobs.known_binary(blob))
    {
        non_text.push(item(NonTextKind::Binary));
        return Classified { non_text, text: None };
    }
    let text = |side: Option<(&RepoPath, BlobId)>| match side {
        None => Some(&[][..]),
        Some((_, blob)) => blobs.blob(blob),
    };
    let (Some(before), Some(after)) = (text(old), text(new)) else {
        non_text.push(item(NonTextKind::ContentUnavailable));
        return Classified { non_text, text: None };
    };
    if is_binary(before) || is_binary(after) {
        non_text.push(item(NonTextKind::Binary));
        return Classified { non_text, text: None };
    }
    // Added or deleted empty: no lines on either side. A file with text on one side keeps its text diff.
    if before.is_empty() && after.is_empty() {
        non_text.push(item(NonTextKind::Empty));
        return Classified { non_text, text: None };
    }
    Classified {
        non_text,
        text: Some(TextChange {
            old,
            new,
            before,
            after,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::content::{Hash, WithBinary};

    fn path(p: &str) -> RepoPath {
        RepoPath::from(p)
    }

    fn entry(content: &[u8], mode: u32, kind: EntryKind) -> FileEntry {
        FileEntry {
            blob: BlobId::of(content),
            mode,
            kind,
            unfiltered: false,
        }
    }

    fn file(content: &[u8]) -> FileEntry {
        entry(content, 0o100644, EntryKind::File)
    }

    fn change(old: Option<(&str, FileEntry)>, new: Option<(&str, FileEntry)>) -> Change {
        Change {
            old: old.map(|(p, e)| (path(p), e)),
            new: new.map(|(p, e)| (path(p), e)),
        }
    }

    fn blobs(contents: &[&[u8]]) -> BTreeMap<BlobId, Vec<u8>> {
        contents.iter().map(|c| (BlobId::of(c), c.to_vec())).collect()
    }

    fn kinds(c: &Classified) -> Vec<NonTextKind> {
        c.non_text.iter().map(|n| n.kind).collect()
    }

    #[test]
    fn changed_text_is_a_text_change() {
        let blobs = blobs(&[b"a\n", b"b\n"]);
        let c = change(Some(("f", file(b"a\n"))), Some(("f", file(b"b\n"))));
        let classified = classify(&c, &blobs);
        assert!(classified.non_text.is_empty());
        let text = classified.text.unwrap();
        assert_eq!((text.before, text.after), (&b"a\n"[..], &b"b\n"[..]));
    }

    #[test]
    fn an_added_file_compares_against_nothing() {
        let blobs = blobs(&[b"a\n"]);
        let c = change(None, Some(("f", file(b"a\n"))));
        let text = classify(&c, &blobs).text.unwrap();
        assert!(text.old.is_none() && text.before.is_empty());
    }

    #[test]
    fn a_mode_change_with_the_same_text_is_only_a_mode_item() {
        let blobs = blobs(&[b"a\n"]);
        let c = change(
            Some(("f", file(b"a\n"))),
            Some(("f", entry(b"a\n", 0o100755, EntryKind::File))),
        );
        let classified = classify(&c, &blobs);
        assert_eq!(kinds(&classified), [NonTextKind::Mode]);
        assert!(classified.text.is_none());
    }

    #[test]
    fn a_mode_change_with_new_text_is_both() {
        let blobs = blobs(&[b"a\n", b"b\n"]);
        let c = change(
            Some(("f", file(b"a\n"))),
            Some(("f", entry(b"b\n", 0o100755, EntryKind::File))),
        );
        let classified = classify(&c, &blobs);
        assert_eq!(kinds(&classified), [NonTextKind::Mode]);
        assert!(classified.text.is_some());
    }

    #[test]
    fn the_same_content_at_a_new_path_is_a_rename_from_the_old_path() {
        let blobs = blobs(&[b"a\n"]);
        let c = change(Some(("old", file(b"a\n"))), Some(("new", file(b"a\n"))));
        let classified = classify(&c, &blobs);
        assert_eq!(kinds(&classified), [NonTextKind::Rename]);
        assert_eq!(classified.non_text[0].from, Some(path("old")));
        assert_eq!(classified.non_text[0].path, path("new"));
    }

    #[test]
    fn a_rename_with_edits_keeps_its_text_diff() {
        let blobs = blobs(&[b"a\n", b"b\n"]);
        let c = change(Some(("old", file(b"a\n"))), Some(("new", file(b"b\n"))));
        let classified = classify(&c, &blobs);
        assert!(classified.non_text.is_empty());
        assert!(classified.text.is_some());
    }

    #[test]
    fn symlinks_and_submodules_never_have_text() {
        let blobs = blobs(&[b"a\n"]);
        let link = entry(b"target", 0o120000, EntryKind::Symlink);
        let c = change(Some(("f", file(b"a\n"))), Some(("f", link)));
        assert_eq!(kinds(&classify(&c, &blobs)), [NonTextKind::Symlink]);

        let sub = entry(b"deadbeef", 0o160000, EntryKind::Submodule);
        let c = change(None, Some(("s", sub)));
        let classified = classify(&c, &blobs);
        assert_eq!(kinds(&classified), [NonTextKind::Submodule]);
        assert!(classified.text.is_none());
    }

    #[test]
    fn content_that_is_not_available_is_its_own_item() {
        let blobs = blobs(&[b"a\n"]);
        let c = change(Some(("f", file(b"a\n"))), Some(("f", file(b"b\n"))));
        let classified = classify(&c, &blobs);
        assert_eq!(kinds(&classified), [NonTextKind::ContentUnavailable]);
        assert!(classified.text.is_none());
    }

    #[test]
    fn binary_on_either_side_is_not_text() {
        let blobs = blobs(&[b"a\n", b"\0\x01"]);
        let c = change(Some(("f", file(b"a\n"))), Some(("f", file(b"\0\x01"))));
        assert_eq!(kinds(&classify(&c, &blobs)), [NonTextKind::Binary]);
    }

    #[test]
    fn a_binary_file_whose_bytes_were_not_kept_is_binary_not_unavailable() {
        let known = BTreeSet::from([BlobId::of(b"\0archive")]);
        let held = blobs(&[b"a\n"]);
        let source = WithBinary {
            bytes: &held,
            binary: &known,
        };
        let added = change(None, Some(("f", file(b"\0archive"))));
        assert_eq!(kinds(&classify(&added, &source)), [NonTextKind::Binary]);
        let from_text = change(Some(("f", file(b"a\n"))), Some(("f", file(b"\0archive"))));
        assert_eq!(kinds(&classify(&from_text, &source)), [NonTextKind::Binary]);
        let to_text = change(Some(("f", file(b"\0archive"))), Some(("f", file(b"b\n"))));
        assert_eq!(kinds(&classify(&to_text, &source)), [NonTextKind::Binary]);
        let unknown = change(None, Some(("f", file(b"b\n"))));
        assert_eq!(kinds(&classify(&unknown, &source)), [NonTextKind::ContentUnavailable]);
    }

    #[test]
    fn an_empty_file_added_or_deleted_has_no_lines_but_text_on_one_side_keeps_its_diff() {
        let blobs = blobs(&[b"", b"a\n"]);
        let added = change(None, Some(("f", file(b""))));
        assert_eq!(kinds(&classify(&added, &blobs)), [NonTextKind::Empty]);
        let deleted = change(Some(("f", file(b""))), None);
        assert_eq!(kinds(&classify(&deleted, &blobs)), [NonTextKind::Empty]);

        let emptied = change(Some(("f", file(b"a\n"))), Some(("f", file(b""))));
        assert!(classify(&emptied, &blobs).text.is_some());
    }

    #[test]
    fn only_differing_entries_are_changes_and_their_text_blobs_are_wanted() {
        let snapshot = |files: &[(&str, FileEntry)]| Snapshot {
            id: crate::content::SnapshotId(Hash([0; 32])),
            entries: files.iter().map(|(p, e)| (path(p), e.clone())).collect(),
        };
        let old = snapshot(&[("same", file(b"s")), ("edit", file(b"1")), ("gone", file(b"g"))]);
        let new = snapshot(&[("same", file(b"s")), ("edit", file(b"2")), ("new", file(b"n"))]);
        let found = changes(&old, &new);
        let paths: Vec<_> = found.iter().map(|c| c.path().clone()).collect();
        assert_eq!(paths, [path("edit"), path("gone"), path("new")]);
        let wanted: Vec<_> = text_blobs(&found).into_iter().map(|(_, b)| b).collect();
        assert_eq!(wanted.len(), 4);
    }
}
