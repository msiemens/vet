//! "Clear semantic caches" reaches the file the answers are kept in.

mod common;

use common::{Client, handler};
use vet_core::content::{BlobId, Hash};
use vet_infra::semantic_cache::SqliteSemanticCache;
use vet_protocol::{Request, Response};
use vet_semantic::ports::{BackendId, ServerIdentity};
use vet_semantic::reuse::{Key, SemanticCache, Value};

#[tokio::test]
async fn clearing_the_semantic_caches_empties_the_file_all_launches_share() {
    let (handler, data) = handler().await;
    let client = Client::new(&handler);
    let key = Key::Text {
        server: ServerIdentity {
            backend: BackendId("rust".into()),
            fingerprint: Hash([1; 32]),
            label: "rust-analyzer".into(),
        },
        extension: b"rs".to_vec(),
        blob: BlobId::of(b"fn main() {}"),
    };
    let file = SqliteSemanticCache::open(data.path()).await.unwrap();
    file.put(&key, &Value::Symbols(vec![]));
    file.flush().await;
    assert!(file.get(&key).await.is_some());

    assert!(matches!(
        client.call(Request::ClearSemanticCaches).await,
        Ok(Response::Done)
    ));

    assert!(file.get(&key).await.is_none());
}
