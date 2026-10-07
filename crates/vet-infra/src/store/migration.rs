//! Schema migrations; sea-orm-migration records applied ones in `seaql_migrations`.
//! The schema is created whole from the entities. A database that applied
//! migrations this build does not have comes from another version of Vet and
//! is not upgraded: `SqliteStore::open` recreates it.

use std::collections::HashSet;

use sea_orm::{DbBackend, Schema};
use sea_orm_migration::prelude::*;

use super::entity::*;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Initial)]
    }
}

/// Whether `db` applied migrations that `Migrator` does not know.
pub async fn is_foreign(db: &impl ConnectionTrait) -> Result<bool, DbErr> {
    let known: HashSet<String> = Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    let applied = Migrator::get_migration_models(db).await?;
    Ok(applied.iter().any(|m| !known.contains(&m.version)))
}

struct Initial;

impl MigrationName for Initial {
    fn name(&self) -> &str {
        "m0001_schema"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Initial {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let schema = Schema::new(DbBackend::Sqlite);
        macro_rules! create {
            ($($entity:path),+) => {$(
                manager.create_table(schema.create_table_from_entity($entity)).await?;
                for index in schema.create_index_from_entity($entity) {
                    manager.create_index(index).await?;
                }
            )+};
        }
        create!(
            pin::Entity,
            snapshot::Entity,
            snapshot_path::Entity,
            tree_node::Entity,
            tree_child::Entity,
            session::Entity,
            session_base::Entity,
            project_filter::Entity,
            project_filter_item::Entity,
            timeline::Entity,
            anchor::Entity,
            span::Entity,
            ack::Entity,
            thread::Entity,
            message::Entity,
            run_target::Entity,
            run_approval::Entity,
            server_path::Entity
        );
        Ok(())
    }
}
