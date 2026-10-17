//! The base of the changeset on view with its display facts.

use super::state::State;
use super::views::{BaseView, Contained};
use super::{LiveSession, Result, manifest};
use crate::base;
use crate::changeset::Source;
use crate::ports::{GitRepo, LiveStore};

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// The base with its display facts; `current` is the branch the working tree is on.
    pub(super) async fn base_view(&self, st: &State, current: Option<&str>) -> Result<BaseView> {
        let own = manifest::target_view(&st.source, &st.head, current).branch;
        let contained = base::contained_in(&st.base_choice, &st.base_commit, &st.head, own.as_deref());
        Ok(BaseView {
            commit: self.facts.commit_info(&st.base_commit).await?,
            contained_in: match contained {
                Some(reference) => Some(self.contained(st, &reference).await),
                None => None,
            },
            into: self.merged_into(st).await?,
            head_off_base: st.off_base,
        })
    }

    /// What there is to say of a changeset whose tip `reference` already holds. What git cannot
    /// say is left out: the changeset is empty either way.
    async fn contained(&self, st: &State, reference: &str) -> Contained {
        let git = &self.acquisition.git;
        let default = git
            .default_branch()
            .await
            .map_err(|e| tracing::warn!(error = %e, "the default branch is not found"))
            .ok()
            .flatten();
        let target = match &st.source {
            Source::PullRequest { .. } => self
                .pull_request_head(&st.source)
                .await
                .ok()
                .flatten()
                .map(|head| head.target_branch),
            _ => None,
        };
        let branch = target.unwrap_or_else(|| base::reference_name(reference, default.as_deref()));
        let reference_tip = git
            .resolve_commit(reference)
            .await
            .map_err(|e| tracing::warn!(error = %e, reference, "the branch holding the changeset cannot be read"))
            .ok()
            .flatten();
        let same_tip = reference_tip.as_ref() == Some(&st.head);
        let merged = match &reference_tip {
            Some(tip) if !same_tip => self.facts.merged_into_branch(&st.head, tip).await,
            _ => None,
        };
        let (merged_by, commits, since) = match merged {
            Some((info, commits, since)) => (Some(info), commits, since),
            None => (None, None, None),
        };
        Contained {
            branch,
            same_tip,
            merged_by,
            commits,
            since,
        }
    }

    /// The branch a pull request is merged into, when that is not the repository's default branch.
    async fn merged_into(&self, st: &State) -> Result<Option<String>> {
        if !matches!(st.source, Source::PullRequest { .. }) {
            return Ok(None);
        }
        let Some(head) = self.pull_request_head(&st.source).await? else {
            tracing::warn!(source = ?st.source, "the target branch of the pull request is not stored");
            return Ok(None);
        };
        // With no default branch to compare with, the target is said.
        let default = self.acquisition.git.default_branch().await?;
        Ok((default.as_ref() != Some(&head.target_branch)).then_some(head.target_branch))
    }
}
