//! What a symbol is, from what a server reports about it (§7.3): its file and
//! the names of the symbols that contain it, outermost first. Servers only
//! report positions and names; the identity is decided here, once, for every
//! language.

use vet_core::content::RepoPath;

use crate::ports::{Pos, Span, Sym, SymbolInfo};

/// A symbol of a document outline with the symbols nested in it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Outline {
    pub name: String,
    /// The whole definition, including its body.
    pub span: Span,
    /// Where its name is.
    pub at: Pos,
    pub detail: Option<String>,
    pub children: Vec<Outline>,
}

/// The symbols of `outline`, each named by the chain of its containers.
pub fn flatten(file: &RepoPath, outline: &[Outline]) -> Vec<SymbolInfo> {
    fn walk(file: &RepoPath, chain: &mut Vec<String>, outline: &[Outline], out: &mut Vec<SymbolInfo>) {
        for symbol in outline {
            chain.push(symbol.name.clone());
            out.push(SymbolInfo {
                sym: Sym {
                    file: file.clone(),
                    name: chain.clone(),
                },
                span: symbol.span,
                at: symbol.at,
                detail: symbol.detail.clone(),
            });
            walk(file, chain, &symbol.children, out);
            chain.pop();
        }
    }
    let mut out = Vec::new();
    walk(file, &mut Vec::new(), outline, &mut out);
    out
}

/// The name of a symbol a server reports with only its container's name.
pub fn qualified(container: Option<String>, name: String) -> Vec<String> {
    let mut chain: Vec<_> = container.into_iter().filter(|c| !c.is_empty()).collect();
    chain.push(name);
    chain
}

fn contains(span: Span, pos: Pos) -> bool {
    let key = |p: Pos| (p.line, p.col);
    key(span.start) <= key(pos) && key(pos) <= key(span.end)
}

/// The symbol a server item (a call hierarchy entry, a workspace symbol)
/// refers to, as one of the `symbols` of its file: the one whose name is at
/// `at`, else the innermost one that contains `at`. An item the outline does
/// not know is named by its own name alone.
pub fn resolve(symbols: &[SymbolInfo], file: RepoPath, name: &str, span: Span, at: Pos) -> SymbolInfo {
    let exact = symbols.iter().find(|s| s.at == at);
    let innermost = || {
        symbols
            .iter()
            .filter(|s| contains(s.span, at))
            .max_by_key(|s| (s.span.start.line, s.span.start.col))
    };
    exact.or_else(innermost).cloned().unwrap_or_else(|| SymbolInfo {
        sym: Sym {
            file,
            name: vec![name.to_owned()],
        },
        span,
        at,
        detail: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, col: u32) -> Pos {
        Pos { line, col }
    }

    fn span(sl: u32, sc: u32, el: u32, ec: u32) -> Span {
        Span {
            start: pos(sl, sc),
            end: pos(el, ec),
        }
    }

    fn file() -> RepoPath {
        RepoPath(b"src/lib.rs".to_vec())
    }

    fn outline(name: &str, span: Span, at: Pos, children: Vec<Outline>) -> Outline {
        Outline {
            name: name.into(),
            span,
            at,
            detail: None,
            children,
        }
    }

    fn symbols() -> Vec<SymbolInfo> {
        let tree = vec![outline(
            "Shape",
            span(0, 0, 20, 1),
            pos(0, 7),
            vec![
                outline("area", span(2, 4, 6, 5), pos(2, 11), vec![]),
                outline("scale", span(8, 4, 12, 5), pos(8, 11), vec![]),
            ],
        )];
        flatten(&file(), &tree)
    }

    #[test]
    fn nested_symbols_are_named_by_their_container_chain() {
        let names: Vec<_> = symbols().into_iter().map(|s| s.sym.name).collect();
        assert_eq!(names, [vec!["Shape"], vec!["Shape", "area"], vec!["Shape", "scale"]]);
        assert!(symbols().iter().all(|s| s.sym.file == file()));
    }

    #[test]
    fn a_container_name_qualifies_a_flat_symbol_unless_it_is_empty() {
        assert_eq!(qualified(Some("Shape".into()), "area".into()), ["Shape", "area"]);
        assert_eq!(qualified(Some(String::new()), "area".into()), ["area"]);
        assert_eq!(qualified(None, "area".into()), ["area"]);
    }

    #[test]
    fn an_item_resolves_to_the_symbol_named_at_its_position() {
        let found = resolve(&symbols(), file(), "area", span(2, 4, 6, 5), pos(2, 11));
        assert_eq!(found.sym.name, ["Shape", "area"]);
    }

    #[test]
    fn an_item_inside_a_symbol_resolves_to_the_innermost_one() {
        let found = resolve(&symbols(), file(), "closure", span(9, 8, 10, 9), pos(9, 10));
        assert_eq!(found.sym.name, ["Shape", "scale"]);
        // Only the container holds this position.
        let found = resolve(&symbols(), file(), "field", span(14, 4, 14, 9), pos(14, 4));
        assert_eq!(found.sym.name, ["Shape"]);
    }

    #[test]
    fn an_item_the_outline_does_not_know_is_named_by_itself() {
        let found = resolve(&symbols(), file(), "free", span(30, 0, 32, 1), pos(30, 3));
        assert_eq!(
            found.sym,
            Sym {
                file: file(),
                name: vec!["free".into()]
            }
        );
        assert_eq!((found.span, found.at), (span(30, 0, 32, 1), pos(30, 3)));
    }
}
