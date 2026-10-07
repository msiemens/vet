//! The newest revision known. Semantic work for an older one is worth nothing: its
//! positions and results cannot be shown against the newer revision, and the viewer
//! asks again. The session announces every move of its revision here, and the
//! semantic layer drops the work of the ones left behind.

use tokio::sync::watch;
use vet_semantic::join::{Either, race};

pub struct Revisions(watch::Sender<u32>);

impl Default for Revisions {
    fn default() -> Self {
        Self(watch::channel(0).0)
    }
}

impl Revisions {
    /// Records that `revision` exists; an older one than is known changes nothing.
    pub fn advance(&self, revision: u32) {
        self.0.send_if_modified(|newest| {
            let newer = revision > *newest;
            if newer {
                *newest = revision;
            }
            newer
        });
    }

    /// The result of `work`, unless a revision newer than `revision` is or becomes known
    /// first: then `work` is dropped and the newer revision is returned.
    pub async fn until_superseded<T>(&self, revision: u32, work: impl Future<Output = T>) -> Result<T, u32> {
        self.advance(revision);
        let mut newest = self.0.subscribe();
        let superseded = async {
            // The sender lives as long as `self`, so waiting never fails.
            let current = newest.wait_for(|newest| *newest > revision).await.map(|newest| *newest);
            current.unwrap_or(revision)
        };
        match race(superseded, work).await {
            Either::Left(current) => Err(current),
            Either::Right(done) => Ok(done),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn revisions_run_work_to_the_end_while_nothing_newer_is_known() {
        let revisions = Revisions::default();
        assert_eq!(revisions.until_superseded(2, async { "done" }).await, Ok("done"));
        // Older news changes nothing.
        revisions.advance(1);
        assert_eq!(revisions.until_superseded(2, async { "again" }).await, Ok("again"));
        assert_eq!(revisions.until_superseded(1, async { "late" }).await, Err(2));
    }
}
