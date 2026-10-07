//! `vet_app::ports::RecentStore`: the recently opened repositories, oldest row first.

use sea_orm::{ActiveModelTrait, EntityTrait, QueryOrder, Set, TransactionTrait};
use vet_app::ports::{RecentStore, StoreError};

use super::SqliteStore;
use super::entity::recent_project;

impl RecentStore for SqliteStore {
    async fn recent_projects(&self) -> Result<Vec<String>, StoreError> {
        let rows = recent_project::Entity::find()
            .order_by_desc(recent_project::Column::Id)
            .all(&self.db)
            .await?;
        Ok(rows.into_iter().map(|r| r.path).collect())
    }

    async fn set_recent_projects(&self, paths: Vec<String>) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        recent_project::Entity::delete_many().exec(&txn).await?;
        for path in paths.into_iter().rev() {
            recent_project::ActiveModel {
                path: Set(path),
                ..Default::default()
            }
            .insert(&txn)
            .await?;
        }
        Ok(txn.commit().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_list_keeps_its_order_across_opens() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = SqliteStore::open(dir.path()).await.unwrap();
            assert!(store.recent_projects().await.unwrap().is_empty());
            store
                .set_recent_projects(vec!["/b".into(), "/a".into(), "/c".into()])
                .await
                .unwrap();
            store.set_recent_projects(vec!["/c".into(), "/b".into()]).await.unwrap();
        }
        let store = SqliteStore::open(dir.path()).await.unwrap();
        assert_eq!(store.recent_projects().await.unwrap(), ["/c", "/b"]);
    }
}
