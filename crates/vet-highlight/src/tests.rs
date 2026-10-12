use super::*;
use Kind::{Attribute, Comment, Function, Heading, Keyword, Link, Number, Qualifier, String as Str, Type};

/// The highlighted pieces of `text` with their kind, read back through the encoding's offsets
/// (UTF-16 units, line breaks normalized to `\n`).
fn pieces(text: &str, language: Language) -> Vec<(String, Kind)> {
    let units: Vec<u16> = text.replace("\r\n", "\n").replace('\r', "\n").encode_utf16().collect();
    highlight(text, language)
        .iter()
        .map(|s| {
            (
                String::from_utf16(&units[s.from as usize..s.to as usize]).unwrap(),
                s.kind,
            )
        })
        .collect()
}

fn has(pieces: &[(String, Kind)], text: &str, kind: Kind) -> bool {
    pieces.iter().any(|(t, k)| t == text && *k == kind)
}

/// The text of a file of the repository, relative to this crate.
fn repository_file(path: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The protocol's documentation of the wire numbers and the UI's list of style classes are
/// written by hand: this ties them to `Kind`, whose variant names (in lower case) they use.
#[test]
fn the_protocol_docs_and_the_ui_list_the_kinds_in_wire_order() {
    let mut kinds = [
        Keyword, Str, Number, Comment, Type, Function, Attribute, Heading, Link, Qualifier,
    ];
    kinds.sort_by_key(|&k| k as u8);
    assert!(
        kinds.iter().enumerate().all(|(i, &k)| k as usize == i),
        "wire numbers have gaps"
    );
    let names = kinds.map(|k| format!("{k:?}").to_lowercase());

    let protocol = repository_file("../vet-protocol/src/text.rs");
    let docs = protocol
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("///"))
        .collect::<Vec<_>>()
        .join(" ");
    let docs = docs.split_whitespace().collect::<Vec<_>>().join(" ");
    let numbered: Vec<_> = names.iter().enumerate().map(|(i, n)| format!("{i} {n}")).collect();
    let numbered = numbered.join(", ");
    assert!(
        docs.contains(&format!("({numbered})")),
        "the docs of `Highlights` do not say: {numbered}"
    );

    let ui = repository_file("../../ui/src/lib/highlights.ts");
    let list = ui
        .split("const CLASSES = [")
        .nth(1)
        .and_then(|rest| rest.split(']').next());
    let classes: Vec<_> = list
        .unwrap_or_default()
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    assert_eq!(classes, names.map(|n| format!("syn-{n}")));
}

#[test]
fn rust_is_highlighted() {
    let p = pieces("fn main() { let n: u32 = 7; // done\n}", Language::Rust);
    assert!(has(&p, "fn", Keyword));
    assert!(has(&p, "main", Function));
    assert!(has(&p, "u32", Type));
    assert!(has(&p, "7", Number));
    assert!(has(&p, "// done", Comment));
}

/// Each language is read with its own grammar.
#[test]
fn every_plain_language_is_highlighted_by_its_own_grammar() {
    for (language, text, piece, kind) in [
        (Language::Python, "def f(x):\n    return x\n", "def", Keyword),
        (Language::CSharp, "class A { }\n", "class", Keyword),
        (Language::Yaml, "a: \"s\"\n", "\"s\"", Str),
        (Language::Toml, "name = \"vet\"\n", "\"vet\"", Str),
        (Language::Bash, "if [ -f x ]; then echo; fi\n", "if", Keyword),
        (Language::Css, "a { margin: 0 }\n", "0", Number),
    ] {
        let p = pieces(text, language);
        assert!(has(&p, piece, kind), "{language:?}: {p:?}");
    }
}

#[test]
fn typescript_and_tsx_are_highlighted() {
    let p = pieces(
        "const n: number = 1;\ninterface A { b: string }\n",
        Language::TypeScript,
    );
    assert!(has(&p, "const", Keyword));
    assert!(has(&p, "interface", Keyword));
    assert!(has(&p, "number", Type));
    let p = pieces("const a = <div id=\"x\">hi</div>;\n", Language::Tsx);
    assert!(has(&p, "div", Attribute));
    assert!(has(&p, "\"x\"", Str));
}

