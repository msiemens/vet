use std::sync::OnceLock;

use tree_sitter_highlight::HighlightConfiguration;

use crate::Kind;

/// A language with a grammar and a highlight query.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Language {
    Rust,
    Python,
    TypeScript,
    Tsx,
    /// Also JSX: the grammar parses both.
    JavaScript,
    Json,
    Markdown,
    /// The inline half of Markdown; only reached by injection from the block grammar.
    MarkdownInline,
    Yaml,
    Toml,
    Bash,
    CSharp,
    Css,
    Html,
}

impl Language {
    pub(crate) const ALL: [Language; 14] = [
        Self::Rust,
        Self::Python,
        Self::TypeScript,
        Self::Tsx,
        Self::JavaScript,
        Self::Json,
        Self::Markdown,
        Self::MarkdownInline,
        Self::Yaml,
        Self::Toml,
        Self::Bash,
        Self::CSharp,
        Self::Css,
        Self::Html,
    ];

    /// The language of a file by its extension; `None` for any other file.
    pub fn of_path(path: &str) -> Option<Self> {
        let name = path.rsplit('/').next().unwrap_or(path);
        let (_, extension) = name.rsplit_once('.')?;
        Some(match extension.to_ascii_lowercase().as_str() {
            "rs" => Self::Rust,
            "py" | "pyi" => Self::Python,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "json" => Self::Json,
            "md" | "markdown" => Self::Markdown,
            "yaml" | "yml" => Self::Yaml,
            "toml" => Self::Toml,
            "sh" | "bash" => Self::Bash,
            "cs" | "csx" => Self::CSharp,
            "css" => Self::Css,
            "html" | "htm" | "svg" => Self::Html,
            _ => return None,
        })
    }

    /// The language a Markdown code fence names in its info string (`rust`, `rust,ignore`,
    /// `ts {1}`); `None` for a missing or unknown name.
    pub fn of_fence(info: &str) -> Option<Self> {
        let name = info.trim().split([' ', '\t', ',', '{']).next().unwrap_or_default();
        Some(match name.to_ascii_lowercase().as_str() {
            "rust" | "rs" => Self::Rust,
            "python" | "py" => Self::Python,
            "typescript" | "ts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "javascript" | "js" | "jsx" => Self::JavaScript,
            "csharp" | "cs" | "c#" => Self::CSharp,
            "json" => Self::Json,
            "yaml" | "yml" => Self::Yaml,
            "toml" => Self::Toml,
            "sh" | "bash" | "shell" | "zsh" => Self::Bash,
            "css" => Self::Css,
            "html" => Self::Html,
            "markdown" | "md" => Self::Markdown,
            "markdown_inline" => Self::MarkdownInline,
            _ => return None,
        })
    }

