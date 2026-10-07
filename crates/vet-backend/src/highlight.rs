//! Syntax highlighting for what the backend sends: a file's texts and a hover's code blocks.
//! A text is parsed once per content, whatever the file it is in, and off the async workers.

use std::sync::Arc;

use vet_core::content::BlobId;
use vet_highlight::{Cache, Language};
use vet_semantic::signature::{Signature, Syntax};

/// What the cache of highlights may hold, in bytes.
const CAPACITY: usize = 64 << 20;

#[derive(Clone)]
pub struct Highlighter {
    cache: Arc<Cache<BlobId>>,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self {
            cache: Arc::new(Cache::new(CAPACITY)),
        }
    }
}

impl Highlighter {
    /// The highlights of `text`, a file's content at `path`; empty if there is no text or its
    /// language has no grammar.
    pub async fn text(&self, path: &str, text: Option<&str>) -> Vec<u32> {
        let (Some(text), Some(language)) = (text, Language::of_path(path)) else {
            return Vec::new();
        };
        let (cache, text) = (self.cache.clone(), text.to_owned());
        tokio::task::spawn_blocking(move || cache.get(&BlobId::of(text.as_bytes()), &text, language).to_vec())
            .await
            .unwrap_or_default()
    }
}

/// The highlights of the signature of a symbol of the file at `path`; empty for prose and for a
/// language without a grammar.
pub fn signature(path: &str, signature: &Signature) -> Vec<u32> {
    let language = match &signature.syntax {
        Syntax::File => Language::of_path(path),
        Syntax::Fence(info) if info.is_empty() => Language::of_path(path),
        Syntax::Fence(info) => Language::of_fence(info),
        Syntax::Plain => None,
    };
    language.map_or_else(Vec::new, |l| {
        vet_highlight::encode(&vet_highlight::highlight(&signature.code, l))
    })
}

/// The highlights of the code blocks of a hover over the file at `path`.
pub fn code_blocks(path: &str, markdown: &str) -> Vec<Vec<u32>> {
    vet_highlight::code_blocks(markdown, Language::of_path(path))
}
