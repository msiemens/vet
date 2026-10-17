//! The kept trees of every project: one [`KeptTrees`] per project, below that
//! project's directory in the data directory (see `storage::layout`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use super::kept::{Collected, KeptTrees, Referenced};
use crate::budget::{Budget, SEMANTIC_IO};
use crate::storage::layout::{PROJECTS, ROOT, SOURCES, project_dir};
use crate::sync::lock;

pub struct ProjectTrees {
    data: PathBuf,
    max_idle: Duration,
    referenced: Arc<dyn Referenced>,
    io: Budget,
    /// By the `sources` directory, so a set found on disk is the one a project opened.
    sets: Mutex<HashMap<PathBuf, Arc<KeptTrees>>>,
}

impl ProjectTrees {
    pub fn new(data: PathBuf, max_idle: Duration, referenced: Arc<dyn Referenced>, io: Budget) -> Self {
        Self {
            data,
            max_idle,
            referenced,
            io,
            sets: Mutex::default(),
        }
    }

    /// Trees with a budget of their own, for tests.
    #[doc(hidden)]
    pub fn for_test(data: PathBuf, max_idle: Duration, referenced: Arc<dyn Referenced>) -> Self {
        Self::new(data, max_idle, referenced, Budget::new(SEMANTIC_IO))
    }

    fn set(&self, sources: PathBuf) -> Arc<KeptTrees> {
        lock(&self.sets)
            .entry(sources.clone())
            .or_insert_with(|| {
                Arc::new(KeptTrees::new(
                    sources,
                    self.max_idle,
                    self.referenced.clone(),
                    self.io.clone(),
                ))
            })
            .clone()
    }

    /// The trees of the project whose repository root is `root`. The project's `root` file
    /// is written here, so that the directory says whose it is.
    pub async fn of(&self, root: &str) -> Arc<KeptTrees> {
        let dir = project_dir(&self.data, root);
        let set = self.set(dir.join(SOURCES));
        let root = root.to_owned();
        let _ = self.io.blocking(move || write_root(&dir, &root)).await;
        set
    }

    /// The trees of every project that has a directory.
    async fn all(&self) -> Vec<Arc<KeptTrees>> {
        let projects = self.data.join(PROJECTS);
        let found = self
            .io
            .blocking(move || -> Vec<PathBuf> {
                std::fs::read_dir(projects)
                    .map(|entries| {
                        entries
                            .flatten()
                            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                            .map(|e| e.path().join(SOURCES))
                            .filter(|sources| sources.is_dir())
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .await
            .unwrap_or_default();
        found.into_iter().map(|sources| self.set(sources)).collect()
    }

    /// Removes what launches that died left in every project; see [`KeptTrees::sweep`].
    pub async fn sweep(&self) -> usize {
        let mut removed = 0;
        for set in self.all().await {
            removed += set.sweep().await;
        }
        removed
    }

    /// [`KeptTrees::collect`] in every project.
    pub async fn collect(&self, now: SystemTime) -> Collected {
        let mut done = Collected::default();
        for set in self.all().await {
            done += set.collect(now).await;
        }
        done
    }

    /// [`KeptTrees::clear`] in every project.
    pub async fn clear(&self) -> Collected {
        let mut done = Collected::default();
        for set in self.all().await {
            done += set.clear().await;
        }
        done
    }
}

fn write_root(dir: &Path, root: &str) {
    let marker = dir.join(ROOT);
    if std::fs::read_to_string(&marker).is_ok_and(|known| known == root) {
        return;
    }
    let _ = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(marker, root));
}