#[test]
fn javascript_and_jsx_are_highlighted() {
    let p = pieces(
        "function f() { return `t` + null; }\nconst e = <b/>;\n",
        Language::JavaScript,
    );
    assert!(has(&p, "function", Keyword));
    assert!(has(&p, "f", Function));
    assert!(has(&p, "`t`", Str));
    assert!(has(&p, "null", Number));
    assert!(has(&p, "b", Attribute));
}

#[test]
fn json_values_are_highlighted_but_keys_are_not_strings() {
    let p = pieces("{\"k\": [true, null, 1.5, \"v\"]}", Language::Json);
    assert!(has(&p, "true", Number));
    assert!(has(&p, "null", Number));
    assert!(has(&p, "1.5", Number));
    assert!(has(&p, "\"v\"", Str));
    // A key is not coloured as a string.
    assert!(!has(&p, "\"k\"", Str));
}

#[test]
fn html_highlights_its_script_as_javascript() {
    let p = pieces(
        "<a href=\"x\">t</a><script>let n = 1;</script><style>b { color: red }</style><!-- c -->",
        Language::Html,
    );
    assert!(has(&p, "a", Attribute));
    assert!(has(&p, "x", Str));
    assert!(has(&p, "<!-- c -->", Comment));
    // The script is JavaScript.
    assert!(has(&p, "let", Keyword));
    assert!(has(&p, "1", Number));
}

#[test]
fn markdown_highlights_its_markup_and_the_code_in_fences() {
    let p = pieces(
        "# Title\n\nsome **bold** [t](http://u)\n\n```rust\nfn f() {}\n```\n",
        Language::Markdown,
    );
    assert!(has(&p, "Title", Heading));
    assert!(has(&p, "**bold**", Heading));
    assert!(has(&p, "http://u", Link));
    assert!(has(&p, "fn", Keyword));
}

#[test]
fn a_string_and_a_comment_over_several_lines_are_one_span() {
    let p = pieces("let s = \"a\nb\";\n/* x\n y */\n", Language::Rust);
    assert!(has(&p, "\"a\nb\"", Str));
    assert!(has(&p, "/* x\n y */", Comment));
}

#[test]
fn a_crlf_line_break_counts_as_one_unit() {
    let p = pieces("let a = 1;\r\nlet s = \"x\r\ny\";\r\nlet b = 2;\r\n", Language::Rust);
    assert!(has(&p, "1", Number));
    assert!(has(&p, "\"x\ny\"", Str));
    assert!(has(&p, "2", Number));
    let spans = highlight("a\r\n1", Language::Json);
    assert_eq!(
        spans,
        [Span {
            from: 2,
            to: 3,
            kind: Number
        }]
    );
}

#[test]
fn columns_count_utf16_units() {
    // The emoji is two units, `é` one.
    let spans = highlight("let s = \"😀é\"; let n = 5;", Language::Rust);
    let units: Vec<u16> = "let s = \"😀é\"; let n = 5;".encode_utf16().collect();
    let text = |s: &Span| String::from_utf16(&units[s.from as usize..s.to as usize]).unwrap();
    assert!(spans.iter().any(|s| s.kind == Str && text(s) == "\"😀é\""));
    assert!(spans.iter().any(|s| s.kind == Number && text(s) == "5"));
}

#[test]
fn spans_are_ordered_and_apart() {
    let text = "fn f<T: Clone>(a: &T) -> u8 { /* c */ 1 + 2 }\n".repeat(20);
    let spans = highlight(&text, Language::Rust);
    assert!(!spans.is_empty());
    assert!(spans.iter().all(|s| s.from < s.to));
    assert!(spans.windows(2).all(|w| w[0].to <= w[1].from));
}

