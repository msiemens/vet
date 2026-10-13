//! Trust (§7.5) as the session keeps it: the project's, and the pull request on view's at one
//! head commit. Neither is review state: they decide what analysis may execute.

use super::{LiveSession, Result, SessionError};
use crate::changeset::{CommitId, Source};
use crate::ports::{GitRepo, LiveStore, ReviewWrites};
use crate::trust::Trust;

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// Marks the repository trusted, or not.
    pub async fn set_project_trusted(&self, trusted: bool) -> Result<()> {
        let st = self.hold().await;
        st.writes().set_project_trusted(self.repo.clone(), trusted).await?;
        Ok(())
    }

    /// Trusts the pull request on view at its current head: a push to it ends this.
    pub async fn trust_pull_request(&self) -> Result<()> {
        let st = self.hold().await;
        if !matches!(st.source, Source::PullRequest { .. }) {
            return Err(SessionError::Invalid(
                "the changeset on view is not a pull request".into(),
            ));
        }
        st.writes()
            .set_pull_request_trusted_head(self.repo.clone(), st.source.clone(), Some(st.head.clone()))
            .await?;
        Ok(())
    }

    /// The trust of the changeset on view, and the commit its target is built on.
    pub async fn trust(&self) -> Result<(Trust, CommitId)> {
        let (source, head) = {
            let st = self.published.read();
            (st.source.clone(), st.head.clone())
        };
        Ok((self.view().trust(&source).await?, head))
    }
}
