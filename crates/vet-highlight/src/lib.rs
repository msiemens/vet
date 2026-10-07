//! Syntax highlighting: a text and its language in, the ranges to colour out. The grammars are
//! tree-sitter's; a range's kind is one of the few [`Kind`]s the theme has colours for.
//!
//! Offsets are UTF-16 units of the text with `\r\n` (and a lone `\r`) counted as one unit, as
//! CodeMirror counts its document, so a client applies them without looking at the text.

mod cache;
mod kind;
mod language;
mod markdown;
mod units;

use tree_sitter_highlight::{HighlightEvent, Highlighter};

pub use cache::Cache;
pub use kind::Kind;
pub use language::Language;
pub use markdown::code_blocks;
use units::Units;

/// Texts over this size are not highlighted: parsing them would cost more than the colour is worth.
pub const MAX_BYTES: usize = 2 << 20;

/// A coloured range: `from..to` in UTF-16 units.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    pub from: u32,
    pub to: u32,
    pub kind: Kind,
}

/// The coloured ranges of `text`, in order and not overlapping; none for a text over
/// [`MAX_BYTES`] or one the grammar fails on.
pub fn highlight(text: &str, language: Language) -> Vec<Span> {
    if text.len() > MAX_BYTES {
        return Vec::new();
    }
    let mut highlighter = Highlighter::new();
    let events = highlighter.highlight(language.config(), text.as_bytes(), None, None, |name| {
        Language::of_fence(name).map(Language::config)
    });
    let Ok(events) = events else { return Vec::new() };

    let mut units = Units::new(text);
    let mut spans: Vec<Span> = Vec::new();
    // The kinds of the captures enclosing the current position; the innermost one colours it.
    let mut open: Vec<Option<Kind>> = Vec::new();
    for event in events {
        match event {
            Ok(HighlightEvent::HighlightStart(h)) => open.push(language::RECOGNIZED[h.0].1),
            Ok(HighlightEvent::HighlightEnd) => {
                open.pop();
            }
            Ok(HighlightEvent::Source { start, end }) => {
                let Some(&Some(kind)) = open.last() else { continue };
                let (from, to) = (units.at(start), units.at(end));
                match spans.last_mut() {
                    Some(last) if last.kind == kind && last.to == from => last.to = to,
                    _ if from < to => spans.push(Span { from, to, kind }),
                    _ => {}
                }
            }
            Err(_) => return Vec::new(),
        }
    }
    spans
}

/// The wire form of `spans`: three numbers per span, the gap since the end of the previous one
/// (or the start of the text), its length and its [`Kind`]. For ordered, non-overlapping spans.
pub fn encode(spans: &[Span]) -> Vec<u32> {
    let mut out = Vec::with_capacity(spans.len() * 3);
    let mut end = 0;
    for span in spans {
        out.extend([span.from - end, span.to - span.from, span.kind as u32]);
        end = span.to;
    }
    out
}

#[cfg(test)]
mod tests;