#[test]
fn empty_text_and_unknown_languages_have_none() {
    assert!(highlight("", Language::Rust).is_empty());
    assert_eq!(Language::of_path("notes.txt"), None);
    assert_eq!(Language::of_path("Makefile"), None);
    assert_eq!(Language::of_fence("brainfuck"), None);
    assert_eq!(Language::of_fence(""), None);
}

#[test]
fn text_over_the_limit_is_not_highlighted() {
    let text = "1,".repeat(MAX_BYTES / 2 + 1);
    assert!(highlight(&text, Language::Json).is_empty());
}

#[test]
fn a_file_extension_names_its_language() {
    for (path, language) in [
        ("src/main.rs", Language::Rust),
        ("a/b.PY", Language::Python),
        ("x.pyi", Language::Python),
        ("x.ts", Language::TypeScript),
        ("x.mts", Language::TypeScript),
        ("x.tsx", Language::Tsx),
        ("x.js", Language::JavaScript),
        ("x.cjs", Language::JavaScript),
        ("x.jsx", Language::JavaScript),
        ("x.json", Language::Json),
        ("README.md", Language::Markdown),
        ("x.yml", Language::Yaml),
        ("Cargo.toml", Language::Toml),
        ("x.sh", Language::Bash),
        ("x.cs", Language::CSharp),
        ("x.css", Language::Css),
        ("x.svg", Language::Html),
    ] {
        assert_eq!(Language::of_path(path), Some(language), "{path}");
    }
    // The extension is the file name's, not a directory's.
    assert_eq!(Language::of_path("dir.rs/Makefile"), None);
}

#[test]
fn a_fence_info_string_names_its_language() {
    assert_eq!(Language::of_fence("rust"), Some(Language::Rust));
    assert_eq!(Language::of_fence("rust,ignore"), Some(Language::Rust));
    assert_eq!(Language::of_fence("TS {1}"), Some(Language::TypeScript));
    assert_eq!(Language::of_fence("c#"), Some(Language::CSharp));
    assert_eq!(Language::of_fence("shell"), Some(Language::Bash));
}

#[test]
fn every_language_builds() {
    for language in Language::ALL {
        language.config();
        language.signature_config();
    }
}

#[test]
fn spans_are_encoded_as_gap_length_kind_triples() {
    let spans = [
        Span {
            from: 2,
            to: 5,
            kind: Keyword,
        },
        Span {
            from: 5,
            to: 6,
            kind: Number,
        },
        Span {
            from: 9,
            to: 12,
            kind: Comment,
        },
    ];
    assert_eq!(encode(&spans), [2, 3, 0, 0, 1, 2, 3, 3, 3]);
    assert_eq!(encode(&[]), Vec::<u32>::new());
}

#[test]
fn cache_parses_a_text_once_per_key_and_language() {
    let cache = Cache::new(1 << 20);
    let first = cache.get(&1u32, "fn f() {}", Language::Rust);
    assert!(!first.is_empty());
    // The same key answers from the cache, whatever text comes with it.
    let again = cache.get(&1, "something else entirely", Language::Rust);
    assert!(std::sync::Arc::ptr_eq(&first, &again));
    // Another language is another entry.
    let other = cache.get(&1, "fn f() {}", Language::Python);
    assert!(!std::sync::Arc::ptr_eq(&first, &other));
}

#[test]
fn cache_drops_the_entry_used_longest_ago() {
    let text = "fn f() { 1 }";
    let one = encode(&highlight(text, Language::Rust)).len() * 4;
    let cache = Cache::new(one * 2);
    let a = cache.get(&1u32, text, Language::Rust);
    cache.get(&2, text, Language::Rust);
    // Using 1 again makes 2 the oldest.
    cache.get(&1, text, Language::Rust);
    cache.get(&3, text, Language::Rust);
    assert!(cache.size() <= one * 2);
    assert!(std::sync::Arc::ptr_eq(&a, &cache.get(&1, "", Language::Rust)));
    // 2 was dropped: it is parsed again, from the text now given.
    assert!(cache.get(&2, "", Language::Rust).is_empty());
}

