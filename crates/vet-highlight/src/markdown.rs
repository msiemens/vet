use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

use crate::{Language, encode, highlight};

/// A fenced code block of a Markdown text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Fence {
    /// The block's info string, trimmed; empty for a fence without one.
    pub info: String,
    /// The block's code as a Markdown renderer shows it, with its final line break.
    pub code: String,
}

/// The fenced code blocks of `markdown`, in document order.
pub fn fenced_blocks(markdown: &str) -> Vec<Fence> {
    let mut blocks = Vec::new();
    let mut open: Option<Fence> = None;
    for event in Parser::new(markdown) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                open = Some(Fence {
                    info: info.trim().to_owned(),
                    code: String::new(),
                });
            }
            Event::Text(text) => {
                if let Some(block) = &mut open {
                    block.code.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => blocks.extend(open.take()),
            _ => {}
        }
    }
    blocks
}

/// The highlights of each fenced code block of `markdown`, in document order, encoded like
/// [`encode`] over the block's code. A fence without a language takes `default`; one with an
/// unknown language, or no language at all, has no highlights.
pub fn code_blocks(markdown: &str, default: Option<Language>) -> Vec<Vec<u32>> {
    fenced_blocks(markdown)
        .into_iter()
        .map(|block| {
            Language::of_fence_or(&block.info, default)
                .map(|l| encode(&highlight(&block.code, l)))
                .unwrap_or_default()
        })
        .collect()
}
