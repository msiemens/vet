//! `vet_app::ports::ServerSettings`: the custom language server path per language.

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait};
use vet_app::ports::{ServerSettings, StoreError};

use super::SqliteStore;

/// Unique per language.
pub mod server_path {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "server_path")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        #[sea_orm(unique)]
        pub language: String,
        pub path: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

impl ServerSettings for SqliteStore {
    async fn custom_server_path(&self, language: String) -> Result<Option<String>, StoreError> {
        let row = server_path::Entity::find()
            .filter(server_path::Column::Language.eq(language))
            .one(&self.db)
            .await?;
        Ok(row.map(|r| r.path))
    }

    async fn set_custom_server_path(&self, language: String, path: Option<String>) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
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
            let store = SqliteStore::open(dir.path(), Default::default()).await.unwrap();
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
        let store = SqliteStore::open(dir.path(), Default::default()).await.unwrap();
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
