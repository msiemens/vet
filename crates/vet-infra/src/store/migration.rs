//! Schema migrations; sea-orm-migration records applied ones in `seaql_migrations`.

use sea_orm::{DbBackend, Schema};
use sea_orm_migration::prelude::*;

use super::entity::*;
use super::run::{run_approval, run_target};
use super::servers::server_path;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(Initial),
            Box::new(RunState),
            Box::new(SessionBase),
            Box::new(ServerPaths),
            Box::new(SessionRepo),
            Box::new(ReviewLifecycle),
            Box::new(CloseUnstartedSessions),
            Box::new(ProjectFilter),
        ]
    }
}

struct ProjectFilter;

impl MigrationName for ProjectFilter {
    fn name(&self) -> &str {
        "m0008_project_filter"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for ProjectFilter {
    /// Each repository takes the filter of its most recent session; the
    /// per-session filter storage is dropped. A fresh database has none of it:
    /// `Initial` creates the current shape.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let schema = Schema::new(DbBackend::Sqlite);
        manager
            .create_table(schema.create_table_from_entity(project_filter::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(project_filter_item::Entity))
            .await?;
        if !manager.has_table("filter_item").await? {
            return Ok(());
        }
        let db = manager.get_connection();
        const LATEST: &str = "(SELECT MAX(id) FROM session WHERE repo IS NOT NULL GROUP BY repo)";
        for sql in [
            format!(
                "INSERT INTO project_filter (repo, hide_binary) SELECT repo, hide_binary FROM session WHERE id IN {LATEST}"
            ),
            format!(
                "INSERT INTO project_filter_item (repo, kind, value) \
                 SELECT s.repo, f.kind, f.value FROM filter_item f JOIN session s ON s.id = f.session \
                 WHERE s.id IN {LATEST} ORDER BY f.id"
            ),
            "DROP TABLE filter_item".to_owned(),
            "ALTER TABLE session DROP COLUMN hide_binary".to_owned(),
        ] {
            db.execute_unprepared(&sql).await?;
        }
        Ok(())
    }
}

struct CloseUnstartedSessions;

impl MigrationName for CloseUnstartedSessions {
    fn name(&self) -> &str {
        "m0007_close_unstarted_sessions"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CloseUnstartedSessions {
    /// An open session without a mark, an acknowledgement or a thread never had
    /// a review and must not resume as one. Its session refs stay in git.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE session SET closed_at = CAST(strftime('%s', 'now') AS INTEGER), complete = 0 \
                 WHERE closed_at IS NULL \
                 AND NOT EXISTS (SELECT 1 FROM span WHERE span.session = session.id) \
                 AND NOT EXISTS (SELECT 1 FROM ack WHERE ack.session = session.id) \
                 AND NOT EXISTS (SELECT 1 FROM thread WHERE thread.session = session.id)",
            )
            .await?;
        Ok(())
    }
}

struct ReviewLifecycle;

impl MigrationName for ReviewLifecycle {
    fn name(&self) -> &str {
        "m0006_review_lifecycle"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for ReviewLifecycle {
    /// Sessions stored before keep their rows: a missing base commit is the
    /// pin's `HEAD`, a missing time is 0.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, column, def) in [
            (
                "session",
                "complete",
                ColumnDef::new(Alias::new("complete")).boolean().null().to_owned(),
            ),
            (
                "timeline",
                "at",
                ColumnDef::new(Alias::new("at")).big_integer().null().to_owned(),
            ),
            (
                "session_base",
                "commit",
                ColumnDef::new(Alias::new("commit")).string().null().to_owned(),
            ),
        ] {
            // A fresh database already has the columns, as in `SessionRepo`.
            if manager.has_column(table, column).await? {
                continue;
            }
            manager
                .alter_table(Table::alter().table(Alias::new(table)).add_column(def).to_owned())
                .await?;
        }
        Ok(())
    }
}

struct SessionRepo;

impl MigrationName for SessionRepo {
    fn name(&self) -> &str {
        "m0005_session_repo"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for SessionRepo {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // A fresh database already has the column: `Initial` creates the table
        // from the current entity.
        if manager.has_column("session", "repo").await? {
            return Ok(());
        }
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("session"))
                    .add_column(ColumnDef::new(Alias::new("repo")).string().null())
                    .to_owned(),
            )
            .await
    }
}

struct ServerPaths;

impl MigrationName for ServerPaths {
    fn name(&self) -> &str {
        "m0004_server_paths"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for ServerPaths {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let schema = Schema::new(DbBackend::Sqlite);
        manager
            .create_table(schema.create_table_from_entity(server_path::Entity))
            .await
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

struct SessionBase;

impl MigrationName for SessionBase {
    fn name(&self) -> &str {
        "m0003_session_base"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for SessionBase {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let schema = Schema::new(DbBackend::Sqlite);
        manager
            .create_table(schema.create_table_from_entity(session_base::Entity))
            .await
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
