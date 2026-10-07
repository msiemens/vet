//! Syntax highlighting for what the backend sends: a file's texts and a hover's code blocks.
//! A text is parsed once per content, whatever the file it is in, and off the async workers.

use std::sync::Arc;

use vet_core::content::BlobId;
use vet_highlight::{Cache, Language};

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

/// The highlights of the code blocks of a hover over the file at `path`.
pub fn code_blocks(path: &str, markdown: &str) -> Vec<Vec<u32>> {
    vet_highlight::code_blocks(markdown, Language::of_path(path))
}
