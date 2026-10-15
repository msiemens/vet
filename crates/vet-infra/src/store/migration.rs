//! Schema migrations; sea-orm-migration records the applied ones in `seaql_migrations`.
//!
//! The first migration, the baseline, is the schema as it was when migrations began, kept as
//! SQL: it must never follow the entities. Every later schema change is a migration of its own,
//! appended to [`Migrator::migrations`] and named `m<date>_<number>_<what>`, that changes the
//! tables of a database in place. Tests check that the migrations lead to the schema the
//! entities describe, so a change of an entity without a migration fails.
//!
//! A database that applied the migration of a release that created the schema whole and named
//! it after a hash (`schema_…`) cannot be upgraded and is recreated by `SqliteStore::open`.

use sea_orm::DatabaseConnection;
use sea_orm_migration::prelude::*;

pub(super) struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(Sql {
                name: "m20261014_000001_baseline",
                sql: include_str!("migration/baseline.sql"),
            }),
            Box::new(Sql {
                name: "m20261014_000002_settings",
                sql: include_str!("migration/settings.sql"),
            }),
            Box::new(Sql {
                name: "m20261014_000003_version_availability",
                sql: include_str!("migration/version_availability.sql"),
            }),
            Box::new(Sql {
                name: "m20261014_000004_pull_request_details",
                sql: include_str!("migration/pull_request_details.sql"),
            }),
            Box::new(Sql {
                name: "m20261015_000005_typed_verdicts",
                sql: include_str!("migration/typed_verdicts.sql"),
            }),
        ]
    }
}

/// A migration that is SQL statements.
struct Sql {
    name: &'static str,
    sql: &'static str,
}

impl MigrationName for Sql {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Sql {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(self.sql).await?;
        Ok(())
    }
}

/// What a database's applied migrations say about the version of Vet that wrote it.
#[derive(PartialEq, Eq, Debug)]
pub(super) enum Origin {
    /// Every migration it applied is known; it is upgraded by the rest.
    Known,
    /// It applied the hash-named migration of a release that did not migrate.
    Unmigrated,
    /// It applied migrations of a newer Vet.
    Newer,
}

