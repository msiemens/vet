//! What a symbol is, from what a server reports about it (§7.3): its file and
//! the names of the symbols that contain it, outermost first. Servers only
//! report positions and names; the identity is decided here, once, for every
//! language.

use vet_core::content::RepoPath;

use crate::ports::{Pos, Span, Sym, SymbolInfo, SymbolKind};

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
    /// Whether it is a function, method or the like: what it declares is local to it.
    pub callable: bool,
    pub kind: SymbolKind,
    pub children: Vec<Outline>,
}

/// Whether a server names a symbol only by where it is: a function or class expression it
/// reports as `<function>` or `<class>`, a callback as `items.map() callback`, a lambda as
/// `(lambda)`, or compiler-made code by a bracketed last part, as in
/// `Program.<top-level-statements-entry-point>`.
fn is_anonymous(name: &str) -> bool {
    let last = name.rsplit('.').next().unwrap_or(name);
    (last.starts_with('<') && last.ends_with('>'))
        || (name.starts_with('(') && name.ends_with(')'))
        || name.ends_with(" callback")
}

/// Whether some symbol of the outline is declared where a server item named `name` has its
/// name. The file-level item a server reports for code outside every declaration (named after
/// the file, at its start) is not, whatever declaration happens to start there.
pub fn is_declared_at(symbols: &[SymbolInfo], name: &str, at: Pos) -> bool {
    symbols
        .iter()
        .any(|s| s.at == at && s.sym.name.last().map(String::as_str) == Some(name))
}

