use super::*;
use Kind::{Attribute, Comment, Function, Heading, Keyword, Link, Number, String as Str, Type};

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
    let mut kinds = [Keyword, Str, Number, Comment, Type, Function, Attribute, Heading, Link];
    kinds.sort_by_key(|&k| k as u8);
    assert!(
        kinds.iter().enumerate().all(|(i, &k)| k as usize == i),
        "wire numbers have gaps"
    );
    let names = kinds.map(|k| format!("{k:?}").to_lowercase());

    let protocol = repository_file("../vet-protocol/src/lib.rs");
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
fn rust() {
    let p = pieces("fn main() { let n: u32 = 7; // done\n}", Language::Rust);
    assert!(has(&p, "fn", Keyword));
    assert!(has(&p, "main", Function));
    assert!(has(&p, "u32", Type));
    assert!(has(&p, "7", Number));
    assert!(has(&p, "// done", Comment));
}

#[test]
fn python() {
    let p = pieces("def f(x):\n    return \"s\" + 1  # c\n", Language::Python);
    assert!(has(&p, "def", Keyword));
    assert!(has(&p, "f", Function));
    assert!(has(&p, "\"s\"", Str));
    assert!(has(&p, "1", Number));
    assert!(has(&p, "# c", Comment));
}

#[test]
fn typescript_and_tsx() {
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
fn javascript_and_jsx() {
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
fn csharp() {
    let p = pieces("class A { public int N = 1; } // c\n", Language::CSharp);
    assert!(has(&p, "class", Keyword));
    assert!(has(&p, "1", Number));
    assert!(has(&p, "// c", Comment));
}

#[test]
fn json() {
    let p = pieces("{\"k\": [true, null, 1.5, \"v\"]}", Language::Json);
    assert!(has(&p, "true", Number));
    assert!(has(&p, "null", Number));
    assert!(has(&p, "1.5", Number));
    assert!(has(&p, "\"v\"", Str));
    // A key is not coloured as a string.
    assert!(!has(&p, "\"k\"", Str));
}

#[test]
fn yaml() {
    let p = pieces("a: 1\nb: \"s\" # c\nc: true\n", Language::Yaml);
    assert!(has(&p, "1", Number));
    assert!(has(&p, "\"s\"", Str));
    assert!(has(&p, "# c", Comment));
    assert!(has(&p, "true", Number));
}

#[test]
fn toml() {
    let p = pieces("[package]\nname = \"vet\" # c\nn = 3\nok = true\n", Language::Toml);
    assert!(has(&p, "\"vet\"", Str));
    assert!(has(&p, "3", Number));
    assert!(has(&p, "true", Number));
    assert!(has(&p, "# c", Comment));
}

#[test]
fn bash() {
    let p = pieces("if [ -f x ]; then echo \"hi\"; fi # c\n", Language::Bash);
    assert!(has(&p, "if", Keyword));
    assert!(has(&p, "\"hi\"", Str));
    assert!(has(&p, "# c", Comment));
}

#[test]
fn css() {
    let p = pieces(
        "a.b { color: #fff; margin: 0 } /* c */\n@media print {}\n",
        Language::Css,
    );
    assert!(has(&p, "/* c */", Comment));
    assert!(has(&p, "0", Number));
}

#[test]
fn html_with_script_and_style() {
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
fn markdown_with_inline_and_fenced_code() {
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
fn crlf_is_one_unit() {
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
fn languages_by_path() {
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
fn fence_names() {
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
    }
}

#[test]
fn encoding_is_gap_length_kind() {
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
