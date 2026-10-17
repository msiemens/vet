use vet_core::review::{Outgoing, PendingVote, Verdict};

use super::{VoteSent, pull_request};
use crate::changeset::Source;
use crate::platform::{Discussion, Platform, PlatformError};
use crate::ports::{GitRepo, LiveStore, ReviewWrites};
use crate::session::hold::Held;
use crate::session::{LiveSession, Result, SessionError};

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// The vote of the viewer's on the pull request on view that waits to be sent; `None` without
    /// one, and for another changeset.
    pub async fn pending_vote(&self) -> Result<Option<PendingVote>> {
        let source = self.published.read().source.clone();
        if !matches!(source, Source::PullRequest { .. }) {
            return Ok(None);
        }
        Ok(self.acquisition.store.pending_vote(self.repo.clone(), source).await?)
    }

    /// The pending vote if sending it is up to a post: not one that is being posted, which the next
    /// sync settles.
    pub(super) async fn vote_to_send(&self) -> Result<Option<PendingVote>> {
        Ok(self
            .pending_vote()
            .await?
            .filter(|v| !matches!(v.outgoing, Outgoing::Posting { .. })))
    }

    /// Stages `verdict` (one of the platform's) as the viewer's vote on a platform that keeps it
    /// apart from posting, to be sent with the next post. It replaces a vote staged before; the
    /// viewer's current vote on the platform stages nothing and clears the one staged.
    pub async fn stage_vote<P: Platform + Sync>(&self, platform: &P, verdict: Verdict) -> Result<()> {
        let traits = platform.traits();
        if !traits.verdict_separate {
            return Err(SessionError::Invalid(
                "this platform sets the vote with the review".into(),
            ));
        }
        if !traits.verdicts.contains(&verdict) {
            return Err(SessionError::Invalid(format!("the platform has no verdict {verdict}")));
        }
        let _exchange = self.exchange.lock().await;
        let current = {
            let st = self.published.read();
            let (remote, number) = pull_request(&st)?;
            (self.exchange.cache.discussion(&remote, number)).and_then(|(_, d)| d.viewer_verdict)
        };
        let vote = (current.as_ref() != Some(&verdict)).then_some(PendingVote {
            verdict,
            outgoing: Outgoing::Draft,
        });
        self.store_vote(vote).await
    }

    /// Drops the vote that waits to be sent. One that is being posted stays until that is settled.
    pub async fn discard_vote(&self) -> Result<()> {
        let _exchange = self.exchange.lock().await;
        if matches!(
            self.pending_vote().await?,
            Some(PendingVote {
                outgoing: Outgoing::Posting { .. },
                ..
            })
        ) {
            return Err(SessionError::Invalid(
                "the vote is being posted: discard it once that is done".into(),
            ));
        }
        self.store_vote(None).await
    }

    async fn store_vote(&self, vote: Option<PendingVote>) -> Result<()> {
        let st = self.hold().await;
        let source = pull_request(&st).map(|_| st.source.clone())?;
        st.writes().set_pending_vote(self.repo.clone(), source, vote).await?;
        Ok(())
    }

    /// Sends the pending vote to the platform. Written as posting before the platform is asked, so
    /// that a vote Vet did not see through is looked for at the next sync. Answers what became of
    /// it, and why if it failed.
    pub(super) async fn send_vote<P: Platform + Sync>(
        &self,
        platform: &P,
    ) -> Result<(VoteSent, Option<PlatformError>)> {
        let _exchange = self.exchange.lock().await;
        let Some(vote) = self.vote_to_send().await? else {
            return Ok((VoteSent::NotSent, None));
        };
        let (remote, number) = pull_request(&self.published.read())?;
        self.store_vote(Some(PendingVote {
            outgoing: Outgoing::Posting { since: (self.clock)() },
            ..vote.clone()
        }))
        .await?;
        match platform.vote(&remote, number, &vote.verdict).await {
            Ok(()) => {
                self.store_vote(None).await?;
                Ok((VoteSent::Cast, None))
            }
            // The platform may have taken it: the next sync looks.
            Err(error @ PlatformError::Unreachable(_)) => Ok((VoteSent::Failed, Some(error))),
            Err(error) => {
                self.store_vote(Some(PendingVote {
                    outgoing: Outgoing::Failed {
                        error: error.to_string(),
                    },
                    ..vote
                }))
                .await?;
                Ok((VoteSent::Failed, Some(error)))
            }
        }
    }

    /// Settles a vote a post left posting, by what the platform has: the viewer's vote there is the
    /// pending one, or it was not taken. A vote the platform already has needs no sending.
    pub(super) async fn settle_vote(&self, st: &mut Held<'_, S::Writes>, discussion: &Discussion) -> Result<bool> {
        let source = st.source.clone();
        let Some(vote) = self
            .acquisition
            .store
            .pending_vote(self.repo.clone(), source.clone())
            .await?
        else {
            return Ok(false);
        };
        let settled = if discussion.viewer_verdict.as_ref() == Some(&vote.verdict) {
            None
        } else if matches!(vote.outgoing, Outgoing::Posting { .. }) {
            Some(PendingVote {
                outgoing: Outgoing::Draft,
                ..vote
            })
        } else {
            return Ok(false);
        };
        st.writes().set_pending_vote(self.repo.clone(), source, settled).await?;
        Ok(true)
    }
}
