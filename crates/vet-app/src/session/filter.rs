//! Changing the review filter of the changeset on view.

use vet_core::filter::ReviewFilter;

use super::hold::Held;
use super::{LiveSession, Result, SessionError, remap};
use crate::changeset::Source;
use crate::ports::{GitRepo, LiveStore, ReviewWrites};

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// Replaces the repository's review filter, stored whether or not a review
    /// is open, and bumps the revision; completion and the excluded counts
    /// follow with the next manifest. Returns the revision.
    pub async fn set_filter(&self, filter: ReviewFilter) -> Result<u32> {
        self.replace_filter(&mut self.hold().await, filter).await
    }

    /// Adds the file at `path`, or with `directory` everything below the folder at `path`, to the
    /// filter's excluded globs, and bumps the revision unless it was excluded already. Returns
    /// the revision.
    pub async fn exclude_path(&self, path: &str, directory: bool) -> Result<u32> {
        let mut st = self.hold().await;
        let mut filter = st.project_filter().clone();
        let glob = exclude_glob(path, directory);
        if !filter.exclude.contains(&glob) {
            filter.exclude.push(glob);
        }
        self.replace_filter(&mut st, filter).await
    }

    async fn replace_filter(&self, st: &mut Held<'_, S::Writes>, filter: ReviewFilter) -> Result<u32> {
        let filter = filter.normalized();
        filter.validate().map_err(SessionError::Invalid)?;
        if *st.project_filter() == filter {
            return Ok(st.revision);
        }
        let writes = st.writes();
        writes.set_filter(self.repo.clone(), filter.clone()).await?;
        st.filter = filter;
        st.project = None;
        self.refilter(st).await
    }

    /// Narrows the review on view to `filter`, which replaces the project's filter until the
    /// narrowing is reset (`None`), the project's filter is edited or another changeset comes
    /// on view. Nothing is stored. Does nothing when `source` is not the changeset on view, which
    /// the viewer left meanwhile. Returns the new revision, or none when nothing changed.
    pub async fn narrow(&self, source: &Source, filter: Option<ReviewFilter>) -> Result<Option<u32>> {
        let mut st = self.hold().await;
        if st.source != *source {
            return Ok(None);
        }
        let project = st.project_filter().clone();
        let (filter, narrowed) = match filter {
            Some(narrowing) => (narrowing.normalized(), true),
            None => (project.clone(), false),
        };
        filter.validate().map_err(SessionError::Invalid)?;
        if st.filter == filter {
            return Ok(None);
        }
        st.filter = filter;
        st.project = narrowed.then_some(project);
        self.refilter(&mut st).await.map(Some)
    }

    /// Counts the review again under `st.filter` as a new revision.
    async fn refilter(&self, st: &mut Held<'_, S::Writes>) -> Result<u32> {
        let writes = st.writes();
        st.revision += 1;
        if st.changes.is_some() {
            for view in st.remapped_views() {
                remap::recount(st, view);
            }
        }
        st.publish();
        self.revisions.advance(st.revision);
        self.note_progress(&writes, st).await;
        Ok(st.revision)
    }
}

/// The filter glob for exactly the file at `path`, or everything below the folder at `path`:
/// anchored at the root, with the characters that are glob syntax matching themselves.
fn exclude_glob(path: &str, directory: bool) -> String {
    let mut glob = String::from("/");
    for c in path.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}') {
            glob.extend(['[', c, ']']);
        } else {
            glob.push(c);
        }
    }
    if directory {
        glob.push_str("/**");
    }
    glob
}
