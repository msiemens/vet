//! `vet_app::ports::RunStore`: run targets and run approvals, per repo (§7.5, §10).

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
    TransactionTrait,
};
use vet_app::execution::targets::RunTarget;
use vet_app::ports::{RunStore, StoreError};
use vet_core::content::{Hash, SnapshotId};

use super::SqliteStore;
use super::codec::hash;

/// A user-edited or user-added target, unique per `(repo, target_id)`. `args`
/// and `env` are JSON.
pub mod run_target {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "run_target")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        #[sea_orm(unique_key = "repo_target")]
        pub repo: String,
        #[sea_orm(unique_key = "repo_target")]
        pub target_id: String,
        pub label: String,
        pub program: String,
        pub args: String,
        pub cwd: String,
        pub env: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// The last approval of a repo; `baseline` is a stored snapshot of the
/// executor-relevant files at that time.
pub mod run_approval {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "run_approval")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        #[sea_orm(unique)]
        pub repo: String,
        pub hash: Vec<u8>,
        pub baseline: Vec<u8>,
        /// Unix seconds.
        pub at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Snapshots GC must keep: the approval baselines.
pub async fn approved_baselines(db: &impl ConnectionTrait) -> Result<Vec<Vec<u8>>, DbErr> {
    run_approval::Entity::find()
        .select_only()
        .column(run_approval::Column::Baseline)
        .into_tuple()
        .all(db)
        .await
}

fn target_from(m: run_target::Model) -> Result<RunTarget, DbErr> {
    let json = |e: serde_json::Error| DbErr::Custom(format!("corrupt run target: {e}"));
    Ok(RunTarget {
        id: m.target_id,
        source: None,
        label: m.label,
        program: m.program,
        args: serde_json::from_str(&m.args).map_err(json)?,
        cwd: m.cwd,
        env: serde_json::from_str(&m.env).map_err(json)?,
    })
}

impl RunStore for SqliteStore {
    async fn run_targets(&self, repo: String) -> Result<Vec<RunTarget>, StoreError> {
        let rows = run_target::Entity::find()
            .filter(run_target::Column::Repo.eq(repo))
            .order_by_asc(run_target::Column::Id)
            .all(&self.db)
            .await?;
        Ok(rows.into_iter().map(target_from).collect::<Result<_, _>>()?)
    }

    async fn put_run_target(&self, repo: String, target: RunTarget) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        let old = run_target::Entity::find()
            .filter(run_target::Column::Repo.eq(repo.clone()))
            .filter(run_target::Column::TargetId.eq(target.id.clone()))
            .one(&txn)
            .await?;
        let json = |e: serde_json::Error| DbErr::Custom(e.to_string());
        let row = run_target::ActiveModel {
            id: old.map_or(Default::default(), |o| Set(o.id)),
            repo: Set(repo),
            target_id: Set(target.id),
            label: Set(target.label),
            program: Set(target.program),
            args: Set(serde_json::to_string(&target.args).map_err(json)?),
            cwd: Set(target.cwd),
            env: Set(serde_json::to_string(&target.env).map_err(json)?),
        };
        if row.id.is_set() {
            row.update(&txn).await?;
        } else {
            row.insert(&txn).await?;
        }
        Ok(txn.commit().await?)
    }

    async fn delete_run_target(&self, repo: String, id: String) -> Result<(), StoreError> {
        run_target::Entity::delete_many()
            .filter(run_target::Column::Repo.eq(repo))
            .filter(run_target::Column::TargetId.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    async fn record_run_approval(
        &self,
        repo: String,
        approved: Hash,
        baseline: SnapshotId,
        at: i64,
    ) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        run_approval::Entity::delete_many()
            .filter(run_approval::Column::Repo.eq(repo.clone()))
            .exec(&txn)
            .await?;
        run_approval::ActiveModel {
            repo: Set(repo),
            hash: Set(approved.0.to_vec()),
            baseline: Set(baseline.0.0.to_vec()),
            at: Set(at),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        Ok(txn.commit().await?)
    }

    async fn last_run_approval(&self, repo: String) -> Result<Option<(Hash, SnapshotId)>, StoreError> {
        let row = run_approval::Entity::find()
            .filter(run_approval::Column::Repo.eq(repo))
            .one(&self.db)
            .await?;
        Ok(row
            .map(|r| Ok::<_, DbErr>((hash(&r.hash)?, SnapshotId(hash(&r.baseline)?))))
            .transpose()?)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use vet_app::ports::Store;
    use vet_core::content::{BlobId, EntryKind, FileEntry, RepoPath, Snapshot};
    use vet_core::merkle::snapshot_id;

    use super::*;
    use crate::store::StoreConfig;

    async fn open(dir: &tempfile::TempDir) -> SqliteStore {
        let config = StoreConfig {
            gc_grace: Duration::ZERO,
            ..Default::default()
        };
        SqliteStore::open(dir.path(), config).await.unwrap()
    }

    fn target(id: &str, program: &str) -> RunTarget {
        RunTarget {
            id: id.into(),
            source: None,
            label: format!("label {id}"),
            program: program.into(),
            args: vec!["run".into(), "a b".into()],
            cwd: "ui".into(),
            env: vec![("PORT".into(), "1".into())],
        }
    }

    #[tokio::test]
    async fn targets_are_per_repo_and_replaced_by_id() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = open(&dir).await;
            store.put_run_target("/a".into(), target("x", "npm")).await.unwrap();
            store.put_run_target("/a".into(), target("y", "npm")).await.unwrap();
            store.put_run_target("/b".into(), target("x", "yarn")).await.unwrap();
            store.put_run_target("/a".into(), target("x", "pnpm")).await.unwrap();
        }
        let store = open(&dir).await;
        assert_eq!(
            store.run_targets("/a".into()).await.unwrap(),
            vec![target("x", "pnpm"), target("y", "npm")]
        );
        store.delete_run_target("/a".into(), "x".into()).await.unwrap();
        assert_eq!(store.run_targets("/a".into()).await.unwrap(), vec![target("y", "npm")]);
        assert_eq!(store.run_targets("/b".into()).await.unwrap(), vec![target("x", "yarn")]);
    }

    #[tokio::test]
    async fn the_last_approval_per_repo_wins_and_its_baseline_survives_gc() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir).await;
        let snapshot = |n: u8| {
            let entry = FileEntry {
                blob: BlobId::of(&[n]),
                mode: 0o100644,
                kind: EntryKind::File,
                unfiltered: false,
            };
            let entries = BTreeMap::from([(RepoPath(b"Cargo.toml".to_vec()), entry)]);
            Snapshot {
                id: snapshot_id(&entries),
                entries,
            }
        };
        let (old, new) = (snapshot(1), snapshot(2));
        for s in [&old, &new] {
            store.put_snapshot(s.clone(), Default::default(), 0).await.unwrap();
        }
        store
            .record_run_approval("/r".into(), old.id.0, old.id, 1)
            .await
            .unwrap();
        store
            .record_run_approval("/r".into(), new.id.0, new.id, 2)
            .await
            .unwrap();
        assert_eq!(
            store.last_run_approval("/r".into()).await.unwrap(),
            Some((new.id.0, new.id))
        );
        assert_eq!(store.last_run_approval("/other".into()).await.unwrap(), None);

        store.gc(i64::MAX / 2).await.unwrap();
        assert!(store.snapshot(new.id).await.unwrap().is_some());
        assert!(store.snapshot(old.id).await.unwrap().is_none());
    }
}