#[test]
fn cache_does_not_keep_what_exceeds_it() {
    let cache = Cache::new(4);
    let tokens = cache.get(&1u32, "fn f() { 1 }", Language::Rust);
    assert!(!tokens.is_empty());
    assert_eq!(cache.size(), 0);
}

/// The highlighted pieces of a signature with their kind.
fn signature_pieces(code: &str, language: Language) -> Vec<(String, Kind)> {
    let units: Vec<u16> = code.encode_utf16().collect();
    highlight_signature(code, language)
        .iter()
        .map(|s| {
            (
                String::from_utf16(&units[s.from as usize..s.to as usize]).unwrap(),
                s.kind,
            )
        })
        .collect()
}

#[test]
fn a_csharp_signature_shows_its_namespaces_apart_from_its_types() {
    let p = signature_pieces(
        "System.Collections.Generic.List<Line> Invoice.Merge(System.Collections.Generic.Dictionary<string, \
         System.Collections.Generic.List<Line>> groups, Task<Line> x)",
        Language::CSharp,
    );
    assert!(has(&p, "System.Collections.Generic.", Qualifier));
    assert!(has(&p, "List", Type));
    assert!(has(&p, "Dictionary", Type));
    assert!(has(&p, "Invoice.", Qualifier));
    assert!(has(&p, "Merge", Function));
    // Without a namespace there is nothing to dim.
    assert!(has(&p, "Task", Type));
    assert!(
        !p.iter()
            .any(|(t, k)| *k == Qualifier && (t.contains("Task") || t.contains("Merge")))
    );
}

#[test]
fn a_csharp_signature_is_offset_in_utf16_units_whatever_comes_before() {
    let code = "Zähler Acme.Billing.Core.Line Merge(Zähler z, Acme.Core.Line l)";
    let p = signature_pieces(code, Language::CSharp);
    assert!(has(&p, "Acme.Billing.Core.", Qualifier));
    assert!(has(&p, "Acme.Core.", Qualifier));
    assert!(has(&p, "Merge", Function));
}

#[test]
fn a_rust_signature_shows_its_modules_apart_from_its_types() {
    let p = signature_pieces(
        "fn merge(groups: std::collections::HashMap<String, std::vec::Vec<crate::billing::core::Line>>) \
         -> crate::billing::core::Line",
        Language::Rust,
    );
    assert!(has(&p, "std::collections::", Qualifier));
    assert!(has(&p, "std::vec::", Qualifier));
    assert!(has(&p, "crate::billing::core::", Qualifier));
    assert!(has(&p, "HashMap", Type));
    assert!(has(&p, "Vec", Type));
    assert!(has(&p, "Line", Type));
    assert!(has(&p, "merge", Function));
    assert!(has(&p, "fn", Keyword));
}

#[test]
fn a_typescript_signature_shows_its_namespaces_apart_from_its_types() {
    let p = signature_pieces(
        "Acme.Billing.Core.Invoice.merge(groups: Map<string, Array<Acme.Billing.Core.Line>>, \
         extra: Promise<import(\"./models\").Item>)",
        Language::TypeScript,
    );
    assert!(has(&p, "Acme.Billing.Core.Invoice.", Qualifier));
    assert!(has(&p, "merge", Function));
    assert!(has(&p, "Acme.Billing.Core.", Qualifier));
    assert!(has(&p, "Line", Type));
    assert!(has(&p, "Map", Type));
}

#[test]
fn a_python_signature_shows_its_modules_apart_from_its_types() {
    let p = signature_pieces(
        "def merge(self, groups: dict[str, list[Line]], other: billing.core.Line) -> billing.core.Line",
        Language::Python,
    );
    assert_eq!(
        p.iter()
            .filter(|(t, k)| t == "billing.core." && *k == Qualifier)
            .count(),
        2
    );
    assert!(!p.iter().any(|(t, k)| *k == Qualifier && t.contains("Line")));
}

