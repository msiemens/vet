//! Line splitting, range hashing and exact line mapping between two texts.

use std::collections::HashMap;
use std::sync::OnceLock;

use similar::{Algorithm, DiffOp, capture_diff_slices};

use crate::content::{Hash, LineRange, RepoPath, SnapshotId};
use crate::review::{Anchor, Side};

/// Lines of context hashed on each side of an anchor.
pub(crate) const CONTEXT: usize = 3;
/// Context entry for a line beyond the start or end of the file.
pub(crate) const NO_LINE: Hash = Hash([0; 32]);

/// A text split into lines. `raw` keeps line terminators; `norm` drops trailing
/// whitespace, which includes `\r` and `\n`.
pub(crate) struct Text<'a> {
    pub raw: Vec<&'a [u8]>,
    pub norm: Vec<&'a [u8]>,
    hashes: OnceLock<Vec<Hash>>,
}

impl<'a> Text<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        let raw: Vec<_> = bytes.split_inclusive(|&b| b == b'\n').collect();
        let norm = raw.iter().map(|l| l.trim_ascii_end()).collect();
        Self {
            raw,
            norm,
            hashes: OnceLock::new(),
        }
    }

    /// Hash of each normalized line, computed on first use.
    pub fn hashes(&self) -> &[Hash] {
        self.hashes
            .get_or_init(|| self.norm.iter().map(|l| Hash::of(l)).collect())
    }

    fn in_bounds(&self, r: LineRange) -> bool {
        r.start < r.end && r.end as usize <= self.raw.len()
    }

    pub fn raw_hash(&self, r: LineRange) -> Option<Hash> {
        self.in_bounds(r).then(|| {
            let mut h = blake3::Hasher::new();
            for l in &self.raw[r.start as usize..r.end as usize] {
                h.update(l);
            }
            Hash(*h.finalize().as_bytes())
        })
    }

    /// Hash of the normalized line hashes of `r`.
    pub fn norm_hash(&self, r: LineRange) -> Option<Hash> {
        self.in_bounds(r).then(|| {
            let mut h = blake3::Hasher::new();
            for l in &self.hashes()[r.start as usize..r.end as usize] {
                h.update(&l.0);
            }
            Hash(*h.finalize().as_bytes())
        })
    }

    /// `CONTEXT` lines before, then `CONTEXT` lines after, normalized.
    pub fn context(&self, r: LineRange) -> Vec<Hash> {
        let hashes = self.hashes();
        let at = |i: Option<usize>| i.and_then(|i| hashes.get(i)).copied().unwrap_or(NO_LINE);
        let (s, e) = (r.start as usize, r.end as usize);
        (1..=CONTEXT)
            .rev()
            .map(|k| at(s.checked_sub(k)))
            .chain((0..CONTEXT).map(|k| at(Some(e + k))))
            .collect()
    }

    pub fn anchor(&self, side: Side, snapshot: SnapshotId, file: RepoPath, range: LineRange) -> Option<Anchor> {
        Some(Anchor {
            side,
            snapshot,
            file,
            range,
            content_hash_raw: self.raw_hash(range)?,
            content_hash_norm: self.norm_hash(range)?,
            context_hashes: self.context(range),
        })
    }
}

/// Old line index to new line index for every line the diff leaves unchanged.
pub(crate) fn diff_map(old: &[&[u8]], new: &[&[u8]]) -> Vec<Option<usize>> {
    let mut map = vec![None; old.len()];
    for op in capture_diff_slices(Algorithm::Myers, old, new) {
        if let DiffOp::Equal {
            old_index,
            new_index,
            len,
        } = op
        {
            for k in 0..len {
                map[old_index + k] = Some(new_index + k);
            }
        }
    }
    map
}

/// Like [`diff_map`], but drops mappings the diff chose between equal
/// alternatives: a line stays mapped only if its content is unique in both texts
/// or both its neighbours map to the new line's neighbours.
pub(crate) fn unique_map(old: &[&[u8]], new: &[&[u8]]) -> Vec<Option<usize>> {
    let map = diff_map(old, new);
    fn count<'a>(lines: &[&'a [u8]]) -> HashMap<&'a [u8], u32> {
        let mut c = HashMap::new();
        for l in lines {
            *c.entry(*l).or_default() += 1;
        }
        c
    }
    let (co, cn) = (count(old), count(new));
    let stable = |i: usize, j: usize| {
        let before = if i == 0 {
            j == 0
        } else {
            j > 0 && map[i - 1] == Some(j - 1)
        };
        let after = if i + 1 == old.len() {
            j + 1 == new.len()
        } else {
            map[i + 1] == Some(j + 1)
        };
        before && after
    };
    map.iter()
        .enumerate()
        .map(|(i, &j)| j.filter(|&j| (co[old[i]] == 1 && cn[new[j]] == 1) || stable(i, j)))
        .collect()
}
