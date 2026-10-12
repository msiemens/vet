//! Syntax highlighting: a text and its language in, the ranges to colour out. The grammars are
//! tree-sitter's; a range's kind is one of the few [`Kind`]s the theme has colours for.
//!
//! Offsets are in the units of [`vet_core::text`], so a client applies them without looking at the
//! text.

mod cache;
mod kind;
mod language;
mod markdown;
mod syntax;

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};
use vet_core::text::Units;

pub use cache::Cache;
pub use kind::Kind;
pub use language::Language;
pub use markdown::{Fence, code_blocks, fenced_blocks};
pub use syntax::Syntax;

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
    spans(text, language.config())
}

/// The coloured ranges of a signature `code`, which tell the qualifier of a qualified name from
/// the name ([`Kind::Qualifier`]). A signature is a declaration without its body, often not valid
/// code on its own: C# is read as a member of a class, where it is.
pub fn highlight_signature(code: &str, language: Language) -> Vec<Span> {
    let config = language.signature_config();
    if language != Language::CSharp {
        return spans(code, config);
    }
    let before = "class _ { ";
    let wrapped = format!("{before}{code}; }}");
    let mut units = Units::new(&wrapped);
    let (shift, end) = (units.at(before.len()), units.at(before.len() + code.len()));
    spans(&wrapped, config)
        .into_iter()
        .filter(|s| s.from >= shift && s.to <= end)
        .map(|s| Span {
            from: s.from - shift,
            to: s.to - shift,
            ..s
        })
        .collect()
}

fn spans(text: &str, config: &HighlightConfiguration) -> Vec<Span> {
    if text.len() > MAX_BYTES {
        return Vec::new();
    }
    let mut highlighter = Highlighter::new();
    let events = highlighter.highlight(config, text.as_bytes(), None, None, |name| {
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

/// The part of `encoded` (see [`encode`]) inside `from..to`, as the wire form of a text that
/// starts at `from`: spans are cut to the range and moved back by `from`.
pub fn slice(encoded: &[u32], from: u32, to: u32) -> Vec<u32> {
    let mut spans = Vec::new();
    let mut end = 0;
    for &[gap, len, kind] in encoded.as_chunks::<3>().0 {
        let start = end + gap;
        end = start + len;
        let (lo, hi) = (start.max(from), end.min(to));
        if lo < hi {
            spans.push((lo - from, hi - from, kind));
        }
    }
    let mut out = Vec::with_capacity(spans.len() * 3);
    let mut last = 0;
    for (lo, hi, kind) in spans {
        out.extend([lo - last, hi - lo, kind]);
        last = hi;
    }
    out
}

#[cfg(test)]
mod tests;
