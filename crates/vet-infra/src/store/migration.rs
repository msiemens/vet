//! Schema migrations; sea-orm-migration records applied ones in `seaql_migrations`.

use sea_orm::{DbBackend, Schema};
use sea_orm_migration::prelude::*;

use super::entity::*;
use super::run::{run_approval, run_target};

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Initial), Box::new(RunState)]
    }
}

struct Initial;

impl MigrationName for Initial {
    fn name(&self) -> &str {
        "m0001_initial"
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
            filter_item::Entity,
            timeline::Entity,
            anchor::Entity,
            span::Entity,
            ack::Entity,
            thread::Entity,
            message::Entity
        );
        Ok(())
    }
}

struct RunState;

impl MigrationName for RunState {
    fn name(&self) -> &str {
        "m0002_run_state"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for RunState {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let schema = Schema::new(DbBackend::Sqlite);
        manager
            .create_table(schema.create_table_from_entity(run_target::Entity))
            .await?;
        for index in schema.create_index_from_entity(run_target::Entity) {
            manager.create_index(index).await?;
        }
        manager
            .create_table(schema.create_table_from_entity(run_approval::Entity))
            .await?;
        Ok(())
    }
}
