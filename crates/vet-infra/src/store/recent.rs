//! `vet_app::ports::RecentStore`: the recently opened repositories, oldest row first.

use sea_orm::{ActiveModelTrait, EntityTrait, QueryOrder, Set, TransactionTrait};
use vet_app::ports::{RecentProject, RecentStore, StoreError};

use super::SqliteStore;
use super::entity::recent_project;

impl RecentStore for SqliteStore {
    async fn recent_projects(&self) -> Result<Vec<RecentProject>, StoreError> {
        let rows = recent_project::Entity::find()
            .order_by_desc(recent_project::Column::Id)
            .all(&self.db)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| RecentProject {
                path: r.path,
                opened_at: r.opened_at,
            })
            .collect())
    }

    async fn set_recent_projects(&self, projects: Vec<RecentProject>) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        recent_project::Entity::delete_many().exec(&txn).await?;
        for project in projects.into_iter().rev() {
            recent_project::ActiveModel {
                path: Set(project.path),
                opened_at: Set(project.opened_at),
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

    fn project(path: &str, opened_at: i64) -> RecentProject {
        RecentProject {
            path: path.into(),
            opened_at,
        }
    }

    #[tokio::test]
    async fn the_list_keeps_its_order_and_times_across_opens() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = SqliteStore::open(dir.path()).await.unwrap();
            assert!(store.recent_projects().await.unwrap().is_empty());
            store
                .set_recent_projects(vec![project("/b", 3), project("/a", 2), project("/c", 1)])
                .await
                .unwrap();
            store
                .set_recent_projects(vec![project("/c", 9), project("/b", 3)])
                .await
                .unwrap();
        }
        let store = SqliteStore::open(dir.path()).await.unwrap();
        assert_eq!(
            store.recent_projects().await.unwrap(),
            [project("/c", 9), project("/b", 3)]
        );
    }
}