pub(super) async fn origin(db: &DatabaseConnection) -> Result<Origin, DbErr> {
    let known: Vec<String> = Migrator::migrations().iter().map(|m| m.name().to_owned()).collect();
    let unknown: Vec<String> = Migrator::get_migration_models(db)
        .await?
        .into_iter()
        .map(|m| m.version)
        .filter(|version| !known.contains(version))
        .collect();
    Ok(if unknown.is_empty() {
        Origin::Known
    } else if unknown.iter().all(|version| version.starts_with("schema_")) {
        Origin::Unmigrated
    } else {
        Origin::Newer
    })
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, DbBackend, Schema, Statement};
    use vet_app::platform::{Discussion, Verdict};

    use super::super::ReviewWriter;
    use super::super::entity::*;
    use super::*;

    const REPO: &str = "/kept";

    async fn database() -> (tempfile::TempDir, DatabaseConnection) {
        let dir = tempfile::tempdir().unwrap();
        let writer = ReviewWriter::connect(dir.path()).await.unwrap();
        (dir, writer.db)
    }

    /// The SQL that creates every table and index the entities describe.
    fn entity_statements() -> Vec<String> {
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
            version_file::Entity,
            review::Entity,
            project_filter::Entity,
            project_filter_item::Entity,
            project_trust::Entity,
            anchor::Entity,
            span::Entity,
            ack::Entity,
            thread::Entity,
            message::Entity,
            server_path::Entity,
            recent_project::Entity,
            backend_setting::Entity,
            preference::Entity,
            pull_request::Entity
        );
        out
    }

    /// Every column, index and foreign key of the database, one line each. The order of a
    /// table's columns is left out: a migration that adds a column puts it last.
    async fn schema(db: &DatabaseConnection) -> Vec<String> {
        const TABLES: &str = "m.type = 'table' AND m.name NOT LIKE 'sqlite_%' AND m.name <> 'seaql_migrations'";
        let queries = [
            format!(
                "SELECT m.name || '.' || c.name || ' ' || c.type || ' notnull=' || c.\"notnull\" \
                 || ' default=' || ifnull(c.dflt_value, '-') || ' pk=' || c.pk \
                 FROM sqlite_master m, pragma_table_info(m.name) c WHERE {TABLES}"
            ),
            format!(
                "SELECT m.name || ' index ' || i.name || ' unique=' || i.\"unique\" || ' on ' || \
                 (SELECT group_concat(name, ',') FROM (SELECT name FROM pragma_index_info(i.name) ORDER BY seqno)) \
                 FROM sqlite_master m, pragma_index_list(m.name) i WHERE {TABLES}"
            ),
            format!(
                "SELECT m.name || '.' || f.\"from\" || ' references ' || f.\"table\" || '.' || ifnull(f.\"to\", '') \
                 || ' on delete ' || f.on_delete || ' on update ' || f.on_update \
                 FROM sqlite_master m, pragma_foreign_key_list(m.name) f WHERE {TABLES}"
            ),
        ];
        let mut lines = Vec::new();
        for query in queries {
            for row in db
                .query_all_raw(Statement::from_string(DbBackend::Sqlite, query))
                .await
                .unwrap()
            {
                lines.push(row.try_get_by_index::<String>(0).unwrap());
            }
        }
        lines.sort();
        lines
    }

    /// What `actual` has that `expected` lacks (`+`) and the other way round (`-`).
    fn differences(expected: &[String], actual: &[String]) -> Vec<String> {
        let only = |from: &[String], other: &[String], mark: &str| -> Vec<String> {
            from.iter()
                .filter(|line| !other.contains(line))
                .map(|line| format!("{mark} {line}"))
                .collect()
        };
        [only(actual, expected, "+"), only(expected, actual, "-")].concat()
    }

    #[tokio::test]
    async fn the_migrations_lead_to_the_schema_of_the_entities() {
        let (_migrated_dir, migrated) = database().await;
        Migrator::up(&migrated, None).await.unwrap();

        let (_created_dir, created) = database().await;
        for sql in entity_statements() {
            created.execute_unprepared(&sql).await.unwrap();
        }

        assert_eq!(
            differences(&schema(&created).await, &schema(&migrated).await),
            Vec::<String>::new(),
            "an entity changed without a migration that changes the tables the same way"
        );
    }

    #[tokio::test]
    async fn a_database_migrated_from_any_earlier_state_equals_a_new_one_and_keeps_its_rows() {
        let (_new_dir, new) = database().await;
        Migrator::up(&new, None).await.unwrap();
        let expected = schema(&new).await;

        for applied in 1..Migrator::migrations().len() as u32 {
            let (_dir, old) = database().await;
            Migrator::up(&old, Some(applied)).await.unwrap();
            old.execute_unprepared(
                "INSERT INTO recent_project (path, opened_at) VALUES ('/kept', 7); \
                 INSERT INTO changeset (repo, kind, key) VALUES ('/kept', 0, '')",
            )
            .await
            .unwrap();

            Migrator::up(&old, None).await.unwrap();

            assert_eq!(
                differences(&expected, &schema(&old).await),
                Vec::<String>::new(),
                "from the state after {applied} migrations"
            );
            let rows = old
                .query_all_raw(Statement::from_string(
                    DbBackend::Sqlite,
                    "SELECT path FROM recent_project UNION ALL SELECT repo FROM changeset",
                ))
                .await
                .unwrap();
            assert_eq!(rows.len(), 2, "from the state after {applied} migrations");
        }
    }

    #[tokio::test]
    async fn a_database_is_known_unmigrated_or_newer_by_its_applied_migrations() {
        let (_dir, db) = database().await;
        assert_eq!(origin(&db).await.unwrap(), Origin::Known);
        Migrator::up(&db, None).await.unwrap();
        assert_eq!(origin(&db).await.unwrap(), Origin::Known);

        let record =
            |version: &str| format!("INSERT INTO seaql_migrations (version, applied_at) VALUES ('{version}', 0)");
        db.execute_unprepared(&record("schema_0123456789abcdef")).await.unwrap();
        assert_eq!(origin(&db).await.unwrap(), Origin::Unmigrated);

        db.execute_unprepared(&record("m29990101_000001_from_the_future"))
            .await
            .unwrap();
        assert_eq!(origin(&db).await.unwrap(), Origin::Newer);
    }

    #[tokio::test]
    async fn a_vote_and_a_copys_verdict_spelled_earlier_become_how_vet_spells_them_an_unknown_stays() {
        let (_dir, db) = database().await;
        Migrator::up(&db, Some((Migrator::migrations().len() - 1) as u32))
            .await
            .unwrap();
        // What a staged vote was spelled as in the versions before the verdicts
        // are typed, and what each is spelled as now.
        let spelled: &[(&str, &str)] = &[
            ("approved", "approve"),
            ("approved-with-suggestions", "approve-with-suggestions"),
            ("rejected", "reject"),
            ("APPROVE", "approve"),
            ("REQUEST_CHANGES", "request-changes"),
            // The one that is spelled the same both ways.
            ("wait-for-author", "wait-for-author"),
            // A spelling the migrations do not know stays spelled.
            ("hidden rule", "hidden rule"),
        ];
        let rows: Vec<String> = spelled
            .iter()
            .enumerate()
            .map(|(i, (spelled, _))| format!("('{REPO}', 0, 'k{i}', '{spelled}')"))
            .collect();
        db.execute_unprepared(&format!(
            "INSERT INTO changeset (repo, kind, key, vote) VALUES {}",
            rows.join(", ")
        ))
        .await
        .unwrap();

        let detail = |verdict: &str| {
            serde_json::json!({
                "title": "", "url": "", "description": "", "draft": false,
                "source_branch": "", "target_branch": "", "state": "Open",
                "author": "", "author_name": "", "author_avatar": null,
                "reviewers": [], "viewer_verdict": verdict, "labels": [],
                "ci": null, "decision": "Undecided", "blocked": false,
                "conversation": [], "threads": [], "complete": true,
            })
            .to_string()
        };
        for (i, verdict) in ["approved", "rejected", "hidden rule", "approve"].iter().enumerate() {
            db.execute_unprepared(&format!(
                "INSERT INTO pull_request (host, path, number, head, updated_at, synced_at, detail) \
                 VALUES ('dev.azure.com', '{REPO}', {i}, '', 1, 1, '{}')",
                detail(verdict).replace('\'', "''")
            ))
            .await
            .unwrap();
        }

        Migrator::up(&db, None).await.unwrap();

        let votes = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("SELECT key, vote FROM changeset WHERE repo = '{REPO}' ORDER BY key"),
            ))
            .await
            .unwrap();
        let got: Vec<String> = votes.iter().map(|r| r.try_get("", "vote").unwrap()).collect();
        let kept: Vec<String> = spelled.iter().map(|(_, vote)| (*vote).to_owned()).collect();
        assert_eq!(got, kept);

        let copies = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("SELECT detail FROM pull_request WHERE path = '{REPO}' ORDER BY number"),
            ))
            .await
            .unwrap();
        let got: Vec<Option<Verdict>> = copies
            .iter()
            .map(|r| {
                serde_json::from_str::<Discussion>(&r.try_get::<String>("", "detail").unwrap())
                    .unwrap()
                    .viewer_verdict
            })
            .collect();
        let kept: Vec<Option<Verdict>> = [
            Verdict::Approve,
            Verdict::Reject,
            Verdict::Other("hidden rule".into()),
            // One spelled the way Vet spells it already is left as it is.
            Verdict::Approve,
        ]
        .into_iter()
        .map(Some)
        .collect();
        assert_eq!(got, kept);
    }
}