/// The symbols of `outline`, each named by the chain of its named containers. Only what is
/// declared by name is a symbol: closures, callbacks and the variables, parameters and
/// constants local to a function are not, and what they hold belongs to the nearest symbol
/// around them. A function declared inside a function is one, a class or type declared
/// there too.
pub fn flatten(file: &RepoPath, outline: &[Outline]) -> Vec<SymbolInfo> {
    fn walk(file: &RepoPath, chain: &mut Vec<String>, local: bool, outline: &[Outline], out: &mut Vec<SymbolInfo>) {
        for symbol in outline {
            let anonymous = is_anonymous(&symbol.name);
            // What a variable holds (a function, a class, an object) is inside it as well.
            let holds_code = symbol.callable || anonymous || (!symbol.container && !symbol.children.is_empty());
            if anonymous || (local && !symbol.container) {
                walk(file, chain, holds_code, &symbol.children, out);
                continue;
            }
            chain.push(symbol.name.clone());
            out.push(SymbolInfo {
                sym: Sym {
                    file: file.clone(),
                    name: chain.clone(),
                },
                span: symbol.span,
                at: symbol.at,
                detail: symbol.detail.clone(),
                kind: symbol.kind,
            });
            walk(file, chain, holds_code, &symbol.children, out);
            chain.pop();
        }
    }
    let mut out = Vec::new();
    walk(file, &mut Vec::new(), false, outline, &mut out);
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
/// not know is named by its own name alone, unless it is anonymous: that is no symbol.
pub fn resolve(symbols: &[SymbolInfo], file: RepoPath, name: &str, span: Span, at: Pos) -> Option<SymbolInfo> {
    let exact = symbols.iter().find(|s| s.at == at);
    let innermost = || {
        symbols
            .iter()
            .filter(|s| contains(s.span, at))
            .max_by_key(|s| (s.span.start.line, s.span.start.col))
    };
    if let Some(found) = exact.or_else(innermost) {
        return Some(found.clone());
    }
    (!is_anonymous(name)).then(|| SymbolInfo {
        sym: Sym {
            file,
            name: vec![name.to_owned()],
        },
        span,
        at,
        detail: None,
        kind: SymbolKind::Other,
    })
}

/// The innermost of `symbols` that holds `pos`: the symbol a reference there is part of. What
/// a function declares locally is no symbol (see [`flatten`]), so a reference in a function's
/// signature or body belongs to the function. A constant or field is one, whether it holds a
/// closure or plain data: a reference in its initializer is part of it.
pub fn enclosing(symbols: &[SymbolInfo], pos: Pos) -> Option<&SymbolInfo> {
    let key = |p: Pos| (p.line, p.col);
    symbols
        .iter()
        .filter(|s| contains(s.span, pos))
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
            callable: false,
            kind: SymbolKind::Type,
            children,
        }
    }

    /// A function or method: what it declares is local to it.
    fn function(name: &str, span: Span, at: Pos, children: Vec<Outline>) -> Outline {
        Outline {
            callable: true,
            kind: SymbolKind::Function,
            ..outline(name, span, at, children)
        }
    }

    fn names(symbols: Vec<SymbolInfo>) -> Vec<Vec<String>> {
        symbols.into_iter().map(|s| s.sym.name).collect()
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
        let found = resolve(&symbols(), file(), "area", span(2, 4, 6, 5), pos(2, 11)).unwrap();
        assert_eq!(found.sym.name, ["Shape", "area"]);
    }

    #[test]
    fn an_item_inside_a_symbol_resolves_to_the_innermost_one() {
        let found = resolve(&symbols(), file(), "closure", span(9, 8, 10, 9), pos(9, 10)).unwrap();
        assert_eq!(found.sym.name, ["Shape", "scale"]);
        // Only the container holds this position.
        let found = resolve(&symbols(), file(), "field", span(14, 4, 14, 9), pos(14, 4)).unwrap();
        assert_eq!(found.sym.name, ["Shape"]);
    }

    #[test]
    fn a_callback_outside_every_symbol_resolves_to_none() {
        let found = resolve(
            &symbols(),
            file(),
            "describe(\"suite\") callback",
            span(30, 0, 32, 1),
            pos(30, 0),
        );
        assert_eq!(found, None);
        let found = resolve(&symbols(), file(), "<function>", span(30, 0, 32, 1), pos(30, 0));
        assert_eq!(found, None);
    }

    #[test]
    fn lambdas_and_compiler_made_code_outside_every_symbol_resolve_to_none() {
        for name in ["(lambda)", "Program.<top-level-statements-entry-point>"] {
            assert_eq!(resolve(&symbols(), file(), name, span(30, 0, 32, 1), pos(30, 0)), None);
        }
        // A generic type's name is a name.
        assert!(resolve(&symbols(), file(), "Box<T>.get()", span(30, 0, 32, 1), pos(30, 0)).is_some());
    }

    #[test]
    fn a_file_level_item_is_not_declared_by_what_starts_at_the_same_place() {
        // `Shape` starts where a server puts the item it names after the file.
        let symbols = symbols();
        assert!(!is_declared_at(&symbols, "a.ts", pos(0, 7)));
        assert!(is_declared_at(&symbols, "Shape", pos(0, 7)));
        assert!(!is_declared_at(&symbols, "Shape", pos(0, 0)));
    }

    /// What typescript-language-server lists for a class whose method has a callback, a local
    /// constant holding a closure, a local function and a function expression.
    fn typescript_widget() -> Vec<SymbolInfo> {
        let callback = |name: &str, line| function(name, span(line, 20, line, 40), pos(line, 20), vec![]);
        let method = function(
            "method",
            span(2, 2, 10, 3),
            pos(2, 2),
            vec![
                variable_with(
                    "local",
                    span(3, 4, 3, 40),
                    pos(3, 10),
                    vec![callback("items.map() callback", 3)],
                ),
                variable_with("inner", span(4, 4, 4, 40), pos(4, 10), vec![]),
                function("named", span(5, 4, 5, 30), pos(5, 13), vec![]),
                callback("items.forEach() callback", 6),
                function("<function>", span(7, 4, 7, 30), pos(7, 5), vec![]),
            ],
        );
        let tree = vec![
            outline(
                "Widget",
                span(0, 0, 12, 1),
                pos(0, 13),
                vec![variable("field", span(1, 2, 1, 12), pos(1, 2)), method],
            ),
            variable("arrow", span(14, 0, 14, 40), pos(14, 13)),
        ];
        flatten(&file(), &tree)
    }

    fn variable_with(name: &str, span: Span, at: Pos, children: Vec<Outline>) -> Outline {
        Outline {
            children,
            ..variable(name, span, at)
        }
    }

    #[test]
    fn closures_and_callbacks_are_no_symbols_but_named_declarations_are() {
        assert_eq!(
            names(typescript_widget()),
            [
                vec!["Widget"],
                vec!["Widget", "field"],
                vec!["Widget", "method"],
                vec!["Widget", "method", "named"],
                vec!["arrow"],
            ]
        );
    }

    #[test]
    fn a_closure_assigned_at_module_level_is_a_named_symbol() {
        let tree = vec![variable_with(
            "handler",
            span(0, 0, 4, 1),
            pos(0, 6),
            vec![function("items.map() callback", span(1, 4, 3, 5), pos(1, 4), vec![])],
        )];
        assert_eq!(names(flatten(&file(), &tree)), [vec!["handler"]]);
    }

    #[test]
    fn a_class_declared_inside_a_function_keeps_its_members() {
        let tree = vec![function(
            "make",
            span(0, 0, 8, 1),
            pos(0, 9),
            vec![outline(
                "Local",
                span(1, 2, 6, 3),
                pos(1, 8),
                vec![
                    variable("count", span(2, 4, 2, 14), pos(2, 4)),
                    function("bump", span(3, 4, 5, 5), pos(3, 4), vec![]),
                ],
            )],
        )];
        assert_eq!(
            names(flatten(&file(), &tree)),
            [
                vec!["make"],
                vec!["make", "Local"],
                vec!["make", "Local", "count"],
                vec!["make", "Local", "bump"]
            ]
        );
    }

    /// An outline entry that holds no code of its own: a parameter, variable or field.
    fn variable(name: &str, span: Span, at: Pos) -> Outline {
        Outline {
            container: false,
            kind: SymbolKind::Other,
            ..outline(name, span, at, vec![])
        }
    }

    /// What a Python server lists for `class Invoice` with `merge(self, groups)`, which has a local
    /// `total` and a nested function `key`: its parameters and locals are in the outline.
    fn python_invoice() -> Vec<SymbolInfo> {
        let merge = function(
            "merge",
            span(2, 4, 12, 0),
            pos(2, 8),
            vec![
                variable("self", span(2, 14, 2, 18), pos(2, 14)),
                variable("groups", span(2, 20, 2, 26), pos(2, 20)),
                variable("total", span(4, 8, 4, 17), pos(4, 8)),
                function("key", span(6, 8, 8, 20), pos(6, 12), vec![]),
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
    fn a_reference_outside_every_symbol_belongs_to_none() {
        let symbols = python_invoice();
        assert_eq!(within(&symbols, pos(21, 0)), None);
        assert_eq!(within(&[], pos(0, 0)), None);
    }

    #[test]
    fn a_reference_in_a_module_level_constant_belongs_to_it() {
        // `const handler = () => helper()` holds code, `const LIMIT = 10` data; the outline
        // does not say which, and the reference is part of the constant either way.
        let tree = vec![variable("handler", span(3, 0, 3, 30), pos(3, 6))];
        let symbols = flatten(&file(), &tree);
        assert_eq!(within(&symbols, pos(3, 20)), Some(vec!["handler".into()]));
        assert_eq!(within(&symbols, pos(4, 0)), None);
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
        let found = resolve(&symbols(), file(), "free", span(30, 0, 32, 1), pos(30, 3)).unwrap();
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
