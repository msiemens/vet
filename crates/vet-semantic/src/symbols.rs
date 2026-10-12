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
    /// See [`SymbolInfo::container`].
    pub container: bool,
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
                container: symbol.container,
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
        container: false,
    })
}

/// The innermost container among `symbols` that holds `pos`: the symbol a reference there is
/// part of. A variable, parameter or field is not one, so a reference in a function's signature
/// belongs to the function, not to its parameter.
pub fn enclosing(symbols: &[SymbolInfo], pos: Pos) -> Option<&SymbolInfo> {
    let key = |p: Pos| (p.line, p.col);
    symbols
        .iter()
        .filter(|s| s.container && contains(s.span, pos))
        .max_by_key(|s| (key(s.span.start), std::cmp::Reverse(key(s.span.end))))
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
            container: true,
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

    /// An outline entry that holds no code of its own: a parameter, variable or field.
    fn variable(name: &str, span: Span, at: Pos) -> Outline {
        Outline {
            container: false,
            ..outline(name, span, at, vec![])
        }
    }

    /// What a Python server lists for `class Invoice` with `merge(self, groups)`, which has a local
    /// `total` and a nested function `key`: its parameters and locals are in the outline.
    fn python_invoice() -> Vec<SymbolInfo> {
        let merge = outline(
            "merge",
            span(2, 4, 12, 0),
            pos(2, 8),
            vec![
                variable("self", span(2, 14, 2, 18), pos(2, 14)),
                variable("groups", span(2, 20, 2, 26), pos(2, 20)),
                variable("total", span(4, 8, 4, 17), pos(4, 8)),
                outline("key", span(6, 8, 8, 20), pos(6, 12), vec![]),
            ],
        );
        let tree = vec![
            outline("Invoice", span(0, 0, 20, 0), pos(0, 6), vec![merge]),
            variable("LIMIT", span(22, 0, 22, 9), pos(22, 0)),
        ];
        flatten(&file(), &tree)
    }

    fn within(symbols: &[SymbolInfo], pos: Pos) -> Option<Vec<String>> {
        enclosing(symbols, pos).map(|s| s.sym.name.clone())
    }

    #[test]
    fn a_reference_belongs_to_the_function_not_to_the_parameter_or_variable_it_is_in() {
        let symbols = python_invoice();
        // In the annotation of the parameter `groups`.
        assert_eq!(
            within(&symbols, pos(2, 23)),
            Some(vec!["Invoice".into(), "merge".into()])
        );
        // In the initializer of the local `total`.
        assert_eq!(
            within(&symbols, pos(4, 14)),
            Some(vec!["Invoice".into(), "merge".into()])
        );
    }

    #[test]
    fn a_reference_belongs_to_the_innermost_container() {
        let symbols = python_invoice();
        assert_eq!(
            within(&symbols, pos(7, 12)),
            Some(vec!["Invoice".into(), "merge".into(), "key".into()])
        );
        // In the class body, between its members.
        assert_eq!(within(&symbols, pos(15, 4)), Some(vec!["Invoice".into()]));
    }

    #[test]
    fn a_reference_outside_every_container_belongs_to_none() {
        let symbols = python_invoice();
        assert_eq!(within(&symbols, pos(21, 0)), None);
        // A module-level variable is not a container either.
        assert_eq!(within(&symbols, pos(22, 4)), None);
        assert_eq!(within(&[], pos(0, 0)), None);
    }

    #[test]
    fn of_two_containers_that_start_alike_the_smaller_one_wins() {
        // `impl Invoice { fn f() {} }` on one line: the method starts where the impl does.
        let tree = vec![outline(
            "impl Invoice",
            span(0, 0, 0, 30),
            pos(0, 5),
            vec![outline("f", span(0, 0, 0, 20), pos(0, 3), vec![])],
        )];
        let symbols = flatten(&file(), &tree);
        assert_eq!(
            within(&symbols, pos(0, 10)),
            Some(vec!["impl Invoice".into(), "f".into()])
        );
        assert_eq!(within(&symbols, pos(0, 25)), Some(vec!["impl Invoice".into()]));
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
