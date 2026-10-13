use std::collections::HashSet;
use std::sync::atomic::{AtomicI64, Ordering};

use tempfile::TempDir;
use vet_core::content::{BlobId, Hash, RepoPath, SnapshotId};
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{
    At, BackendId, Diagnostic, Location, Pos, ServerIdentity, Severity, Span, Sym, SymbolInfo, SymbolKind,
};
use vet_semantic::reuse::{Basis, ProjectQuery, Restore};

use super::codec::address;
use super::*;

fn server(fingerprint: u8) -> ServerIdentity {
    ServerIdentity {
        backend: BackendId("rust".into()),
        fingerprint: Hash([fingerprint; 32]),
        label: "rust-analyzer 1".into(),
    }
}

fn text_key(blob: u8) -> Key {
    Key::Text {
        server: server(1),
        extension: b"rs".to_vec(),
        blob: BlobId(Hash([blob; 32])),
    }
}

/// A key whose answer is a hover: one per `n`.
fn hover_key(n: u8) -> Key {
    project_key(ProjectQuery::Hover(at("a.rs", u32::from(n), 0)))
}

fn basis() -> Basis {
    Basis {
        snapshot: SnapshotId(Hash([2; 32])),
        policy: AnalysisPolicy { allow_executing: true },
        restore: Restore::Recorded(Hash([3; 32])),
    }
}

fn project_key(query: ProjectQuery) -> Key {
    Key::Project {
        server: server(1),
        basis: basis(),
        query,
    }
}

fn at(file: &str, line: u32, col: u32) -> At {
    At {
        file: file.into(),
        pos: Pos { line, col },
    }
}

fn span(line: u32) -> Span {
    Span {
        start: Pos { line, col: 1 },
        end: Pos { line: line + 2, col: 0 },
    }
}

fn hover(text: &str) -> Value {
    Value::Hover(Some(text.to_owned()))
}

/// A cache whose clock ticks once per batch written.
async fn ticking(dir: &TempDir, budget: u64) -> SqliteSemanticCache {
    let clock = AtomicI64::new(0);
    SqliteSemanticCache::start(
        dir.path(),
        budget,
        Arc::new(move || clock.fetch_add(1, Ordering::SeqCst)),
    )
    .await
    .unwrap()
}

async fn open(dir: &TempDir) -> SqliteSemanticCache {
    SqliteSemanticCache::open(dir.path(), 256 << 20).await.unwrap()
}

