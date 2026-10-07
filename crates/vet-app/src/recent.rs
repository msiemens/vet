//! The recently opened repositories: the ten most recent, newest first, each once.

use tokio::sync::Mutex;

use crate::ports::{RecentStore, StoreError};

const LIMIT: usize = 10;

pub struct RecentProjects<S> {
    store: S,
    /// A change reads the list and writes it back; two at once would lose one.
    changing: Mutex<()>,
}

impl<S: RecentStore + Sync> RecentProjects<S> {
    pub fn new(store: S) -> Self {
        Self {
            store,
            changing: Mutex::new(()),
        }
    }

    pub async fn list(&self) -> Result<Vec<String>, StoreError> {
        self.store.recent_projects().await
    }

    /// Puts `path` first. Returns the new list.
    pub async fn record(&self, path: String) -> Result<Vec<String>, StoreError> {
        self.change(|list| {
            list.retain(|entry| *entry != path);
            list.insert(0, path);
            list.truncate(LIMIT);
        })
        .await
    }

    /// Returns the new list.
    pub async fn forget(&self, path: &str) -> Result<Vec<String>, StoreError> {
        self.change(|list| list.retain(|entry| entry != path)).await
    }

    async fn change(&self, edit: impl FnOnce(&mut Vec<String>)) -> Result<Vec<String>, StoreError> {
        let _changing = self.changing.lock().await;
        let mut list = self.store.recent_projects().await?;
        edit(&mut list);
        self.store.set_recent_projects(list.clone()).await?;
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Memory(std::sync::Mutex<Vec<String>>);

    impl RecentStore for Memory {
        async fn recent_projects(&self) -> Result<Vec<String>, StoreError> {
            Ok(self.0.lock().unwrap().clone())
        }

        async fn set_recent_projects(&self, paths: Vec<String>) -> Result<(), StoreError> {
            *self.0.lock().unwrap() = paths;
            Ok(())
        }
    }

    #[tokio::test]
    async fn the_list_is_newest_first_without_duplicates_and_capped_at_ten() {
        let recent = RecentProjects::new(Memory::default());
        for i in 0..10 {
            recent.record(format!("/r{i}")).await.unwrap();
        }
        let list = recent.record("/r3".into()).await.unwrap();
        assert_eq!(list[..3], ["/r3", "/r9", "/r8"]);
        assert_eq!(list.iter().filter(|p| *p == "/r3").count(), 1);
        assert_eq!(list.len(), 10);

        let list = recent.record("/new".into()).await.unwrap();
        assert_eq!(list.len(), 10);
        assert_eq!(list[0], "/new");
        assert!(!list.contains(&"/r0".to_string()), "the oldest is dropped");
        assert_eq!(recent.list().await.unwrap(), list);
    }

    #[tokio::test]
    async fn forgetting_removes_only_that_entry() {
        let recent = RecentProjects::new(Memory::default());
        recent.record("/a".into()).await.unwrap();
        recent.record("/b".into()).await.unwrap();
        assert_eq!(recent.forget("/a").await.unwrap(), ["/b"]);
        assert_eq!(recent.forget("/missing").await.unwrap(), ["/b"]);
    }
}
