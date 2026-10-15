//! The contract of the store ports against the real `SqliteStore`, one run beside the fake's
//! run in `vet-app/src/store_contract.rs` (T4). Every case opens its own database in a temporary
//! data directory, so that what one case stores is another case's fresh state, as the fake
//! observes it too.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use vet_app::store_contract::store_contract;
use vet_infra::store::SqliteStore;

#[tokio::test]
async fn the_real_store_satisfies_the_contract() {
    let data = tempfile::tempdir().unwrap();
    let root = data.path().to_owned();
    let opened = AtomicU32::new(0);
    store_contract(|| async {
        let dir = root.join(format!("case-{}", opened.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&dir).unwrap();
        fresh(&dir).await
    })
    .await;
}

/// A database that has run its migrations, and nothing else.
async fn fresh(dir: &Path) -> SqliteStore {
    SqliteStore::open(dir).await.unwrap()
}
