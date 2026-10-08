//! What keeps a kept materialized tree from being collected besides being in
//! use (§8), read from the store.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;

use vet_app::gc::open_bases;
use vet_core::content::SnapshotId;
use vet_infra::materialize::kept::Referenced;
use vet_infra::store::SqliteStore;

/// The bases of the latest versions of the open changesets. When the store cannot
/// be read nothing is held back: a tree that goes is built again.
pub struct StoreReferences(pub SqliteStore);

impl Referenced for StoreReferences {
    fn snapshots(&self) -> Pin<Box<dyn Future<Output = HashSet<SnapshotId>> + Send + '_>> {
        Box::pin(async move {
            open_bases(&self.0).await.unwrap_or_else(|e| {
                tracing::warn!(error = %e.0, "cannot read which kept trees are referenced");
                HashSet::new()
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use vet_app::changeset::{BaseChoice, CommitId, Source, Version, VersionChange, VersionOrigin};
    use vet_app::ports::{LiveStore, ReviewWrites};
    use vet_core::content::Hash;
    use vet_infra::materialize::kept::{KeptTrees, Limits};
    use vet_semantic::policy::AnalysisPolicy;

    use super::*;

    const POLICY: AnalysisPolicy = AnalysisPolicy { allow_executing: false };

    fn snapshot(n: u8) -> SnapshotId {
        SnapshotId(Hash([n; 32]))
    }

    async fn keep(trees: &KeptTrees, snapshot: SnapshotId) -> std::path::PathBuf {
        let built = trees
            .acquire(snapshot, POLICY, |dir| async move {
                std::fs::create_dir_all(&dir)?;
                std::fs::write(dir.join("file"), "x")
            })
            .await
            .unwrap();
        built.lease.tree().to_owned()
    }

    #[tokio::test]
    async fn the_tree_of_an_open_changesets_base_survives_a_collection_that_takes_the_idle_others() {
        let data = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(data.path()).await.unwrap();
        let commit = CommitId("abc123".into());
        let version = Version {
            n: 1,
            base: snapshot(1),
            base_commit: commit.clone(),
            target: snapshot(2),
            tip: commit.clone(),
            at: 0,
            origin: VersionOrigin::Settled,
            change: VersionChange::default(),
        };
        store
            .writes()
            .start_review(
                "/repo".into(),
                Source::WorkingTree,
                BaseChoice::Commit { id: commit },
                version,
                0,
            )
            .await
            .unwrap();
        let limits = Limits {
            max_idle: Duration::ZERO,
            ..Limits::default()
        };
        let trees = KeptTrees::new(
            data.path().join("materialized"),
            limits,
            std::sync::Arc::new(StoreReferences(store)),
        );
        let referenced = keep(&trees, snapshot(1)).await;
        let idle = keep(&trees, snapshot(9)).await;

        let later = SystemTime::now() + Duration::from_secs(3600);
        let collected = trees.collect(later).await;

        assert_eq!(collected.removed, 1);
        assert!(referenced.is_dir());
        assert!(!idle.exists());
    }
}
