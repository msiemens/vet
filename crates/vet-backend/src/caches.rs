//! The answers of the language servers that outlive a launch: one file for all projects,
//! opened by the first project that has semantics.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::OnceCell;
use vet_infra::semantic_cache::{SqliteSemanticCache, size_on_disk};
use vet_semantic::reuse::SemanticCache;

pub struct PersistentAnswers {
    dir: PathBuf,
    /// The bytes the file may hold, for when it opens.
    budget: AtomicU64,
    file: OnceCell<Option<Arc<SqliteSemanticCache>>>,
}

impl PersistentAnswers {
    pub fn new(dir: PathBuf, budget: u64) -> Self {
        Self {
            dir,
            budget: AtomicU64::new(budget),
            file: OnceCell::new(),
        }
    }

    async fn file(&self) -> Option<&Arc<SqliteSemanticCache>> {
        self.file
            .get_or_init(|| async {
                match SqliteSemanticCache::open(&self.dir, self.budget.load(Ordering::Relaxed)).await {
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

    /// Holds the file to `budget` bytes from now on, and at once.
    pub async fn set_budget(&self, budget: u64) {
        self.budget.store(budget, Ordering::Relaxed);
        if let Some(file) = self.file().await {
            file.set_budget(budget).await;
        }
    }

    /// The bytes the file takes on disk.
    pub fn usage(&self) -> u64 {
        size_on_disk(&self.dir)
    }
}
