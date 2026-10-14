//! The answers of the language servers that outlive a launch: one file for all projects,
//! opened by the first project that has semantics.

use std::sync::Arc;

use tokio::sync::OnceCell;
use vet_app::ports::SettingsStore;
use vet_infra::semantic_cache::{SqliteSemanticCache, size_on_disk};
use vet_infra::store::SqliteStore;
use vet_semantic::reuse::SemanticCache;

pub(crate) struct PersistentAnswers {
    /// Where the file lives, and the settings that say how much it may hold when it opens.
    store: SqliteStore,
    file: OnceCell<Option<Arc<SqliteSemanticCache>>>,
}

impl PersistentAnswers {
    pub(crate) fn new(store: SqliteStore) -> Self {
        Self {
            store,
            file: OnceCell::new(),
        }
    }

    async fn file(&self) -> Option<&Arc<SqliteSemanticCache>> {
        self.file
            .get_or_init(|| async {
                let budget = match self.store.backend_settings().await {
                    Ok(settings) => settings.semantic_cache_bytes(),
                    Err(why) => {
                        tracing::warn!(error = %why.0, "no semantic cache between launches");
                        return None;
                    }
                };
                match SqliteSemanticCache::open(self.store.dir(), budget).await {
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
    pub(crate) async fn store(&self) -> Option<Arc<dyn SemanticCache>> {
        Some(self.file().await?.clone())
    }

    /// Removes every stored answer.
    pub(crate) async fn clear(&self) {
        if let Some(file) = self.file().await {
            file.clear().await;
        }
    }

    /// Holds the file to `budget` bytes at once. The settings the store keeps already say so.
    pub(crate) async fn set_budget(&self, budget: u64) {
        if let Some(file) = self.file().await {
            file.set_budget(budget).await;
        }
    }

    /// The bytes the file takes on disk.
    pub(crate) fn usage(&self) -> u64 {
        size_on_disk(self.store.dir())
    }
}
