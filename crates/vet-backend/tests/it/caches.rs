//! "Clear semantic caches" and the cache's budget reach the file the answers are kept in.

use std::sync::Arc;

use crate::common::{Client, config, handler};
use vet_backend::{Orchestrator, open_store};
use vet_core::content::{BlobId, Hash};
use vet_infra::semantic_cache::SqliteSemanticCache;
use vet_protocol::{BackendSettings, Event, Request, Response, ServerMessage};
use vet_semantic::ports::{BackendId, ServerIdentity};
use vet_semantic::reuse::{Key, SemanticCache, Value};

fn key(text: &str) -> Key {
    Key::Text {
        server: ServerIdentity {
            backend: BackendId("rust".into()),
            fingerprint: Hash([1; 32]),
            label: "rust-analyzer".into(),
        },
        extension: b"rs".to_vec(),
        blob: BlobId::of(text.as_bytes()),
    }
}

/// The bytes the semantic cache's file takes, as the app's Storage figures say.
async fn usage(client: &Client) -> u64 {
    match client.call(Request::ProjectsStorage).await {
        Ok(Response::ProjectsStorage { app, .. }) => app.semantic_cache,
        other => panic!("{other:?}"),
    }
}

fn heard(client: &mut Client) -> Vec<Event> {
    std::iter::from_fn(|| client.inbox.try_recv().ok())
        .filter_map(|m| match m {
            ServerMessage::Event { event } => Some(event),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn clearing_the_semantic_caches_empties_the_file_all_launches_share() {
    let (handler, data) = handler().await;
    let client = Client::new(&handler);
    let file = SqliteSemanticCache::open(data.path(), 256 << 20).await.unwrap();
    file.put(&key("fn main() {}"), &Value::Symbols(vec![]));
    file.flush().await;
    assert!(file.get(&key("fn main() {}")).await.is_some());

    let answer = client.call(Request::ClearSemanticCaches).await;
    assert!(matches!(answer, Ok(Response::Done)), "{answer:?}");

    assert!(file.get(&key("fn main() {}")).await.is_none());
}

#[tokio::test]
async fn clearing_the_semantic_caches_tells_the_other_windows_the_figures_changed() {
    let (handler, _data) = handler().await;
    let clearing = Client::new(&handler);
    let mut watching = Client::new(&handler);
    watching.call(Request::Subscribe).await.unwrap();

    clearing.call(Request::ClearSemanticCaches).await.unwrap();

    assert_eq!(heard(&mut watching), [Event::StorageChanged]);
}

#[tokio::test]
async fn a_smaller_budget_shrinks_the_file_at_once_and_is_kept_for_the_next_launch() {
    let (handler, data) = handler().await;
    let client = Client::new(&handler);
    let file = SqliteSemanticCache::open(data.path(), 256 << 20).await.unwrap();
    let hover = Value::Hover(Some("x".repeat(100_000)));
    for n in 0..40 {
        file.put(&key(&format!("fn f{n}() {{}}")), &hover);
    }
    file.flush().await;
    let before = usage(&client).await;
    assert!(before > 4_000_000, "{before}");

    let settings = BackendSettings {
        semantic_cache_mb: 1,
        finished_review_days: 7,
    };
    let answer = client.call(Request::SetSettings { settings }).await;
    assert!(
        matches!(answer, Ok(Response::Settings { settings: s }) if s == settings),
        "{answer:?}"
    );
    let after = usage(&client).await;
    assert!(after < 2_000_000, "{before} → {after}");

    drop(client);
    drop(handler);
    let again = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config()));
    let answer = Client::new(&again).call(Request::Settings).await;
    assert!(
        matches!(answer, Ok(Response::Settings { settings: s }) if s == settings),
        "{answer:?}"
    );
}
