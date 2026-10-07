//! Schema migrations; sea-orm-migration records applied ones in `seaql_migrations`.
//! The schema is created whole from the entities, and its one migration is
//! named after a hash of the statements that create it. A database that
//! applied another migration has another schema, from another version of Vet,
//! and is not upgraded: `SqliteStore::open` recreates it.

use std::collections::HashSet;
use std::sync::OnceLock;

use sea_orm::{DbBackend, Schema};
use sea_orm_migration::prelude::*;
use vet_core::content::Hash;

use super::entity::*;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Initial)]
    }
}

/// Whether `db` applied migrations that `Migrator` does not know.
pub async fn is_foreign(db: &impl ConnectionTrait) -> Result<bool, DbErr> {
    let known: HashSet<String> = Migrator::migrations().iter().map(|m| m.name().to_owned()).collect();
    let applied = Migrator::get_migration_models(db).await?;
    Ok(applied.iter().any(|m| !known.contains(&m.version)))
}

/// The SQL that creates every table and index.
fn statements() -> Vec<String> {
    let schema = Schema::new(DbBackend::Sqlite);
    let mut out = Vec::new();
    macro_rules! create {
        ($($entity:path),+) => {$(
            out.push(schema.create_table_from_entity($entity).to_string(SqliteQueryBuilder));
            for index in schema.create_index_from_entity($entity) {
                out.push(index.to_string(SqliteQueryBuilder));
            }
        )+};
    }
    create!(
        snapshot::Entity,
        snapshot_path::Entity,
        tree_node::Entity,
        tree_child::Entity,
        changeset::Entity,
        version::Entity,
        review::Entity,
        project_filter::Entity,
        project_filter_item::Entity,
        anchor::Entity,
        span::Entity,
        ack::Entity,
        thread::Entity,
        message::Entity,
        server_path::Entity,
        recent_project::Entity
    );
    out
}

struct Initial;

impl MigrationName for Initial {
    fn name(&self) -> &str {
        static NAME: OnceLock<String> = OnceLock::new();
        NAME.get_or_init(|| {
            let hex = Hash::of(statements().join(";\n").as_bytes()).to_hex();
            format!("schema_{}", &hex[..16])
        })
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Initial {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for sql in statements() {
            manager.get_connection().execute_unprepared(&sql).await?;
        }
        Ok(())
    }
}
