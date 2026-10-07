use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

use crate::{Language, encode, highlight};

/// The highlights of each fenced code block of `markdown`, in document order, encoded like
/// [`encode`] over the block's text (what a Markdown renderer shows as the block's code). A fence
/// without a language takes `default`; one with an unknown language, or no language
/// at all, has no highlights.
pub fn code_blocks(markdown: &str, default: Option<Language>) -> Vec<Vec<u32>> {
    let mut blocks = Vec::new();
    // The language and text of the fenced block being read.
    let mut open: Option<(Option<Language>, String)> = None;
    for event in Parser::new(markdown) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                let info = info.trim();
                let language = if info.is_empty() {
                    default
                } else {
                    Language::of_fence(info)
                };
                open = Some((language, String::new()));
            }
            Event::Text(text) => {
                if let Some((_, code)) = &mut open {
                    code.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((language, code)) = open.take() {
                    blocks.push(language.map(|l| encode(&highlight(&code, l))).unwrap_or_default());
                }
            }
            _ => {}
        }
    }
    blocks
}
