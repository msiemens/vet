//! The responses that carry highlighted text: a file's texts, a file's target text, a hover and a
//! signature. A text is parsed once per content, whatever the file it is in, and off the async
//! workers.

use std::sync::Arc;

use vet_app::session::{FileText, TargetText};
use vet_core::content::BlobId;
use vet_highlight::{Cache, Language};
use vet_infra::budget::Budget;
use vet_protocol::{Response, Side, SymbolSignature, Unavailable};
use vet_semantic::signature::Signature;

use vet_mapping as mapping;

/// What the cache of highlights may hold, in bytes.
const CAPACITY: usize = 64 << 20;

#[derive(Clone)]
pub struct Highlighter {
    cache: Arc<Cache<BlobId>>,
    /// What parsing draws on: it answers a view of a file, so it is the review lane's.
    io: Budget,
}

impl Highlighter {
    pub fn new(io: Budget) -> Self {
        Self {
            cache: Arc::new(Cache::new(CAPACITY)),
            io,
        }
    }

    /// The highlights of `text`, a file's content at `path`; empty if there is no text or its
    /// language has no grammar.
    async fn text(&self, path: &str, text: Option<&str>) -> Vec<u32> {
        let (Some(text), Some(language)) = (text, Language::of_path(path)) else {
            return Vec::new();
        };
        let (cache, text) = (self.cache.clone(), text.to_owned());
        self.io
            .blocking(move || cache.get(&BlobId::of(text.as_bytes()), &text, language).to_vec())
            .await
            .unwrap_or_default()
    }

    pub async fn file_contents(&self, path: &str, text: FileText) -> Response {
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

    pub async fn file_text(&self, path: &str, text: &TargetText) -> Response {
        let highlights = self.text(path, text.content.as_deref().ok()).await;
        mapping::target_text(text, highlights)
    }
}

/// A hover over the file at `path`, with the highlights of the code blocks in its text.
pub fn hover(
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
pub fn signature(
    path: &str,
    revision: u32,
    signature: Option<Signature>,
    unavailable: Option<Unavailable>,
) -> Response {
    let signature = signature.map(|s| SymbolSignature {
        highlights: (s.syntax.language(path)).map_or_else(Vec::new, |l| {
            vet_highlight::encode(&vet_highlight::highlight(&s.code, l))
        }),
        code: s.code,
    });
    Response::Signature {
        revision,
        signature,
        unavailable,
    }
}
