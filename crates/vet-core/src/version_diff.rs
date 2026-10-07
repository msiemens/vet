//! The interdiff of two whole versions of a changeset (design-changesets §3.2): per path, what
//! the author changed between them with what a move of the base explains left out.
//!
//! Version A is rebased onto B's base entry by entry, and the result is compared with B's
//! target. Text goes through [`interdiff`]; an entry that is not text (mode, symlink, submodule,
//! binary) is compared as an entry: if A did not touch it, B's base entry stands in for it, else
//! A's own does.

use std::collections::{BTreeMap, BTreeSet};

use crate::classify::{Change, changes, classify};
use crate::content::{BlobId, EntryKind, FileDiff, FileEntry, LineDelta, RepoPath, Snapshot, SnapshotDiff, is_binary};
use crate::hunks::line_hunks;
use crate::interdiff::{FileInterdiff, InterdiffHunk, interdiff};

/// The four trees of two versions: each one's base and target.
#[derive(Clone, Copy)]
pub struct VersionSides<'a> {
    pub base_a: &'a Snapshot,
    pub target_a: &'a Snapshot,
    pub base_b: &'a Snapshot,
    pub target_b: &'a Snapshot,
}

impl VersionSides<'_> {
    /// The paths either version's changeset changes, which are the only ones that can differ
    /// between the versions in a way the author made. With one base for both, those are the
    /// paths the targets differ in.
    pub fn paths(&self) -> Vec<RepoPath> {
        if self.base_a.id == self.base_b.id {
            return changes(self.target_a, self.target_b)
                .iter()
                .map(|c| c.path().clone())
                .collect();
        }
        let mut paths = BTreeSet::new();
        for (base, target) in [(self.base_a, self.target_a), (self.base_b, self.target_b)] {
            paths.extend(changes(base, target).iter().map(|c| c.path().clone()));
        }
        paths.into_iter().collect()
    }

    /// The files of `paths` whose content [`version_diff`] reads, on any of the four sides.
    pub fn blobs(&self, paths: &[RepoPath]) -> Vec<(RepoPath, BlobId)> {
        let mut out = BTreeSet::new();
        for path in paths {
            for snapshot in [self.base_a, self.target_a, self.base_b, self.target_b] {
                if let Some(e) = snapshot.entries.get(path).filter(|e| e.kind == EntryKind::File) {
                    out.insert((path.clone(), e.blob));
                }
            }
        }
        out.into_iter().collect()
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct VersionDiff {
    /// A's target rebased onto B's base against B's target. A file's `old_blob` is in the blobs
    /// passed to [`version_diff`] (rebased content is added there).
    pub diff: SnapshotDiff,
    /// The hunks of each text file in `diff`, by its path, aligned with the file's own hunks.
    pub hunks: BTreeMap<RepoPath, Vec<InterdiffHunk>>,
}

impl VersionDiff {
    /// The files the interdiff changes, by path, with the lines they gain and lose. A file with
    /// no text lines to count (a mode change, a binary file) has none.
    pub fn changed_files(&self) -> Vec<(RepoPath, LineDelta)> {
        let mut files: BTreeMap<RepoPath, LineDelta> = BTreeMap::new();
        for file in &self.diff.files {
            files.insert(file.path().clone(), LineDelta::of(&file.hunks));
        }
        for item in &self.diff.non_text {
            files.entry(item.path.clone()).or_default();
        }
        files.into_iter().collect()
    }
}

/// The interdiff of `paths` (see [`VersionSides::paths`]). `blobs` holds the content of
/// [`VersionSides::blobs`] that is available and receives the rebased content of the old side.
pub fn version_diff(sides: VersionSides, paths: &[RepoPath], blobs: &mut BTreeMap<BlobId, Vec<u8>>) -> VersionDiff {
    let mut out = VersionDiff::default();
    for path in paths {
        let (base_a, target_a) = (sides.base_a.entries.get(path), sides.target_a.entries.get(path));
        let (base_b, target_b) = (sides.base_b.entries.get(path), sides.target_b.entries.get(path));

        let rebased = text_interdiff(blobs, [base_a, target_a, base_b, target_b]);
        let old = match &rebased {
            Some(fi) => rebased_entry(&fi.old, [base_a, target_a, base_b]),
            None if target_a == base_a => base_b.map(|e| (e.clone(), None)),
            None => target_a.map(|e| (e.clone(), None)),
        };
        if let Some((entry, Some(bytes))) = &old {
            blobs.entry(entry.blob).or_insert_with(|| bytes.clone());
        }
        let old = old.map(|(entry, _)| entry);
        if old.as_ref() == target_b {
            continue;
        }

        let change = Change {
            old: old.map(|e| (path.clone(), e)),
            new: target_b.map(|e| (path.clone(), e.clone())),
        };
        let classified = classify(&change, &*blobs);
        out.diff.non_text.extend(classified.non_text);
        if let Some(text) = classified.text {
            let hunks = match &rebased {
                Some(fi) => fi.hunks.clone(),
                // Without a rebase every line of both sides counts as the author's.
                None => (line_hunks(text.before, text.after).into_iter())
                    .map(|hunk| InterdiffHunk {
                        hunk,
                        flags: Default::default(),
                        target_lines: vec![hunk.new].into_iter().filter(|r| !r.is_empty()).collect(),
                        base_lines: vec![hunk.old].into_iter().filter(|r| !r.is_empty()).collect(),
                    })
                    .collect(),
            };
            let file = FileDiff {
                old_path: text.old.map(|(p, _)| p.clone()),
                new_path: text.new.map(|(p, _)| p.clone()),
                old_blob: text.old.map(|(_, b)| b),
                new_blob: text.new.map(|(_, b)| b),
                hunks: hunks.iter().map(|h| h.hunk).collect(),
            };
            out.hunks.insert(file.path().clone(), hunks);
            out.diff.files.push(file);
        }
    }
    out
}

/// The text interdiff, if every side that exists is a text file whose content is available.
fn text_interdiff(blobs: &BTreeMap<BlobId, Vec<u8>>, sides: [Option<&FileEntry>; 4]) -> Option<FileInterdiff> {
    let [base_a, target_a, base_b, target_b] = sides.map(|side| match side {
        None => Some(&[][..]),
        Some(e) if e.kind == EntryKind::File => blobs.get(&e.blob).map(Vec::as_slice).filter(|b| !is_binary(b)),
        Some(_) => None,
    });
    Some(interdiff(base_a?, target_a?, base_b?, target_b?))
}

/// A's entry rebased onto B's base, with the content to add when it is new. `None` if A's file
/// does not exist there: A deleted it, or never had it and B's base does not either.
fn rebased_entry(
    text: &[u8],
    [base_a, target_a, base_b]: [Option<&FileEntry>; 3],
) -> Option<(FileEntry, Option<Vec<u8>>)> {
    if target_a == base_a {
        return base_b.map(|e| (e.clone(), None));
    }
    // A deleted the file, and nothing of it survives the rebase.
    if target_a.is_none() && text.is_empty() {
        return None;
    }
    // The mode is A's if A changed it, else the new base's.
    let mode = match (target_a, base_a, base_b) {
        (Some(t), Some(b), Some(n)) if t.mode == b.mode => n.mode,
        (Some(t), _, _) => t.mode,
        (None, _, n) => n.or(base_a).map_or(0o100644, |e| e.mode),
    };
    let unfiltered = target_a.is_some_and(|e| e.unfiltered);
    let entry = FileEntry {
        blob: BlobId::of(text),
        mode,
        kind: EntryKind::File,
        unfiltered,
    };
    Some((entry, Some(text.to_vec())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{LineRange, NonTextKind};
    use crate::merkle;

    fn entry(content: &str, mode: u32, kind: EntryKind) -> FileEntry {
        FileEntry {
            blob: BlobId::of(content.as_bytes()),
            mode,
            kind,
            unfiltered: false,
        }
    }

    fn file(content: &str) -> FileEntry {
        entry(content, 0o100644, EntryKind::File)
    }

    /// A tree of `(path, entry)` and the content of all its files.
    struct Tree(Snapshot);

    fn tree(files: &[(&str, FileEntry)]) -> Tree {
        let entries: BTreeMap<RepoPath, FileEntry> =
            files.iter().map(|(p, e)| (RepoPath::from(*p), e.clone())).collect();
        Tree(Snapshot {
            id: merkle::snapshot_id(&entries),
            entries,
        })
    }

    fn lines(prefix: &str, n: usize) -> String {
        (0..n).map(|i| format!("{prefix}{i}\n")).collect()
    }

    fn run(contents: &[&str], base_a: &Tree, target_a: &Tree, base_b: &Tree, target_b: &Tree) -> VersionDiff {
        let sides = VersionSides {
            base_a: &base_a.0,
            target_a: &target_a.0,
            base_b: &base_b.0,
            target_b: &target_b.0,
        };
        let mut blobs: BTreeMap<BlobId, Vec<u8>> = contents
            .iter()
            .map(|c| (BlobId::of(c.as_bytes()), c.as_bytes().to_vec()))
            .collect();
        version_diff(sides, &sides.paths(), &mut blobs)
    }

    #[test]
    fn what_the_rebase_brought_is_left_out() {
        let old_base = lines("l", 20);
        let new_base = old_base.replace("l2\n", "upstream\n");
        let mine_a = old_base.replace("l15\n", "mine\n");
        let mine_b = new_base.replace("l15\n", "mine\n");
        let contents = [&old_base, &new_base, &mine_a, &mine_b].map(String::as_str);
        let (base_a, base_b) = (tree(&[("f", file(&old_base))]), tree(&[("f", file(&new_base))]));
        let (target_a, target_b) = (tree(&[("f", file(&mine_a))]), tree(&[("f", file(&mine_b))]));

        let result = run(&contents, &base_a, &target_a, &base_b, &target_b);

        assert!(result.diff.files.is_empty() && result.diff.non_text.is_empty());
    }

    #[test]
    fn a_file_the_base_move_changed_and_the_author_did_not_is_in_neither_side() {
        let old_base = lines("l", 6);
        let new_base = old_base.replace("l2\n", "upstream\n");
        let contents = [&old_base, &new_base].map(String::as_str);
        let (base_a, base_b) = (tree(&[("f", file(&old_base))]), tree(&[("f", file(&new_base))]));
        let (target_a, target_b) = (tree(&[("f", file(&old_base))]), tree(&[("f", file(&new_base))]));

        let result = run(&contents, &base_a, &target_a, &base_b, &target_b);

        assert_eq!(
            VersionSides {
                base_a: &base_a.0,
                target_a: &target_a.0,
                base_b: &base_b.0,
                target_b: &target_b.0
            }
            .paths(),
            []
        );
        assert!(result.diff.files.is_empty());
    }

    #[test]
    fn a_change_the_author_made_between_versions_is_shown_against_the_rebased_old_side() {
        let old_base = lines("l", 20);
        let new_base = old_base.replace("l2\n", "upstream\n");
        let mine_a = old_base.replace("l15\n", "mine\n");
        let mine_b = new_base.replace("l15\n", "mine, better\n");
        let rebased = new_base.replace("l15\n", "mine\n");
        let contents = [&old_base, &new_base, &mine_a, &mine_b].map(String::as_str);
        let (base_a, base_b) = (tree(&[("f", file(&old_base))]), tree(&[("f", file(&new_base))]));
        let (target_a, target_b) = (tree(&[("f", file(&mine_a))]), tree(&[("f", file(&mine_b))]));

        let mut blobs: BTreeMap<_, _> = contents
            .iter()
            .map(|c| (BlobId::of(c.as_bytes()), c.as_bytes().to_vec()))
            .collect();
        let sides = VersionSides {
            base_a: &base_a.0,
            target_a: &target_a.0,
            base_b: &base_b.0,
            target_b: &target_b.0,
        };
        let result = version_diff(sides, &sides.paths(), &mut blobs);

        let [diff] = &result.diff.files[..] else {
            panic!("{result:?}")
        };
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].new, LineRange { start: 15, end: 16 });
        assert_eq!(
            blobs[&diff.old_blob.unwrap()],
            rebased.as_bytes(),
            "the old side is version A as if made on the new base"
        );
        assert_eq!(diff.new_blob, Some(BlobId::of(mine_b.as_bytes())));
        assert_eq!(result.hunks[diff.path()].len(), 1);
        assert!(!result.hunks[diff.path()][0].flags.conflict);
    }

    #[test]
    fn a_patch_that_does_not_apply_to_the_new_base_is_flagged() {
        let old_base = lines("l", 20);
        let new_base = old_base.replace("l10\n", "upstream\n");
        let mine_a = old_base.replace("l10\n", "mine\n");
        let mine_b = new_base.replace("upstream\n", "mine, redone\n");
        let contents = [&old_base, &new_base, &mine_a, &mine_b].map(String::as_str);
        let (base_a, base_b) = (tree(&[("f", file(&old_base))]), tree(&[("f", file(&new_base))]));
        let (target_a, target_b) = (tree(&[("f", file(&mine_a))]), tree(&[("f", file(&mine_b))]));

        let result = run(&contents, &base_a, &target_a, &base_b, &target_b);

        let hunks = &result.hunks[&RepoPath::from("f")];
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].flags.conflict);
    }

    #[test]
    fn a_change_the_author_took_back_is_informational() {
        let base = lines("l", 10);
        let mine = base.replace("l4\n", "mine\n");
        let contents = [&base, &mine].map(String::as_str);
        let (base_t, mine_t) = (tree(&[("f", file(&base))]), tree(&[("f", file(&mine))]));

        let result = run(&contents, &base_t, &mine_t, &base_t, &base_t);

        let hunks = &result.hunks[&RepoPath::from("f")];
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].flags.informational);
    }

    #[test]
    fn a_file_the_author_added_between_versions_is_all_new() {
        let base = lines("l", 4);
        let created = "created\n";
        let contents = [&base[..], created];
        let (base_t, with_new) = (
            tree(&[("f", file(&base))]),
            tree(&[("f", file(&base)), ("g", file(created))]),
        );

        let result = run(&contents, &base_t, &base_t, &base_t, &with_new);

        let [diff] = &result.diff.files[..] else {
            panic!("{result:?}")
        };
        assert_eq!(
            (diff.old_path.clone(), diff.new_path.clone()),
            (None, Some(RepoPath::from("g")))
        );
        assert_eq!(
            result.hunks[diff.path()][0].target_lines,
            [LineRange { start: 0, end: 1 }]
        );
    }

    #[test]
    fn a_mode_the_base_changed_is_not_the_authors_but_one_the_author_changed_is() {
        let script = "echo\n";
        let plain = entry(script, 0o100644, EntryKind::File);
        let exec = entry(script, 0o100755, EntryKind::File);
        let contents = [script];

        // Upstream made the file executable; the author never touched it.
        let (base_a, base_b) = (tree(&[("run", plain.clone())]), tree(&[("run", exec.clone())]));
        let result = run(&contents, &base_a, &base_a, &base_b, &base_b);
        assert!(result.diff.non_text.is_empty() && result.diff.files.is_empty());

        // The author makes it executable in version B.
        let result = run(&contents, &base_a, &base_a, &base_a, &tree(&[("run", exec)]));
        let [item] = &result.diff.non_text[..] else {
            panic!("{result:?}")
        };
        assert_eq!(item.kind, NonTextKind::Mode);
    }

    #[test]
    fn the_files_a_version_changed_are_listed_with_their_lines_and_a_mode_change_with_none() {
        let before = lines("l", 10);
        let edited = before.replace("l3\n", "a\nb\n").replace("l8\n", "");
        let added = "new\nfile\n";
        let script = "echo\n";
        let (plain, exec) = (
            entry(script, 0o100644, EntryKind::File),
            entry(script, 0o100755, EntryKind::File),
        );
        let contents = [&before[..], &edited, added, script];
        let base = tree(&[("keep", file(&before)), ("edit", file(&before)), ("run", plain)]);
        let now = tree(&[
            ("keep", file(&before)),
            ("edit", file(&edited)),
            ("added", file(added)),
            ("run", exec),
        ]);

        let result = run(&contents, &base, &base, &base, &now);

        assert_eq!(
            result.changed_files(),
            [
                (RepoPath::from("added"), LineDelta { added: 2, removed: 0 }),
                (RepoPath::from("edit"), LineDelta { added: 2, removed: 2 }),
                (RepoPath::from("run"), LineDelta::default()),
            ]
        );
    }

    #[test]
    fn with_one_base_only_the_files_the_targets_differ_in_are_looked_at() {
        let text = lines("l", 4);
        let other = lines("m", 4);
        let (base, a, b) = (
            tree(&[("same", file(&text)), ("moved", file(&text))]),
            tree(&[("same", file(&other)), ("moved", file(&text))]),
            tree(&[("same", file(&other)), ("moved", file(&other))]),
        );
        let sides = VersionSides {
            base_a: &base.0,
            target_a: &a.0,
            base_b: &base.0,
            target_b: &b.0,
        };

        assert_eq!(sides.paths(), [RepoPath::from("moved")]);
    }

    #[test]
    fn symlinks_and_binaries_are_compared_by_entry() {
        let link = |target: &str| entry(target, 0o120000, EntryKind::Symlink);
        let old_base = tree(&[("l", link("a"))]);
        let new_base = tree(&[("l", link("b"))]);
        let author = tree(&[("l", link("c"))]);
        let contents = ["a", "b", "c"];

        // A retargeted the link, the rebase brought a different target, and B kept A's.
        let kept = run(&contents, &old_base, &author, &new_base, &author);
        assert!(kept.diff.non_text.is_empty(), "same entry on both sides");

        let changed = run(&contents, &old_base, &author, &old_base, &tree(&[("l", link("b"))]));
        assert_eq!(changed.diff.non_text.len(), 1);
        assert_eq!(changed.diff.non_text[0].kind, NonTextKind::Symlink);

        let blob = |bytes: &str| file(bytes);
        let binary = ["a\0b", "a\0c"];
        let before = tree(&[("img", blob(binary[0]))]);
        let after = tree(&[("img", blob(binary[1]))]);
        let result = run(&binary, &before, &before, &before, &after);
        assert_eq!(result.diff.non_text[0].kind, NonTextKind::Binary);
    }
}
