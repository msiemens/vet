//! Line splitting and range hashing, and offsets in UTF-16 units.
//!
//! A text's offsets are UTF-16 units with `\r\n` counted as one unit, which is how an editor
//! counts its document, so a client applies offsets without looking at the text.

use std::sync::OnceLock;

use crate::content::{Hash, LineRange, RepoPath, SnapshotId};
use crate::review::{Anchor, Side};

/// Converts byte offsets of a text to offsets in units. Queries mostly ascend, so it walks forward
/// from the last one.
pub struct Units<'a> {
    text: &'a str,
    byte: usize,
    unit: u32,
}

impl<'a> Units<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text, byte: 0, unit: 0 }
    }

    /// The offset in units of the character containing `byte`, or the text's length.
    pub fn at(&mut self, byte: usize) -> u32 {
        if byte < self.byte {
            (self.byte, self.unit) = (0, 0);
        }
        let bytes = self.text.as_bytes();
        for c in self.text[self.byte..].chars() {
            if self.byte >= byte {
                break;
            }
            // The `\r` of `\r\n` is part of the break the `\n` counts.
            if !(c == '\r' && bytes.get(self.byte + 1) == Some(&b'\n')) {
                self.unit += c.len_utf16() as u32;
            }
            self.byte += c.len_utf8();
        }
        self.unit
    }
}

/// Length of `text` in units.
pub fn unit_len(text: &str) -> u32 {
    Units::new(text).at(text.len())
}

/// A text split into `\n`-terminated lines, with each line's byte and unit offset.
pub struct Lines<'a> {
    text: &'a str,
    bytes: Vec<usize>,
    units: Vec<u32>,
}

impl<'a> Lines<'a> {
    pub fn new(text: &'a str) -> Self {
        let (mut bytes, mut units) = (vec![0], vec![0]);
        for line in text.split_inclusive('\n') {
            bytes.push(bytes[bytes.len() - 1] + line.len());
            units.push(units[units.len() - 1] + unit_len(line));
        }
        Self { text, bytes, units }
    }

    pub fn text(&self) -> &'a str {
        self.text
    }

    /// The offset of each line's start, then the text's length. A line past the end of the text
    /// starts at its length.
    pub fn starts(&self) -> &[u32] {
        &self.units
    }

    /// The offset of each line's start in bytes, then the text's length, so that a line's text is
    /// `text[start..start_of_the_next]` without searching for it again.
    pub fn byte_starts(&self) -> &[usize] {
        &self.bytes
    }

    /// The offset of the start of `line`.
    pub fn offset(&self, line: u32) -> u32 {
        self.units[(line as usize).min(self.units.len() - 1)]
    }

    /// The text's length in units.
    pub fn total(&self) -> u32 {
        self.units[self.units.len() - 1]
    }

    /// The text of the lines in `range`.
    pub fn slice(&self, range: LineRange) -> &'a str {
        let at = |line: u32| self.bytes[(line as usize).min(self.bytes.len() - 1)];
        &self.text[at(range.start)..at(range.end)]
    }
}

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

#[cfg(test)]
thread_local! {
    /// How many texts this thread split into lines.
    pub(crate) static SPLITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl<'a> Text<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        #[cfg(test)]
        SPLITS.with(|n| n.set(n.get() + 1));
        let raw: Vec<_> = bytes.split_inclusive(|&b| b == b'\n').collect();
        let norm = raw.iter().map(|l| l.trim_ascii_end()).collect();
        Self {
            raw,
            norm,
            hashes: OnceLock::new(),
        }
    }

    /// Hash of each normalized line, computed on first use.
    pub(crate) fn hashes(&self) -> &[Hash] {
        self.hashes
            .get_or_init(|| self.norm.iter().map(|l| Hash::of(l)).collect())
    }

    fn in_bounds(&self, r: LineRange) -> bool {
        r.start < r.end && r.end as usize <= self.raw.len()
    }

    pub(crate) fn raw_hash(&self, r: LineRange) -> Option<Hash> {
        self.in_bounds(r).then(|| {
            let mut h = blake3::Hasher::new();
            for l in &self.raw[r.start as usize..r.end as usize] {
                h.update(l);
            }
            Hash(*h.finalize().as_bytes())
        })
    }

    /// Hash of the normalized line hashes of `r`.
    pub(crate) fn norm_hash(&self, r: LineRange) -> Option<Hash> {
        self.in_bounds(r).then(|| {
            let mut h = blake3::Hasher::new();
            for l in &self.hashes()[r.start as usize..r.end as usize] {
                h.update(&l.0);
            }
            Hash(*h.finalize().as_bytes())
        })
    }

    /// `CONTEXT` lines before, then `CONTEXT` lines after, normalized.
    pub(crate) fn context(&self, r: LineRange) -> Vec<Hash> {
        let hashes = self.hashes();
        let at = |i: Option<usize>| i.and_then(|i| hashes.get(i)).copied().unwrap_or(NO_LINE);
        let (s, e) = (r.start as usize, r.end as usize);
        (1..=CONTEXT)
            .rev()
            .map(|k| at(s.checked_sub(k)))
            .chain((0..CONTEXT).map(|k| at(Some(e + k))))
            .collect()
    }

    pub(crate) fn anchor(&self, side: Side, snapshot: SnapshotId, file: RepoPath, range: LineRange) -> Option<Anchor> {
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

impl Anchor {
    /// Anchors `range` of `text`; `None` if the range is empty or out of bounds.
    pub fn from_text(side: Side, snapshot: SnapshotId, file: RepoPath, text: &[u8], range: LineRange) -> Option<Self> {
        Text::new(text).anchor(side, snapshot, file, range)
    }
}
