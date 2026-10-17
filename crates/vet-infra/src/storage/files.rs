//! The project directories below `<data>/projects` (see [`super::layout`]), read and removed by
//! their root.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use vet_app::ports::{ProjectFiles, SharedFiles, TreeUsage};

use super::layout::{PROJECTS, ROOT, project_dir};
use crate::budget::Budget;
use crate::materialize::kept::tree_size;
use crate::materialize::projects::ProjectTrees;
use crate::store;

/// Where a removed project's directory waits to be deleted.
const TRASH: &str = ".trash";

#[derive(Clone)]
pub struct DataFiles {
    data: PathBuf,
    trees: Arc<ProjectTrees>,
    io: Budget,
}

impl DataFiles {
    /// The files below the data directory `data`, read on `io`'s threads; `trees` says what its
    /// kept source trees take.
    pub fn new(data: PathBuf, trees: Arc<ProjectTrees>, io: Budget) -> Self {
        Self { data, trees, io }
    }

    async fn blocking<T: Send + 'static>(&self, work: impl FnOnce(&Path) -> T + Send + 'static) -> Option<T> {
        let data = self.data.clone();
        self.io.blocking(move || work(&data)).await.ok()
    }
}

/// Moves the project's directory out of service and deletes it; what stays is the sweep's.
fn remove_project(data: &Path, root: &str) {
    let dir = project_dir(data, root);
    if !dir.exists() {
        return;
    }
    let trash = data.join(PROJECTS).join(TRASH);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let name = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let moved = trash.join(format!("{nanos}-{name}"));
    let renamed = std::fs::create_dir_all(&trash).and_then(|()| std::fs::rename(&dir, &moved));
    let doomed = if renamed.is_ok() { moved } else { dir };
    if let Err(e) = std::fs::remove_dir_all(&doomed) {
        tracing::warn!(dir = %doomed.display(), error = %e, "a removed project's files could not be deleted");
    }
}

impl ProjectFiles for DataFiles {
    async fn roots(&self) -> Vec<String> {
        self.blocking(|data| {
            let Ok(entries) = std::fs::read_dir(data.join(PROJECTS)) else {
                return Vec::new();
            };
            entries
                .flatten()
                .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
                .filter_map(|entry| std::fs::read_to_string(entry.path().join(ROOT)).ok())
                .collect()
        })
        .await
        .unwrap_or_default()
    }

    async fn trees(&self, root: &str) -> TreeUsage {
        self.trees.at(root).usage(SystemTime::now()).await
    }

    async fn remove(&self, root: &str) {
        let root = root.to_owned();
        self.blocking(move |data| remove_project(data, &root)).await;
    }

    async fn sweep(&self) {
        self.blocking(|data| {
            let trash = data.join(PROJECTS).join(TRASH);
            for entry in std::fs::read_dir(trash).into_iter().flatten().flatten() {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        })
        .await;
    }

    async fn shared(&self) -> SharedFiles {
        self.blocking(|data| SharedFiles {
            language_servers: tree_size(&data.join("servers")),
            databases: store::size_on_disk(data),
        })
        .await
        .unwrap_or_default()
    }

    async fn folder_exists(&self, root: &str) -> bool {
        let root = PathBuf::from(root);
        self.io.blocking(move || root.is_dir()).await.unwrap_or(false)
    }
}