    /// The highlight configuration, built on first use.
    pub(crate) fn config(self) -> &'static HighlightConfiguration {
        static CONFIGS: [OnceLock<HighlightConfiguration>; Language::ALL.len()] =
            [const { OnceLock::new() }; Language::ALL.len()];
        CONFIGS[self as usize].get_or_init(|| {
            let mut config = self.build();
            config.configure(&RECOGNIZED.map(|(name, _)| name));
            config
        })
    }

    fn build(self) -> HighlightConfiguration {
        let (language, highlights, injections) = match self {
            Self::Rust => (
                tree_sitter_rust::LANGUAGE,
                tree_sitter_rust::HIGHLIGHTS_QUERY.to_owned(),
                "",
            ),
            Self::Python => (
                tree_sitter_python::LANGUAGE,
                tree_sitter_python::HIGHLIGHTS_QUERY.to_owned(),
                "",
            ),
            Self::TypeScript => (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
                format!(
                    "{}\n{}",
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_typescript::HIGHLIGHTS_QUERY
                ),
                "",
            ),
            Self::Tsx => (
                tree_sitter_typescript::LANGUAGE_TSX,
                format!(
                    "{}\n{}\n{}",
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                    tree_sitter_typescript::HIGHLIGHTS_QUERY
                ),
                "",
            ),
            Self::JavaScript => (
                tree_sitter_javascript::LANGUAGE,
                format!(
                    "{}\n{}",
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
                ),
                "",
            ),
            // A later pattern wins over an earlier one for the same node: keys stay plain.
            Self::Json => (
                tree_sitter_json::LANGUAGE,
                format!(
                    "{}\n(pair key: (_) @string.special.key)",
                    tree_sitter_json::HIGHLIGHTS_QUERY
                ),
                "",
            ),
            Self::Markdown => (
                tree_sitter_md::LANGUAGE,
                tree_sitter_md::HIGHLIGHT_QUERY_BLOCK.to_owned(),
                MARKDOWN_INJECTIONS,
            ),
            Self::MarkdownInline => (
                tree_sitter_md::INLINE_LANGUAGE,
                tree_sitter_md::HIGHLIGHT_QUERY_INLINE.to_owned(),
                "",
            ),
            Self::Yaml => (
                tree_sitter_yaml::LANGUAGE,
                tree_sitter_yaml::HIGHLIGHTS_QUERY.to_owned(),
                "",
            ),
            Self::Toml => (
                tree_sitter_toml_ng::LANGUAGE,
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY.to_owned(),
                "",
            ),
            Self::Bash => (
                tree_sitter_bash::LANGUAGE,
                tree_sitter_bash::HIGHLIGHT_QUERY.to_owned(),
                "",
            ),
            Self::CSharp => (
                tree_sitter_c_sharp::LANGUAGE,
                tree_sitter_c_sharp::HIGHLIGHTS_QUERY.to_owned(),
                "",
            ),
            Self::Css => (
                tree_sitter_css::LANGUAGE,
                tree_sitter_css::HIGHLIGHTS_QUERY.to_owned(),
                "",
            ),
            Self::Html => (
                tree_sitter_html::LANGUAGE,
                tree_sitter_html::HIGHLIGHTS_QUERY.to_owned(),
                tree_sitter_html::INJECTIONS_QUERY,
            ),
        };
        HighlightConfiguration::new(language.into(), format!("{self:?}"), &highlights, injections, "")
            .unwrap_or_else(|error| panic!("the highlight query of {self:?} does not fit its grammar: {error}"))
    }
}

/// The grammar's injections, with the inline content taken whole: its children would otherwise be
/// left out of the inline parse.
const MARKDOWN_INJECTIONS: &str = r#"
(fenced_code_block
  (info_string (language) @injection.language)
  (code_fence_content) @injection.content)
((html_block) @injection.content (#set! injection.language "html"))
((minus_metadata) @injection.content (#set! injection.language "yaml"))
((plus_metadata) @injection.content (#set! injection.language "toml"))
((inline) @injection.content
  (#set! injection.language "markdown_inline")
  (#set! injection.include-children))
"#;

/// The capture names the queries use that are shown, with what they become; a capture takes the
/// entry with the longest matching prefix (`function.method` the `function` one). `None` leaves a
/// range plain although an enclosing capture is shown.
pub(crate) const RECOGNIZED: [(&str, Option<Kind>); 19] = [
    ("keyword", Some(Kind::Keyword)),
    ("string", Some(Kind::String)),
    ("string.special.key", None),
    ("escape", Some(Kind::String)),
    ("number", Some(Kind::Number)),
    ("boolean", Some(Kind::Number)),
    ("constant.builtin", Some(Kind::Number)),
    ("comment", Some(Kind::Comment)),
    ("type", Some(Kind::Type)),
    ("module", Some(Kind::Type)),
    ("namespace", Some(Kind::Type)),
    ("constructor", Some(Kind::Type)),
    ("function", Some(Kind::Function)),
    ("attribute", Some(Kind::Attribute)),
    ("tag", Some(Kind::Attribute)),
    ("text.title", Some(Kind::Heading)),
    ("text.strong", Some(Kind::Heading)),
    ("text.uri", Some(Kind::Link)),
    ("text.reference", Some(Kind::Link)),
];
