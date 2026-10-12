//! Patch ids, and pairing the units of two versions of a changeset the way `git range-diff`
//! pairs commits (design-changesets §3.3).
//!
//! A unit is described by plain data: its patch, its subject and its author.

use similar::{Algorithm, DiffOp, capture_diff_slices};

use crate::content::Hash;
use crate::hunks::changed_lines;

/// A file's content before and after a unit.
#[derive(Clone, Copy, Debug)]
pub struct PatchFile<'a> {
    pub path: &'a [u8],
    pub old: &'a [u8],
    pub new: &'a [u8],
}

/// Hash of a patch's changed lines, without line numbers, context or whitespace, so a patch
/// keeps its id when the base moves and its lines shift or the code around it changes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PatchId(pub Hash);

/// A patch normalized for comparison.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Patch {
    id: PatchId,
    /// A header per file, then its removed and added lines, whitespace stripped.
    lines: Vec<Vec<u8>>,
}

impl Patch {
    /// The patch of `files`, in any order. Files without a change do not count.
    pub fn new(files: &[PatchFile]) -> Self {
        let mut files: Vec<_> = files.iter().filter(|f| f.old != f.new).collect();
        files.sort_by_key(|f| f.path);
        let mut lines = Vec::new();
        for f in files {
            lines.push([b"file ", f.path].concat());
            let strip = |l: &[u8]| l.trim_ascii().to_vec();
            let (old, new) = (split(f.old), split(f.new));
            for h in changed_lines(f.old, f.new) {
                let removed = &old[h.old.start as usize..h.old.end as usize];
                let added = &new[h.new.start as usize..h.new.end as usize];
                lines.extend(removed.iter().map(|l| [b"-".as_slice(), &strip(l)].concat()));
                lines.extend(added.iter().map(|l| [b"+".as_slice(), &strip(l)].concat()));
            }
        }
        let mut hasher = blake3::Hasher::new();
        for l in &lines {
            hasher.update(&(l.len() as u64).to_le_bytes());
            hasher.update(l);
        }
        Self {
            id: PatchId(Hash(*hasher.finalize().as_bytes())),
            lines,
        }
    }

    pub fn id(&self) -> PatchId {
        self.id
    }

    /// How many lines the patch consists of.
    fn size(&self) -> usize {
        self.lines.len()
    }

    /// How many lines differ between the two patches.
    fn distance(&self, other: &Self) -> usize {
        capture_diff_slices(Algorithm::Myers, &self.lines, &other.lines)
            .iter()
            .map(|op| match *op {
                DiffOp::Equal { .. } => 0,
                DiffOp::Delete { old_len, .. } => old_len,
                DiffOp::Insert { new_len, .. } => new_len,
                DiffOp::Replace { old_len, new_len, .. } => old_len + new_len,
            })
            .sum()
    }
}

fn split(text: &[u8]) -> Vec<&[u8]> {
    text.split_inclusive(|&b| b == b'\n').collect()
}

/// The patch id of `files`.
pub fn patch_id(files: &[PatchFile]) -> PatchId {
    Patch::new(files).id()
}

/// A unit of a version: a commit, or a squash.
#[derive(Clone, Copy, Debug)]
pub struct PairUnit<'a> {
    pub patch: &'a Patch,
    pub subject: &'a str,
    pub author: &'a str,
}

/// Why two units are paired.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PairedBy {
    /// The same patch.
    PatchId,
    /// The same subject and author, the patch changed.
    Message,
    /// The patches are similar enough.
    Similarity,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pair {
    Matched {
        a: usize,
        b: usize,
        by: PairedBy,
    },
    /// In the old version only.
    Dropped {
        a: usize,
    },
    /// In the new version only.
    Added {
        b: usize,
    },
}

/// How much of a patch's size it costs to leave it unpaired, in percent: units pair only when
/// their patches differ by less than the two patches would cost unpaired (`git range-diff
/// --creation-factor`).
const CREATION_FACTOR: u64 = 60;

