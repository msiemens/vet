//! What the semantic layer reads of a session: the published state's
//! snapshots and diff of a revision, and where the content of their files comes from. A
//! [`RevisionView`] reads and nothing else: it has no part in the commands that change review state,
//! never takes the write token, and has no handle to the review database, so semantic work can
//! never make a review command wait, and no review command waits for it.

use std::collections::BTreeMap;
use std::ops::Deref;
use std::sync::Arc;

use vet_core::content::{EntryKind, FileDiff, RepoPath};
use vet_core::review::Side;

use super::derived::{Derived, Diffed};
use super::hold::Published;
use super::state::check_revision;
use super::views::SemanticSides;
use super::{LiveSession, Result};
use crate::acquisition::Acquisition;
use crate::changeset::{ChangesetKind, Source};
use crate::ports::{GitRepo, LiveStore};
use crate::trust::{Origin, PullRequestTrust, Trust};

/// The text diff of a revision.
pub struct ChangedFiles(Arc<Diffed>);

impl Deref for ChangedFiles {
    type Target = [FileDiff];

    fn deref(&self) -> &[FileDiff] {
        &self.0.diff.files
    }
}

pub struct RevisionView<G, S> {
    published: Arc<Published>,
    derived: Arc<Derived>,
    acquisition: Arc<Acquisition<G, S>>,
    /// The repository root, which with a source is the key a changeset is stored under.
    repo: String,
}

impl<G, S> Clone for RevisionView<G, S> {
    fn clone(&self) -> Self {
        Self {
            published: Arc::clone(&self.published),
            derived: Arc::clone(&self.derived),
            acquisition: Arc::clone(&self.acquisition),
            repo: self.repo.clone(),
        }
    }
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// What the semantic layer reads of the session.
    pub fn view(&self) -> RevisionView<G, S> {
        RevisionView {
            published: Arc::clone(&self.published),
            derived: Arc::clone(&self.derived),
            acquisition: Arc::clone(&self.acquisition),
            repo: self.repo.clone(),
        }
    }
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> RevisionView<G, S> {
    /// What decides how far analysis of the changeset of `source` may execute project code: the
    /// project's trust and, for a pull request, what is stored of it.
    pub async fn trust(&self, source: &Source) -> Result<Trust> {
        let store = &self.acquisition.store;
        let project = store.project_trusted(self.repo.clone()).await?;
        let origin = match source.kind() {
            ChangesetKind::PullRequest => Origin::PullRequest(
                store
                    .pull_request_trust(self.repo.clone(), source.clone())
                    .await?
                    .unwrap_or_else(PullRequestTrust::unknown),
            ),
            ChangesetKind::WorkingTree | ChangesetKind::Branch => Origin::Local,
        };
        Ok(Trust { project, origin })
    }

    /// What knows the origin of the session's snapshots, for serving them to a
    /// language-server pool and reading their files.
    pub fn acquisition(&self) -> Arc<Acquisition<G, S>> {
        Arc::clone(&self.acquisition)
    }

    /// The snapshots of `revision`: what the semantic layer asks about. Fails
    /// if `revision` is no longer current.
    pub async fn semantic_sides(&self, revision: u32) -> Result<SemanticSides> {
        let st = self.published.read();
        check_revision(&st, revision)?;
        Ok(SemanticSides {
            base: st.base.id,
            base_commit: st.base_commit.clone(),
            target: st.target.id,
            head: st.head.clone(),
            live_target: st.source.kind() == ChangesetKind::WorkingTree,
            source: st.source.clone(),
        })
    }

    /// For each key `key_of` yields over the files of `revision`'s target
    /// snapshot, the first file with it; stops once `limit` keys are found.
    pub async fn target_file_per_key<K: PartialEq>(
        &self,
        revision: u32,
        limit: usize,
        key_of: impl Fn(&RepoPath) -> Option<K>,
    ) -> Result<Vec<(K, RepoPath)>> {
        let st = self.published.read();
        check_revision(&st, revision)?;
        let mut found: Vec<(K, RepoPath)> = Vec::new();
        for path in st.target.entries.keys() {
            if found.len() >= limit {
                break;
            }
            if let Some(key) = key_of(path)
                && !found.iter().any(|(k, _)| *k == key)
            {
                found.push((key, path.clone()));
            }
        }
        Ok(found)
    }

    /// The text of `path` on `side` of `revision`, changed or not; `None` where the file is not
    /// there or has no readable text.
    pub async fn side_text(&self, revision: u32, side: Side, path: &RepoPath) -> Result<Option<String>> {
        let entry = {
            let st = self.published.read();
            check_revision(&st, revision)?;
            let snapshot = match side {
                Side::Base => &st.base,
                Side::Target => &st.target,
            };
            snapshot
                .entries
                .get(path)
                .filter(|e| e.kind == EntryKind::File)
                .map(|e| e.blob)
        };
        let Some(blob) = entry else {
            return Ok(None);
        };
        let text = self.acquisition.text_of(&BTreeMap::new(), path, blob).await?;
        Ok(text.ok())
    }

    /// The text diff of `revision`, for the analyses that map symbols to hunks. It is the state's
    /// own, or the engine's: kept, joined while it is computed, or computed, without the state
    /// being touched.
    pub async fn changed_files(&self, revision: u32) -> Result<ChangedFiles> {
        let (base, target) = {
            let st = self.published.read();
            check_revision(&st, revision)?;
            if let Some(changes) = &st.changes {
                return Ok(ChangedFiles(Arc::clone(&changes.diffed)));
            }
            (Arc::clone(&st.base), Arc::clone(&st.target))
        };
        let diffed = self.derived.diff(&self.acquisition, &base, &target).await?;
        Ok(ChangedFiles(diffed))
    }
}
