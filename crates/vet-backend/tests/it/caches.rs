//! "Clear semantic caches" and the cache's budget reach the file the answers are kept in.

use std::sync::Arc;

use crate::common::{Client, config, handler};
use vet_backend::{Orchestrator, open_store};
use vet_core::content::{BlobId, Hash};
use vet_infra::semantic_cache::SqliteSemanticCache;
use vet_protocol::{BackendSettings, Request, Response};
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

fn usage(response: Result<Response, String>) -> u64 {
    match response {
        Ok(Response::SemanticCacheUsage { bytes }) => bytes,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn clearing_the_semantic_caches_empties_the_file_all_launches_share() {
    let (handler, data) = handler().await;
    let client = Client::new(&handler);
    let file = SqliteSemanticCache::open(data.path(), 256 << 20).await.unwrap();
    file.put(&key("fn main() {}"), &Value::Symbols(vec![]));
    file.flush().await;
    assert!(file.get(&key("fn main() {}")).await.is_some());

    usage(client.call(Request::ClearSemanticCaches).await);

    assert!(file.get(&key("fn main() {}")).await.is_none());
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
    let before = usage(client.call(Request::SemanticCacheUsage).await);
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
    let after = usage(client.call(Request::SemanticCacheUsage).await);
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