async fn count(cache: &SqliteSemanticCache) -> i64 {
    sqlx::query("SELECT COUNT(*) FROM answer")
        .fetch_one(&cache.pool)
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
async fn every_kind_of_answer_is_read_back_exactly_by_a_later_launch() {
    let dir = tempfile::tempdir().unwrap();
    let symbols = Value::Symbols(vec![
        SymbolInfo {
            sym: Sym {
                file: RepoPath(vec![b'd', 0xff, b'.', b'r', b's']),
                name: vec!["Outer".into(), "inner ü".into()],
            },
            span: span(4),
            at: Pos { line: 4, col: 8 },
            detail: Some("fn inner()".into()),
            kind: SymbolKind::Function,
            modules: 1,
        },
        SymbolInfo {
            sym: Sym {
                file: "b.rs".into(),
                name: vec![],
            },
            span: span(0),
            at: Pos { line: 0, col: 0 },
            detail: None,
            kind: SymbolKind::Other,
            modules: 0,
        },
    ]);
    let locations = Value::Locations(vec![
        Location {
            file: "a.rs".into(),
            span: span(1),
            external: false,
        },
        Location {
            file: "/dep/lib.rs".into(),
            span: span(9),
            external: true,
        },
    ]);
    let diagnostics = Value::Diagnostics(
        [Severity::Error, Severity::Warning, Severity::Info, Severity::Hint]
            .into_iter()
            .enumerate()
            .map(|(i, severity)| Diagnostic {
                span: span(i as u32),
                severity,
                message: format!("message {i}"),
                source: (i % 2 == 0).then(|| "rustc".to_owned()),
                code: (i % 3 == 0).then(|| "E0308".to_owned()),
            })
            .collect(),
    );
    let kept = [
        (text_key(1), symbols.clone()),
        (
            project_key(ProjectQuery::DocumentSymbols("a.rs".into())),
            symbols.clone(),
        ),
        (project_key(ProjectQuery::Incoming(at("a.rs", 1, 2))), symbols),
        (
            project_key(ProjectQuery::Definition(at("a.rs", 1, 2))),
            locations.clone(),
        ),
        (project_key(ProjectQuery::References(at("a.rs", 1, 2))), locations),
        (project_key(ProjectQuery::Hover(at("a.rs", 1, 2))), hover("fn a()")),
        (project_key(ProjectQuery::Hover(at("a.rs", 3, 2))), Value::Hover(None)),
        (project_key(ProjectQuery::Diagnostics("a.rs".into())), diagnostics),
    ];

    let first = open(&dir).await;
    for (key, value) in &kept {
        first.put(key, value);
    }
    first.flush().await;
    drop(first);

    let later = open(&dir).await;
    for (key, value) in &kept {
        let found = later.get(key).await;
        assert_eq!(format!("{found:?}"), format!("{:?}", Some(value)), "{key:?}");
    }
}

#[tokio::test]
async fn every_input_of_a_key_is_part_of_where_its_answer_is_kept() {
    let project = |change: fn(&mut ServerIdentity, &mut Basis, &mut ProjectQuery)| {
        let (mut server, mut basis, mut query) = (server(1), basis(), ProjectQuery::Hover(at("a.rs", 1, 2)));
        change(&mut server, &mut basis, &mut query);
        Key::Project { server, basis, query }
    };
    let keys = [
        project(|_, _, _| {}),
        project(|s, _, _| s.fingerprint = Hash([9; 32])),
        project(|s, _, _| s.backend = BackendId("python".into())),
        project(|_, b, _| b.snapshot = SnapshotId(Hash([9; 32]))),
        project(|_, b, _| b.policy = AnalysisPolicy { allow_executing: false }),
        project(|_, b, _| b.restore = Restore::Recorded(Hash([9; 32]))),
        project(|_, _, q| *q = ProjectQuery::Hover(at("a.rs", 2, 2))),
        project(|_, _, q| *q = ProjectQuery::Hover(at("a.rs", 1, 3))),
        project(|_, _, q| *q = ProjectQuery::Hover(at("b.rs", 1, 2))),
        project(|_, _, q| *q = ProjectQuery::Definition(at("a.rs", 1, 2))),
        project(|_, _, q| *q = ProjectQuery::References(at("a.rs", 1, 2))),
        project(|_, _, q| *q = ProjectQuery::Incoming(at("a.rs", 1, 2))),
        project(|_, _, q| *q = ProjectQuery::Outgoing(at("a.rs", 1, 2))),
        project(|_, _, q| *q = ProjectQuery::DocumentSymbols("a.rs".into())),
        project(|_, _, q| *q = ProjectQuery::Diagnostics("a.rs".into())),
        text_key(1),
        text_key(2),
        Key::Text {
            server: server(1),
            extension: b"py".to_vec(),
            blob: BlobId(Hash([1; 32])),
        },
        Key::Text {
            server: server(2),
            extension: b"rs".to_vec(),
            blob: BlobId(Hash([1; 32])),
        },
    ];
    let addresses: HashSet<_> = keys.iter().map(|k| address(k).unwrap().0).collect();
    assert_eq!(addresses.len(), keys.len());

    // Said by the fingerprint already: how the server is called does not matter.
    let mut renamed = server(1);
    renamed.label = "rust-analyzer 2".into();
    let same = Key::Text {
        server: renamed,
        extension: b"rs".to_vec(),
        blob: BlobId(Hash([1; 32])),
    };
    assert_eq!(address(&same), address(&text_key(1)));
}

#[tokio::test]
async fn answers_that_are_not_to_persist_are_not_stored() {
    let dir = tempfile::tempdir().unwrap();
    let cache = open(&dir).await;
    let unrecorded = Key::Project {
        server: server(1),
        basis: Basis {
            restore: Restore::Instance(0),
            ..basis()
        },
        query: ProjectQuery::Hover(at("a.rs", 1, 2)),
    };
    for key in [unrecorded, project_key(ProjectQuery::Workspace("main".into()))] {
        cache.put(&key, &hover("x"));
    }
    cache.flush().await;
    assert_eq!(count(&cache).await, 0);
}

#[tokio::test]
async fn a_value_that_cannot_be_read_is_a_miss_and_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let cache = open(&dir).await;
    let (key, other) = (text_key(1), text_key(2));
    cache.put(&key, &Value::Symbols(vec![]));
    cache.put(&other, &Value::Symbols(vec![]));
    cache.flush().await;

    for garbage in [
        &b"\x07\x00\x00\x00 not a symbol"[..],
        &b""[..],
        &b"\x00\x00\x00\x00trailing"[..],
    ] {
        cache.put(&key, &Value::Symbols(vec![]));
        cache.flush().await;
        sqlx::query("UPDATE answer SET value = ? WHERE key = ?")
            .bind(garbage)
            .bind(&address(&key).unwrap().0[..])
            .execute(&cache.pool)
            .await
            .unwrap();
        assert!(cache.get(&key).await.is_none(), "{garbage:?}");
        assert_eq!(count(&cache).await, 1, "only the damaged row goes");
    }
    assert!(cache.get(&other).await.is_some());
}