/// Pairs the units of the old version `a` with those of the new version `b`: by patch id, then
/// by subject and author, then the rest so that the patches differ as little as possible overall
/// and by less than [`CREATION_FACTOR`] allows. Each unit is in one pair. The result is in the
/// order `git range-diff` lists them in: dropped units where they were, the others in the new
/// version's order.
pub fn pair_units(a: &[PairUnit], b: &[PairUnit]) -> Vec<Pair> {
    let mut a_to_b: Vec<Option<(usize, PairedBy)>> = vec![None; a.len()];
    let mut b_taken = vec![false; b.len()];

    let mut claim = |by: PairedBy, same: &dyn Fn(&PairUnit, &PairUnit) -> bool| {
        for (i, ua) in a.iter().enumerate() {
            if a_to_b[i].is_some() {
                continue;
            }
            if let Some(j) = (0..b.len()).find(|&j| !b_taken[j] && same(ua, &b[j])) {
                a_to_b[i] = Some((j, by));
                b_taken[j] = true;
            }
        }
    };
    claim(PairedBy::PatchId, &|x, y| x.patch.id() == y.patch.id());
    claim(PairedBy::Message, &|x, y| {
        x.subject == y.subject && x.author == y.author
    });

    let free_a: Vec<usize> = (0..a.len()).filter(|&i| a_to_b[i].is_none()).collect();
    let free_b: Vec<usize> = (0..b.len()).filter(|&j| !b_taken[j]).collect();
    for (i, j) in assign(a, b, &free_a, &free_b) {
        a_to_b[i] = Some((j, PairedBy::Similarity));
        b_taken[j] = true;
    }

    let mut b_to_a = vec![None; b.len()];
    for (i, m) in a_to_b.iter().enumerate() {
        if let Some((j, by)) = *m {
            b_to_a[j] = Some((i, by));
        }
    }

    let mut out = Vec::new();
    let mut shown = vec![false; a.len()];
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && a_to_b[i].is_none() {
            out.push(Pair::Dropped { a: i });
            i += 1;
        } else if j < b.len() && b_to_a[j].is_none() {
            out.push(Pair::Added { b: j });
            j += 1;
        } else if i < a.len() && shown[i] {
            i += 1;
        } else if i < a.len() && a_to_b[i].is_some_and(|(m, _)| m == j) {
            let (_, by) = a_to_b[i].unwrap();
            out.push(Pair::Matched { a: i, b: j, by });
            shown[i] = true;
            i += 1;
            j += 1;
        } else if let Some((m, by)) = b_to_a.get(j).copied().flatten() {
            out.push(Pair::Matched { a: m, b: j, by });
            shown[m] = true;
            j += 1;
        } else {
            i += 1;
        }
    }
    out
}

/// The cheapest pairing of `free_a` with `free_b`, as an assignment problem where leaving a unit
/// unpaired costs [`CREATION_FACTOR`] percent of its size.
fn assign(a: &[PairUnit], b: &[PairUnit], free_a: &[usize], free_b: &[usize]) -> Vec<(usize, usize)> {
    let n = free_a.len() + free_b.len();
    if free_a.is_empty() || free_b.is_empty() {
        return Vec::new();
    }
    // Rows are the free old units then placeholders for creations, columns the free new units
    // then placeholders for drops.
    let cost: Vec<Vec<u64>> = (0..n)
        .map(|r| {
            (0..n)
                .map(|c| match (free_a.get(r), free_b.get(c)) {
                    (Some(&i), Some(&j)) => a[i].patch.distance(b[j].patch) as u64 * 100,
                    (Some(&i), None) => a[i].patch.size() as u64 * CREATION_FACTOR,
                    (None, Some(&j)) => b[j].patch.size() as u64 * CREATION_FACTOR,
                    (None, None) => 0,
                })
                .collect()
        })
        .collect();
    hungarian(&cost)
        .into_iter()
        .enumerate()
        .filter(|&(r, c)| r < free_a.len() && c < free_b.len())
        .map(|(r, c)| (free_a[r], free_b[c]))
        .collect()
}

