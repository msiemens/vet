//! The recently opened repositories: the ten most recent, newest first, each once.

use tokio::sync::Mutex;

use crate::ports::{RecentProject, RecentStore, StoreError};

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

    pub async fn list(&self) -> Result<Vec<RecentProject>, StoreError> {
        self.store.recent_projects().await
    }

    /// Puts `path` first, opened at `now` (unix seconds). Returns the new list.
    pub async fn record(&self, path: String, now: i64) -> Result<Vec<RecentProject>, StoreError> {
        self.change(|list| {
            list.retain(|entry| entry.path != path);
            list.insert(0, RecentProject { path, opened_at: now });
            list.truncate(LIMIT);
        })
        .await
    }

    /// Returns the new list.
    pub async fn forget(&self, path: &str) -> Result<Vec<RecentProject>, StoreError> {
        self.change(|list| list.retain(|entry| entry.path != path)).await
    }

    async fn change(&self, edit: impl FnOnce(&mut Vec<RecentProject>)) -> Result<Vec<RecentProject>, StoreError> {
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
    struct Memory(std::sync::Mutex<Vec<RecentProject>>);

    impl RecentStore for Memory {
        async fn recent_projects(&self) -> Result<Vec<RecentProject>, StoreError> {
            Ok(self.0.lock().unwrap().clone())
        }

        async fn set_recent_projects(&self, projects: Vec<RecentProject>) -> Result<(), StoreError> {
            *self.0.lock().unwrap() = projects;
            Ok(())
        }
    }

    fn paths(list: &[RecentProject]) -> Vec<&str> {
        list.iter().map(|p| p.path.as_str()).collect()
    }

    #[tokio::test]
    async fn the_list_is_newest_first_without_duplicates_and_capped_at_ten() {
        let recent = RecentProjects::new(Memory::default());
        for i in 0..10 {
            recent.record(format!("/r{i}"), i).await.unwrap();
        }
        let list = recent.record("/r3".into(), 100).await.unwrap();
        assert_eq!(paths(&list)[..3], ["/r3", "/r9", "/r8"]);
        assert_eq!(list.iter().filter(|p| p.path == "/r3").count(), 1);
        assert_eq!(list.len(), 10);

        let list = recent.record("/new".into(), 101).await.unwrap();
        assert_eq!(list.len(), 10);
        assert_eq!(list[0].path, "/new");
        assert!(!paths(&list).contains(&"/r0"), "the oldest is dropped");
        assert_eq!(recent.list().await.unwrap(), list);
    }

    #[tokio::test]
    async fn reopening_a_project_moves_its_time_forward() {
        let recent = RecentProjects::new(Memory::default());
        recent.record("/a".into(), 10).await.unwrap();
        recent.record("/b".into(), 20).await.unwrap();
        let list = recent.record("/a".into(), 30).await.unwrap();
        assert_eq!(list[0].opened_at, 30);
        assert_eq!(list[1].opened_at, 20);
    }

    #[tokio::test]
    async fn forgetting_removes_only_that_entry() {
        let recent = RecentProjects::new(Memory::default());
        recent.record("/a".into(), 1).await.unwrap();
        recent.record("/b".into(), 2).await.unwrap();
        assert_eq!(paths(&recent.forget("/a").await.unwrap()), ["/b"]);
        assert_eq!(paths(&recent.forget("/missing").await.unwrap()), ["/b"]);
    }
}