#[test]
fn qualifiers_are_only_told_apart_in_signatures() {
    let code = "fn f(a: std::vec::Vec<u8>) {}";
    assert!(!pieces(code, Language::Rust).iter().any(|(_, k)| *k == Qualifier));
    assert!(
        signature_pieces(code, Language::Rust)
            .iter()
            .any(|(_, k)| *k == Qualifier)
    );
    // A language without qualified names has a signature like any other text.
    assert_eq!(
        highlight_signature("a: 1", Language::Yaml),
        highlight("a: 1", Language::Yaml)
    );
}

#[test]
fn an_empty_info_string_takes_the_default() {
    assert_eq!(Language::of_fence_or("", Some(Language::Rust)), Some(Language::Rust));
    assert_eq!(Language::of_fence_or("  ", None), None);
    assert_eq!(
        Language::of_fence_or("python", Some(Language::Rust)),
        Some(Language::Python)
    );
    assert_eq!(Language::of_fence_or("nope", Some(Language::Rust)), None);
}

#[test]
fn a_syntax_names_its_grammar() {
    assert_eq!(Syntax::File.language("a.py"), Some(Language::Python));
    assert_eq!(Syntax::Fence("rust".into()).language("a.py"), Some(Language::Rust));
    assert_eq!(Syntax::Fence(String::new()).language("a.py"), Some(Language::Python));
    assert_eq!(Syntax::Fence("nope".into()).language("a.py"), None);
    assert_eq!(Syntax::Plain.language("a.py"), None);
    assert_eq!(Syntax::File.language("Makefile"), None);
}

#[test]
fn fenced_blocks_of_markdown() {
    let md = "text\n\n```rust,ignore \nfn f() {}\n```\n\n    indented\n\n```\nplain\n\n```\n";
    assert_eq!(
        fenced_blocks(md),
        [
            Fence {
                info: "rust,ignore".into(),
                code: "fn f() {}\n".into()
            },
            Fence {
                info: String::new(),
                code: "plain\n\n".into()
            },
        ]
    );
}

#[test]
fn code_blocks_of_markdown() {
    let md = "```rust\nfn f() {}\n```\n\ntext\n\n```\nfn g() {}\n```\n\n```nope\nfn h() {}\n```\n";
    let blocks = code_blocks(md, Some(Language::Rust));
    assert_eq!(blocks.len(), 3);
    let first = encode(&highlight("fn f() {}\n", Language::Rust));
    assert!(!first.is_empty());
    assert_eq!(blocks[0], first);
    // Without a language the file's.
    assert_eq!(blocks[1], encode(&highlight("fn g() {}\n", Language::Rust)));
    // An unknown one stays plain.
    assert!(blocks[2].is_empty());
    // No file language: nothing to take.
    assert!(code_blocks("```\nfn g() {}\n```\n", None)[0].is_empty());
    // Indented code is not a fence.
    assert!(code_blocks("    fn g() {}\n", Some(Language::Rust)).is_empty());
}

#[test]
fn a_slice_of_the_highlights_is_those_of_its_text_whatever_comes_before_it() {
    // The first line has two-byte and two-unit characters, so units and bytes differ.
    let first = "let é😀 = 1; // ü\n";
    let line = "    fn f() -> u32 { 7 }";
    let file = format!("{first}{line}\n");
    let from = vet_core::text::unit_len(first);
    let to = from + vet_core::text::unit_len(line);
    let whole = encode(&highlight(&file, Language::Rust));
    assert_eq!(slice(&whole, from, to), encode(&highlight(line, Language::Rust)));
    // A range inside the line, and one that cuts a span in two.
    let keyword = slice(&whole, from + 4, from + 6);
    assert_eq!(keyword, [0, 2, Keyword as u32]);
    assert_eq!(
        slice(&whole, from + 5, from + 9),
        [0, 1, Keyword as u32, 1, 1, Function as u32]
    );
}
