//! The kept source trees through the backend: what a start does to the data directory.

use crate::common::handler;

#[tokio::test]
async fn starting_removes_the_trees_of_the_older_layout() {
    let (handler, data) = handler().await;
    let legacy = data.path().join("materialized/0123-noexec/tree");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("file"), "x").unwrap();

    handler.collect_trees().await;

    // Deleted in the background: the start does not wait for it.
    let gone = async {
        while data.path().join("materialized").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), gone)
        .await
        .expect("the older layout is removed");
}
