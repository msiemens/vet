//! A file pair prepared once for resolving or remapping many anchors.

use std::sync::OnceLock;

use crate::content::BlobSource;
use crate::linemap::unique_map;
use crate::review::FileRemapInput;
use crate::text::Text;

type KeptLines = Vec<Option<usize>>;

/// The old and new text of a [`FileRemapInput`] with their line hashes and line
/// maps, each computed on first use and shared by every anchor or span resolved
/// against it.
pub struct PreparedFile<'a> {
    pub(crate) input: &'a FileRemapInput,
    pub(crate) old: Option<Text<'a>>,
    pub(crate) new: Option<Text<'a>>,
    raw_map: OnceLock<KeptLines>,
    norm_map: OnceLock<KeptLines>,
}

impl<'a> PreparedFile<'a> {
    pub fn new(input: &'a FileRemapInput, blobs: &'a dyn BlobSource) -> Self {
        let text = |b| blobs.blob(b).map(Text::new);
        Self {
            input,
            old: input.old_blob.and_then(text),
            new: input.new_blob.and_then(text),
            raw_map: OnceLock::new(),
            norm_map: OnceLock::new(),
        }
    }

    /// Unique line map by exact line bytes; `None` unless both blobs are available.
    pub(crate) fn raw_map(&self) -> Option<&[Option<usize>]> {
        self.map(&self.raw_map, |t| &t.raw)
    }

    /// Unique line map by normalized lines.
    pub(crate) fn norm_map(&self) -> Option<&[Option<usize>]> {
        self.map(&self.norm_map, |t| &t.norm)
    }

    fn map<'s>(
        &'s self,
        cell: &'s OnceLock<KeptLines>,
        lines: impl for<'t> Fn(&'t Text<'a>) -> &'t [&'a [u8]],
    ) -> Option<&'s [Option<usize>]> {
        let (old, new) = (self.old.as_ref()?, self.new.as_ref()?);
        let map = cell.get_or_init(|| {
            if self.input.old_blob == self.input.new_blob {
                (0..old.raw.len()).map(Some).collect()
            } else {
                unique_map(lines(old), lines(new))
            }
        });
        Some(map)
    }
}