/// The column assigned to each row of a square cost matrix at the least total cost.
fn hungarian(cost: &[Vec<u64>]) -> Vec<usize> {
    let n = cost.len();
    let inf = i64::MAX / 4;
    // Potentials and the matching, 1-based with a dummy row 0.
    let (mut u, mut v) = (vec![0i64; n + 1], vec![0i64; n + 1]);
    let (mut matched_row, mut way) = (vec![0usize; n + 1], vec![0usize; n + 1]);
    for row in 1..=n {
        matched_row[0] = row;
        let mut col0 = 0;
        let mut min_v = vec![inf; n + 1];
        let mut used = vec![false; n + 1];
        loop {
            used[col0] = true;
            let (row0, mut delta, mut col1) = (matched_row[col0], inf, 0);
            for col in 1..=n {
                if used[col] {
                    continue;
                }
                let cur = cost[row0 - 1][col - 1] as i64 - u[row0] - v[col];
                if cur < min_v[col] {
                    min_v[col] = cur;
                    way[col] = col0;
                }
                if min_v[col] < delta {
                    delta = min_v[col];
                    col1 = col;
                }
            }
            for col in 0..=n {
                if used[col] {
                    u[matched_row[col]] += delta;
                    v[col] -= delta;
                } else {
                    min_v[col] -= delta;
                }
            }
            col0 = col1;
            if matched_row[col0] == 0 {
                break;
            }
        }
        loop {
            let col1 = way[col0];
            matched_row[col0] = matched_row[col1];
            col0 = col1;
            if col0 == 0 {
                break;
            }
        }
    }
    let mut column_of = vec![0; n];
    for col in 1..=n {
        column_of[matched_row[col] - 1] = col - 1;
    }
    column_of
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(path: &str, old: &str, new: &str) -> Patch {
        Patch::new(&[PatchFile {
            path: path.as_bytes(),
            old: old.as_bytes(),
            new: new.as_bytes(),
        }])
    }

    #[test]
    fn a_patch_keeps_its_id_when_the_base_moves_around_it() {
        let before = patch("f.rs", "a\nb\nc\n", "a\nB\nc\n");
        let rebased = patch("f.rs", "new top\n\na\nb\nc\nd\n", "new top\n\na\nB\nc\nd\n");
        assert_eq!(before.id(), rebased.id());
    }

    #[test]
    fn whitespace_does_not_change_a_patch_id() {
        assert_eq!(patch("f", "x\n", "  y  \n").id(), patch("f", "x\n", "y\n").id());
    }

    #[test]
    fn another_change_or_another_file_decides_the_id_but_the_order_of_files_does_not() {
        let base = patch("f", "x\n", "y\n").id();
        assert_ne!(base, patch("f", "x\n", "z\n").id());
        assert_ne!(base, patch("g", "x\n", "y\n").id());
        let (f, g) = (("f", "1\n", "2\n"), ("g", "3\n", "4\n"));
        let files = |order: [(&'static str, &'static str, &'static str); 2]| {
            let files: Vec<_> = order
                .iter()
                .map(|&(path, old, new)| PatchFile {
                    path: path.as_bytes(),
                    old: old.as_bytes(),
                    new: new.as_bytes(),
                })
                .collect();
            patch_id(&files)
        };
        assert_eq!(files([f, g]), files([g, f]));
    }

    /// A commit's patch that adds `n` distinct lines to `file`, tagged by `tag`.
    fn adding(file: &str, tag: &str, n: usize) -> Patch {
        let new: String = (0..n).map(|i| format!("{tag} {i}\n")).collect();
        patch(file, "", &new)
    }

    fn unit<'a>(patch: &'a Patch, subject: &'a str, author: &'a str) -> PairUnit<'a> {
        PairUnit { patch, subject, author }
    }

    fn matched(a: usize, b: usize, by: PairedBy) -> Pair {
        Pair::Matched { a, b, by }
    }

    #[test]
    fn reordered_commits_pair_by_patch() {
        let (p1, p2, p3) = (adding("a", "one", 5), adding("b", "two", 5), adding("c", "three", 5));
        let old = [unit(&p1, "one", "x"), unit(&p2, "two", "x"), unit(&p3, "three", "x")];
        let new = [unit(&p3, "three", "x"), unit(&p1, "one", "x"), unit(&p2, "two", "x")];
        let pairs = pair_units(&old, &new);
        let by = PairedBy::PatchId;
        let mut sorted = pairs.clone();
        sorted.sort_by_key(|p| match p {
            Pair::Matched { a, .. } => *a,
            _ => usize::MAX,
        });
        assert_eq!(sorted, [matched(0, 1, by), matched(1, 2, by), matched(2, 0, by)]);
        assert_eq!(pairs.len(), 3);
    }

    #[test]
    fn a_reworded_commit_pairs_by_patch_whatever_the_subject() {
        let p = adding("a", "one", 5);
        let pairs = pair_units(&[unit(&p, "wip", "x")], &[unit(&p, "Add one", "x")]);
        assert_eq!(pairs, [matched(0, 0, PairedBy::PatchId)]);
    }

    #[test]
    fn an_amended_commit_pairs_by_subject_and_author_even_if_the_patch_is_all_new() {
        let (before, after) = (adding("a", "old", 5), adding("z", "completely new", 7));
        let pairs = pair_units(
            &[unit(&before, "Fix the parser", "ann")],
            &[unit(&after, "Fix the parser", "ann")],
        );
        assert_eq!(pairs, [matched(0, 0, PairedBy::Message)]);
        // Another author is another commit.
        let pairs = pair_units(
            &[unit(&before, "Fix the parser", "ann")],
            &[unit(&after, "Fix the parser", "bob")],
        );
        assert_eq!(pairs, [Pair::Dropped { a: 0 }, Pair::Added { b: 0 }]);
    }

    #[test]
    fn a_reworded_and_edited_commit_pairs_by_similarity() {
        let before = adding("a", "line", 10);
        let after = adding("a", "line", 11);
        let pairs = pair_units(&[unit(&before, "wip", "x")], &[unit(&after, "Add lines", "x")]);
        assert_eq!(pairs, [matched(0, 0, PairedBy::Similarity)]);
    }

    #[test]
    fn dissimilar_commits_are_a_dropped_and_an_added_one() {
        let (before, after) = (adding("a", "old", 10), adding("b", "new", 10));
        let pairs = pair_units(&[unit(&before, "one", "x")], &[unit(&after, "two", "x")]);
        assert_eq!(pairs, [Pair::Dropped { a: 0 }, Pair::Added { b: 0 }]);
    }

    #[test]
    fn dropped_and_added_commits_stay_where_they_were() {
        let (p1, p2, p3, p4) = (
            adding("a", "one", 5),
            adding("b", "two", 5),
            adding("c", "three", 5),
            adding("d", "four", 5),
        );
        let old = [unit(&p1, "one", "x"), unit(&p2, "two", "x"), unit(&p3, "three", "x")];
        let new = [unit(&p1, "one", "x"), unit(&p3, "three", "x"), unit(&p4, "four", "x")];
        assert_eq!(
            pair_units(&old, &new),
            [
                matched(0, 0, PairedBy::PatchId),
                Pair::Dropped { a: 1 },
                matched(2, 1, PairedBy::PatchId),
                Pair::Added { b: 2 },
            ]
        );
    }

    #[test]
    fn an_identical_patch_is_paired_before_similar_ones_compete_for_it() {
        // `old_a` resembles both new patches, but `old_b` is identical to the first.
        let near: String = (0..10).map(|i| format!("l{i}\n")).collect();
        let far: String = (0..8).map(|i| format!("l{i}\n")).collect();
        let old_a = patch("f", "", &near);
        let old_b = patch("f", "", &far);
        let new_a = patch("f", "", &far);
        let new_b = patch("f", "", &format!("{near}extra\n"));
        let pairs = pair_units(
            &[unit(&old_a, "1", "x"), unit(&old_b, "2", "x")],
            &[unit(&new_a, "3", "x"), unit(&new_b, "4", "x")],
        );
        // old_b and new_a are the same patch, so they pair by id and old_a takes new_b.
        assert!(pairs.contains(&matched(1, 0, PairedBy::PatchId)));
        assert!(pairs.contains(&matched(0, 1, PairedBy::Similarity)));
    }

    #[test]
    fn the_assignment_minimizes_the_total_cost_not_each_pick() {
        // Taking the cheapest cell first would leave row 1 the cell of 100.
        let cost = vec![vec![1, 2], vec![1, 100]];
        assert_eq!(hungarian(&cost), [1, 0]);
        let cost = vec![vec![4, 1, 3], vec![2, 0, 5], vec![3, 2, 2]];
        assert_eq!(hungarian(&cost), [1, 0, 2]);
    }
}
