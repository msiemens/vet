//! The answers of the language servers that outlive a launch: one file for all projects,
//! opened by the first project that has semantics.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::OnceCell;
use vet_infra::semantic_cache::SqliteSemanticCache;
use vet_semantic::reuse::SemanticCache;

pub struct PersistentAnswers {
    dir: PathBuf,
    file: OnceCell<Option<Arc<SqliteSemanticCache>>>,
}

impl PersistentAnswers {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            file: OnceCell::new(),
        }
    }

    async fn file(&self) -> Option<&Arc<SqliteSemanticCache>> {
        self.file
            .get_or_init(|| async {
                match SqliteSemanticCache::open(&self.dir).await {
                    Ok(cache) => Some(Arc::new(cache)),
                    // Only speed is lost without it.
                    Err(why) => {
                        tracing::warn!(error = %why, "no semantic cache between launches");
                        None
                    }
                }
            })
            .await
            .as_ref()
    }

    /// The store for a project's answers; none if the file cannot be had.
    pub async fn store(&self) -> Option<Arc<dyn SemanticCache>> {
        Some(self.file().await?.clone())
    }

    /// Removes every stored answer.
    pub async fn clear(&self) {
        if let Some(file) = self.file().await {
            file.clear().await;
        }
    }
}
