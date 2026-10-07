//! A symbol's signature: the line or block that declares it. Servers give it in two shapes,
//! `documentSymbol.detail` and the Markdown of a hover; this settles which to use and how to
//! pick the declaration out of a hover.

use vet_core::review::Side;
use vet_highlight::{Syntax, fenced_blocks};

use crate::ports::SymbolInfo;
use crate::query::{Primitives, SemanticError};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Signature {
    pub code: String,
    /// What language the code is in.
    pub syntax: Syntax,
}

impl Signature {
    /// The signature a hover shows: servers put the container (a module path) in a code block
    /// of its own before the declaration, and prose after it, so it is the last block that is
    /// not empty. A hover without a code block is taken whole.
    pub fn of_hover(markdown: &str) -> Option<Self> {
        let block = fenced_blocks(markdown)
            .into_iter()
            .rev()
            .find(|b| !b.code.trim().is_empty())
            .map(|mut b| {
                b.code.truncate(b.code.strip_suffix('\n').unwrap_or(&b.code).len());
                Self {
                    code: b.code,
                    syntax: Syntax::Fence(b.info),
                }
            });
        block.or_else(|| {
            let text = markdown.trim();
            (!text.is_empty()).then(|| Self {
                code: text.to_owned(),
                syntax: Syntax::Plain,
            })
        })
    }

    fn of_detail(detail: &str) -> Option<Self> {
        let code = detail.trim();
        (!code.is_empty()).then(|| Self {
            code: code.to_owned(),
            syntax: Syntax::File,
        })
    }
}

/// The signature of `symbol`, looked up on `side`: the declaration in its hover, else the
/// server's `detail`, which is often less (rust-analyzer's leaves out the name). `None` if the
/// server has nothing to say.
pub async fn signature(
    p: &impl Primitives,
    side: Side,
    symbol: &SymbolInfo,
) -> Result<Option<Signature>, SemanticError> {
    let hovered = p.hover(side, &symbol.location()).await;
    if let Ok(Some(text)) = &hovered
        && let Some(found) = Signature::of_hover(text)
    {
        return Ok(Some(found));
    }
    match symbol.detail.as_deref().and_then(Signature::of_detail) {
        Some(found) => Ok(Some(found)),
        None => hovered.map(|_| None),
    }
}

#[cfg(test)]
mod tests {
    use vet_core::content::RepoPath;

    use super::*;
    use crate::ports::{At, Diagnostic, Location, Pos, QueryError, Span, Sym};
    use crate::testutil::block_on;

    fn fence(code: &str, info: &str) -> Signature {
        Signature {
            code: code.into(),
            syntax: Syntax::Fence(info.into()),
        }
    }

    #[test]
    fn the_declaration_follows_the_container_block() {
        let hover = "```rust\nmy_crate::module\n```\n\n```rust\npub fn run(x: u32) -> bool\n```\n\nRuns it.";
        assert_eq!(
            Signature::of_hover(hover),
            Some(fence("pub fn run(x: u32) -> bool", "rust"))
        );
    }

    #[test]
    fn empty_blocks_are_skipped() {
        let hover = "```python\ndef f(x)\n```\n\n```\n\n```\n";
        assert_eq!(Signature::of_hover(hover), Some(fence("def f(x)", "python")));
    }

    #[test]
    fn a_fence_without_an_info_string_stays_unnamed() {
        assert_eq!(Signature::of_hover("```\nint x\n```"), Some(fence("int x", "")));
    }

    #[test]
    fn a_hover_without_code_is_taken_whole() {
        assert_eq!(
            Signature::of_hover("  just prose \n"),
            Some(Signature {
                code: "just prose".into(),
                syntax: Syntax::Plain
            })
        );
    }

    #[test]
    fn nothing_to_show_is_none() {
        assert_eq!(Signature::of_hover(" \n"), None);
        assert_eq!(Signature::of_detail("  "), None);
    }

    /// A server whose hover is `hover` and which answers nothing else.
    struct Hovering(Result<Option<&'static str>, &'static str>);

    impl Primitives for Hovering {
        async fn document_symbols(&self, _: Side, _: &RepoPath) -> Result<Vec<SymbolInfo>, SemanticError> {
            Ok(vec![])
        }
        async fn definition(&self, _: Side, _: &At) -> Result<Vec<Location>, SemanticError> {
            Ok(vec![])
        }
        async fn references(&self, _: Side, _: &At) -> Result<Vec<Location>, SemanticError> {
            Ok(vec![])
        }
        async fn hover(&self, _: Side, _: &At) -> Result<Option<String>, SemanticError> {
            self.0
                .map(|text| text.map(String::from))
                .map_err(|e| SemanticError::Query(QueryError(e.into())))
        }
        async fn incoming_calls(&self, _: Side, _: &At) -> Result<Vec<SymbolInfo>, SemanticError> {
            Ok(vec![])
        }
        async fn outgoing_calls(&self, _: Side, _: &At) -> Result<Vec<SymbolInfo>, SemanticError> {
            Ok(vec![])
        }
        async fn diagnostics(&self, _: Side, _: &RepoPath) -> Result<Vec<Diagnostic>, SemanticError> {
            Ok(vec![])
        }
    }

    fn symbol(detail: Option<&str>) -> SymbolInfo {
        let pos = Pos { line: 0, col: 0 };
        SymbolInfo {
            sym: Sym {
                file: RepoPath(b"a.rs".to_vec()),
                name: vec!["f".into()],
            },
            span: Span { start: pos, end: pos },
            at: pos,
            detail: detail.map(String::from),
        }
    }

    #[test]
    fn the_hover_wins_over_the_detail() {
        let server = Hovering(Ok(Some("```rust\npub fn f() -> u32\n```")));
        let found = block_on(signature(&server, Side::Target, &symbol(Some("fn() -> u32"))));
        assert_eq!(found.unwrap(), Some(fence("pub fn f() -> u32", "rust")));
    }

    #[test]
    fn the_detail_stands_in_for_a_silent_or_failing_hover() {
        let detail = Some(Signature {
            code: "fn() -> u32".into(),
            syntax: Syntax::File,
        });
        for server in [Hovering(Ok(None)), Hovering(Err("boom"))] {
            let found = block_on(signature(&server, Side::Base, &symbol(Some("fn() -> u32"))));
            assert_eq!(found.unwrap(), detail);
        }
    }

    #[test]
    fn without_either_the_answer_is_none_or_the_hovers_error() {
        let silent = block_on(signature(&Hovering(Ok(None)), Side::Target, &symbol(None)));
        assert_eq!(silent.unwrap(), None);
        let failed = block_on(signature(&Hovering(Err("boom")), Side::Target, &symbol(None)));
        assert!(failed.is_err());
    }

    #[test]
    fn detail_is_in_the_files_language() {
        assert_eq!(
            Signature::of_detail(" fn f() "),
            Some(Signature {
                code: "fn f()".into(),
                syntax: Syntax::File
            })
        );
    }
}
