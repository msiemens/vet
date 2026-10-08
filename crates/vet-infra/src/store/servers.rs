//! `vet_app::ports::ServerSettings`: the custom language server path per language.

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait};
use vet_app::ports::{ServerSettings, StoreError};

use super::SqliteStore;
use super::entity::server_path;

impl ServerSettings for SqliteStore {
    async fn custom_server_path(&self, language: String) -> Result<Option<String>, StoreError> {
        let row = server_path::Entity::find()
            .filter(server_path::Column::Language.eq(language))
            .one(&self.reader.db)
            .await?;
        Ok(row.map(|r| r.path))
    }

    async fn set_custom_server_path(&self, language: String, path: Option<String>) -> Result<(), StoreError> {
        let txn = self.writer.db.begin().await?;
        server_path::Entity::delete_many()
            .filter(server_path::Column::Language.eq(language.clone()))
            .exec(&txn)
            .await?;
        if let Some(path) = path {
            server_path::ActiveModel {
                language: Set(language),
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
    async fn a_custom_path_is_per_language_replaced_removed_and_kept_across_opens() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = SqliteStore::open(dir.path()).await.unwrap();
            store
                .set_custom_server_path("rust".into(), Some("/a/ra".into()))
                .await
                .unwrap();
            store
                .set_custom_server_path("python".into(), Some("/a/pyright".into()))
                .await
                .unwrap();
            store
                .set_custom_server_path("rust".into(), Some("/b/ra".into()))
                .await
                .unwrap();
        }
        let store = SqliteStore::open(dir.path()).await.unwrap();
        assert_eq!(
            store.custom_server_path("rust".into()).await.unwrap().as_deref(),
            Some("/b/ra")
        );
        assert_eq!(
            store.custom_server_path("python".into()).await.unwrap().as_deref(),
            Some("/a/pyright")
        );
        assert_eq!(store.custom_server_path("csharp".into()).await.unwrap(), None);

        store.set_custom_server_path("rust".into(), None).await.unwrap();
        assert_eq!(store.custom_server_path("rust".into()).await.unwrap(), None);
        assert_eq!(
            store.custom_server_path("python".into()).await.unwrap().as_deref(),
            Some("/a/pyright")
        );
    }
}
