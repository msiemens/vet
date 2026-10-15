//! The responses that carry highlighted text: a file's texts, a file's target text, a hover and a
//! signature. A text is parsed once per content, whatever the file it is in, and off the async
//! workers.

use std::sync::Arc;

use vet_app::session::{FileText, TargetText};
use vet_core::content::{BlobId, RepoPath};
use vet_highlight::{Cache, Language};
use vet_infra::budget::Budget;
use vet_protocol::{Response, Side, SymbolSignature, Unavailable};
use vet_semantic::signature::Signature;

use vet_mapping as mapping;

/// What the cache of highlights may hold, in bytes.
const CAPACITY: usize = 64 << 20;

#[derive(Clone)]
pub(crate) struct Highlighter {
    cache: Arc<Cache<BlobId>>,
    /// What parsing draws on: it answers a view of a file, so it is the review lane's.
    io: Budget,
}

/// The sources a list of places reads beyond the session: a file outside the repository a
/// language server named (a dependency's or toolchain's source) and the highlighting of a text.
/// It answers for `vet_app::semantics`'s pieces that name them.
#[derive(Clone)]
pub(crate) struct Sources {
    highlighter: Highlighter,
}

impl vet_app::ports::LineSources for Sources {
    /// Only a source file of the language `language` that named `file` is read: library and
    /// toolchain sources have the extension of their language, a key, a dotfile or another
    /// tool's configuration does not.
    fn serves(&self, language: &str, file: &RepoPath) -> bool {
        vet_infra::language::is_source_of(language, file)
    }

    /// The text of `file` outside the repository; `None` over `max_bytes` or none to read.
    async fn outside(&self, file: &RepoPath, max_bytes: u64) -> Option<String> {
        let path = std::path::PathBuf::from(file.to_string_lossy());
        if tokio::fs::metadata(&path).await.ok()?.len() > max_bytes {
            return None;
        }
        let bytes = tokio::fs::read(&path).await.ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// `text` is the content of the file at `path`.
    async fn highlights(&self, path: &str, text: &str) -> Vec<u32> {
        self.highlighter.text(path, Some(text)).await
    }

    /// The wire form the highlighting carries, cut by [`vet_highlight::slice`].
    fn cut(&self, highlights: &[u32], from: u32, to: u32) -> Vec<u32> {
        vet_highlight::slice(highlights, from, to)
    }
}

impl Sources {
    pub(crate) fn new(highlighter: Highlighter) -> Self {
        Self { highlighter }
    }
}

impl Highlighter {
    pub(crate) fn new(io: Budget) -> Self {
        Self {
            cache: Arc::new(Cache::new(CAPACITY)),
            io,
        }
    }

    /// The highlights of `text`, a file's content at `path`; empty if there is no text or its
    /// language has no grammar.
    pub(crate) async fn text(&self, path: &str, text: Option<&str>) -> Vec<u32> {
        let (Some(text), Some(language)) = (text, Language::of_path(path)) else {
            return Vec::new();
        };
        let (cache, text) = (self.cache.clone(), text.to_owned());
        self.io
            .blocking(move || cache.get(&BlobId::of(text.as_bytes()), &text, language).to_vec())
            .await
            .unwrap_or_default()
    }

    pub(crate) async fn file_contents(&self, path: &str, text: FileText) -> Response {
        let (base_highlights, target_highlights) = tokio::join!(
            self.text(path, text.base.as_deref()),
            self.text(path, text.target.as_deref())
        );
        Response::FileContents {
            revision: text.revision,
            base: text.base,
            target: text.target,
            changes: text.changes.iter().map(mapping::text_change).collect(),
            base_highlights,
            target_highlights,
        }
    }

    pub(crate) async fn file_text(&self, path: &str, text: &TargetText) -> Response {
        let highlights = self.text(path, text.content.as_deref().ok()).await;
        mapping::target_text(text, highlights)
    }
}

/// A hover over the file at `path`, with the highlights of the code blocks in its text.
pub(crate) fn hover(
    path: &str,
    revision: u32,
    side: Side,
    text: Option<String>,
    unavailable: Option<Unavailable>,
) -> Response {
    let code_highlights = text
        .as_deref()
        .map_or_else(Vec::new, |t| vet_highlight::code_blocks(t, Language::of_path(path)));
    Response::Hover {
        revision,
        side,
        text,
        code_highlights,
        unavailable,
    }
}

/// The signature of a symbol of the file at `path`; its highlights are empty for prose and for
/// a language without a grammar.
pub(crate) fn signature(
    path: &str,
    revision: u32,
    signature: Option<Signature>,
    unavailable: Option<Unavailable>,
) -> Response {
    let signature = signature.map(|s| SymbolSignature {
        highlights: (s.syntax.language(path)).map_or_else(Vec::new, |l| {
            vet_highlight::encode(&vet_highlight::highlight_signature(&s.code, l))
        }),
        code: s.code,
    });
    Response::Signature {
        revision,
        signature,
        unavailable,
    }
}
