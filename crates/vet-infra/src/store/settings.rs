//! `vet_app::ports::SettingsStore` and `PreferenceStore`: what the backend and the window
//! remember between launches, in the same file as the reviews.

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait};
use vet_app::ports::{PreferenceStore, Settings, SettingsStore, StoreError};

use super::SqliteStore;
use super::entity::{backend_setting, preference};

const SEMANTIC_CACHE_MB: &str = "semantic_cache_mb";
const FINISHED_REVIEW_DAYS: &str = "finished_review_days";

/// The scope of what belongs to the whole app.
const APP: &str = "";

impl SettingsStore for SqliteStore {
    async fn backend_settings(&self) -> Result<Settings, StoreError> {
        let mut settings = Settings::default();
        for row in backend_setting::Entity::find().all(&self.reader.db).await? {
            let Ok(value) = u32::try_from(row.value) else {
                continue;
            };
            match row.key.as_str() {
                SEMANTIC_CACHE_MB => settings.semantic_cache_mb = value,
                FINISHED_REVIEW_DAYS => settings.finished_review_days = value,
                _ => {}
            }
        }
        Ok(settings)
    }

    async fn set_backend_settings(&self, settings: Settings) -> Result<(), StoreError> {
        let txn = self.writer.db.begin().await?;
        backend_setting::Entity::delete_many().exec(&txn).await?;
        for (key, value) in [
            (SEMANTIC_CACHE_MB, settings.semantic_cache_mb),
            (FINISHED_REVIEW_DAYS, settings.finished_review_days),
        ] {
            backend_setting::ActiveModel {
                key: Set(key.to_owned()),
                value: Set(i64::from(value)),
            }
            .insert(&txn)
            .await?;
        }
        Ok(txn.commit().await?)
    }
}

impl PreferenceStore for SqliteStore {
    async fn preferences(&self, scope: Option<String>) -> Result<Vec<(String, String)>, StoreError> {
        let rows = preference::Entity::find()
            .filter(preference::Column::Scope.eq(scope.unwrap_or_else(|| APP.to_owned())))
            .order_by_asc(preference::Column::Key)
            .all(&self.reader.db)
            .await?;
        Ok(rows.into_iter().map(|r| (r.key, r.value)).collect())
    }

    async fn set_preference(
        &self,
        scope: Option<String>,
        key: String,
        value: Option<String>,
    ) -> Result<(), StoreError> {
        let scope = scope.unwrap_or_else(|| APP.to_owned());
        let txn = self.writer.db.begin().await?;
        preference::Entity::delete_many()
            .filter(preference::Column::Scope.eq(scope.clone()))
            .filter(preference::Column::Key.eq(key.clone()))
            .exec(&txn)
            .await?;
        if let Some(value) = value {
            preference::ActiveModel {
                scope: Set(scope),
                key: Set(key),
                value: Set(value),
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
    async fn settings_are_kept_across_opens_and_a_new_database_has_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let changed = Settings {
            semantic_cache_mb: 64,
            finished_review_days: 7,
        };
        {
            let store = SqliteStore::open(dir.path()).await.unwrap();
            assert_eq!(store.backend_settings().await.unwrap(), Settings::default());
            store.set_backend_settings(changed).await.unwrap();
            store.set_backend_settings(changed).await.unwrap();
        }
        let store = SqliteStore::open(dir.path()).await.unwrap();
        assert_eq!(store.backend_settings().await.unwrap(), changed);
    }

    async fn set(store: &SqliteStore, scope: Option<&str>, key: &str, value: Option<&str>) {
        store
            .set_preference(scope.map(Into::into), key.into(), value.map(Into::into))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn preferences_are_kept_per_scope_replaced_forgotten_and_kept_across_opens() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = SqliteStore::open(dir.path()).await.unwrap();
            assert!(store.preferences(None).await.unwrap().is_empty());
            set(&store, None, "theme", Some("\"dark\"")).await;
            set(&store, None, "theme", Some("\"light\"")).await;
            set(&store, None, "wrap", Some("true")).await;
            set(&store, Some("/a"), "theme", Some("\"project a\"")).await;
            set(&store, Some("/b"), "tabs", Some("[]")).await;
            set(&store, None, "wrap", None).await;
            set(&store, None, "never kept", None).await;
        }
        let store = SqliteStore::open(dir.path()).await.unwrap();
        let pairs = |list: Vec<(String, String)>| list.into_iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>();
        assert_eq!(pairs(store.preferences(None).await.unwrap()), ["theme=\"light\""]);
        assert_eq!(
            pairs(store.preferences(Some("/a".into())).await.unwrap()),
            ["theme=\"project a\""]
        );
        assert_eq!(pairs(store.preferences(Some("/b".into())).await.unwrap()), ["tabs=[]"]);
        assert!(store.preferences(Some("/c".into())).await.unwrap().is_empty());
    }
}