#[tokio::test]
async fn a_file_of_another_format_is_recreated() {
    for foreign in ["another version", "not a database"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        if foreign == "another version" {
            let old = SqlitePoolOptions::new()
                .connect_with(SqliteConnectOptions::new().filename(&path).create_if_missing(true))
                .await
                .unwrap();
            sqlx::query("CREATE TABLE answer (something INTEGER)")
                .execute(&old)
                .await
                .unwrap();
            sqlx::query("PRAGMA user_version = 99").execute(&old).await.unwrap();
            old.close().await;
        } else {
            std::fs::write(&path, "this is not what SQLite writes".repeat(100)).unwrap();
        }

        let cache = open(&dir).await;
        cache.put(&hover_key(1), &hover("kept"));
        cache.flush().await;
        assert!(cache.get(&hover_key(1)).await.is_some(), "{foreign}");
    }
}

#[tokio::test]
async fn over_its_budget_the_rows_used_least_recently_go_first() {
    let dir = tempfile::tempdir().unwrap();
    // A row is 105 bytes; the tenth goes over, and eviction stops below 800.
    let cache = ticking(&dir, 1000).await;
    let text = "x".repeat(100);
    for blob in 0..9 {
        cache.put(&hover_key(blob), &hover(&text));
        cache.flush().await;
    }
    assert_eq!(count(&cache).await, 9);

    // The oldest is used again, so the next oldest ones go.
    assert!(cache.get(&hover_key(0)).await.is_some());
    cache.put(&hover_key(9), &hover(&text));
    cache.flush().await;

    let mut present = Vec::new();
    for blob in 0..10 {
        if cache.get(&hover_key(blob)).await.is_some() {
            present.push(blob);
        }
    }
    assert_eq!(present, [0, 4, 5, 6, 7, 8, 9]);
}

#[tokio::test]
async fn a_smaller_budget_removes_the_least_recently_used_at_once_and_the_file_shrinks() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ticking(&dir, 1 << 30).await;
    let text = "x".repeat(10_000);
    for blob in 0..100 {
        cache.put(&hover_key(blob), &hover(&text));
        cache.flush().await;
    }
    let before = size_on_disk(dir.path());
    assert!(before > 1_000_000, "{before}");

    // About 10 kB a row: four fifths of 100 kB keep the 7 used last.
    cache.set_budget(100_000).await;
    assert_eq!(count(&cache).await, 7);
    assert!(cache.get(&hover_key(99)).await.is_some());
    assert!(cache.get(&hover_key(92)).await.is_none());
    let after = size_on_disk(dir.path());
    assert!(after < before / 4, "{before} → {after}");

    // The new budget holds for what comes after, too.
    for blob in 100..120 {
        cache.put(&hover_key(blob), &hover(&text));
    }
    cache.flush().await;
    assert!(count(&cache).await <= 10);
}

#[tokio::test]
async fn two_launches_on_one_file_lose_none_of_each_others_writes() {
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = (open(&dir).await, open(&dir).await);
    async fn write(cache: &SqliteSemanticCache, blobs: std::ops::Range<u8>) {
        for blob in blobs {
            cache.put(&hover_key(blob), &hover(&format!("answer {blob}")));
        }
        cache.flush().await;
    }
    tokio::join!(write(&first, 0..100), write(&second, 100..200));

    let third = open(&dir).await;
    for blob in 0..200 {
        let found = third.get(&hover_key(blob)).await;
        assert_eq!(
            format!("{found:?}"),
            format!("{:?}", Some(hover(&format!("answer {blob}")))),
            "{blob}"
        );
    }
}

#[tokio::test]
async fn clearing_removes_everything_and_the_cache_keeps_working() {
    let dir = tempfile::tempdir().unwrap();
    let cache = open(&dir).await;
    cache.put(&hover_key(1), &hover("one"));
    cache.put(&hover_key(2), &hover("two"));
    // Not flushed first: the writes queued before the clear are cleared too.
    let freed = cache.clear().await;
    assert!(freed > 0, "{freed}");
    assert!(cache.get(&hover_key(1)).await.is_none());
    assert_eq!(count(&cache).await, 0);

    cache.put(&hover_key(3), &hover("three"));
    cache.flush().await;
    assert!(cache.get(&hover_key(3)).await.is_some());
}
