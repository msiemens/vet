//! What is known of pull requests: the one place the platform's answers about a pull request are
//! kept, however they came (a list, one pull request, its discussion), so that every reader sees the
//! same, most recent, state of it.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use super::discussion::Discussion;
use super::model::{PullRequest, PullRequestHead};
use super::remote::RemoteKey;

/// How many pull requests are kept; the one observed longest ago goes first.
const KEPT: usize = 256;

type Key = (RemoteKey, u64);

struct Known {
    /// When the platform last changed the pull request, as of the observation `head` is from, and
    /// the commit its source branch was at then.
    head: Option<(PullRequestHead, i64, String)>,
    /// What the discussion was when it was last read, and when (unix seconds).
    discussion: Option<(i64, Discussion)>,
    /// Orders the entries by how recently they were observed.
    seen: u64,
}

#[derive(Default)]
struct Entries {
    by_pull_request: HashMap<Key, Known>,
    tick: u64,
}

impl Entries {
    fn entry(&mut self, remote: &RemoteKey, number: u64) -> &mut Known {
        self.tick += 1;
        let tick = self.tick;
        let key = (remote.clone(), number);
        if !self.by_pull_request.contains_key(&key) && self.by_pull_request.len() >= KEPT {
            let oldest = self
                .by_pull_request
                .iter()
                .min_by_key(|(_, k)| k.seen)
                .map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                self.by_pull_request.remove(&oldest);
            }
        }
        let known = self.by_pull_request.entry(key).or_insert(Known {
            head: None,
            discussion: None,
            seen: tick,
        });
        known.seen = tick;
        known
    }
}

/// `head` with the name `discussion` gives the pull request.
fn named_by(head: &mut PullRequestHead, discussion: &Discussion) {
    head.title.clone_from(&discussion.title);
    head.draft = discussion.draft;
    head.target_branch.clone_from(&discussion.target_branch);
}

/// The pull requests the platforms were asked about, by remote and number. Shared by everything
/// that reads or learns of one.
#[derive(Default)]
pub struct PullRequestCache {
    entries: Mutex<Entries>,
}

impl PullRequestCache {
    fn entries(&self) -> std::sync::MutexGuard<'_, Entries> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Notes how the platform names `pull_request` now. An answer older than what is known (a list
    /// kept from before the pull request changed) does not replace it.
    pub fn observe(&self, remote: &RemoteKey, pull_request: &PullRequest) {
        let mut entries = self.entries();
        let known = entries.entry(remote, pull_request.number);
        if known
            .head
            .as_ref()
            .is_some_and(|(_, at, _)| *at > pull_request.updated_at)
        {
            return;
        }
        known.head = Some((
            PullRequestHead::of(pull_request),
            pull_request.updated_at,
            pull_request.head.clone(),
        ));
    }

    /// How the platform last named the pull request. What was not learnt since Vet started is read
    /// from `stored`, how the platform named it when it was last stored, which a discussion read
    /// since is newer than.
    pub fn head(&self, remote: &RemoteKey, number: u64, stored: Option<PullRequestHead>) -> Option<PullRequestHead> {
        let entries = self.entries();
        let known = entries.by_pull_request.get(&(remote.clone(), number));
        if let Some((head, _, _)) = known.and_then(|k| k.head.as_ref()) {
            return Some(head.clone());
        }
        let mut head = stored?;
        if let Some((_, discussion)) = known.and_then(|k| k.discussion.as_ref()) {
            named_by(&mut head, discussion);
        }
        Some(head)
    }

    /// The commit the pull request's source branch was at and the platform's `updatedAt` when it
    /// was last observed; none if it was not.
    pub fn revision(&self, remote: &RemoteKey, number: u64) -> Option<(String, i64)> {
        let entries = self.entries();
        entries
            .by_pull_request
            .get(&(remote.clone(), number))?
            .head
            .as_ref()
            .map(|(_, at, commit)| (commit.clone(), *at))
    }

    /// Notes the discussion of the pull request as read at `at`, which is as current as anything
    /// the platform said of it before: the name it gives the pull request replaces the one known.
    pub fn record_discussion(&self, remote: &RemoteKey, number: u64, at: i64, discussion: Discussion) {
        let mut entries = self.entries();
        let known = entries.entry(remote, number);
        if let Some((head, _, _)) = known.head.as_mut() {
            named_by(head, &discussion);
        }
        known.discussion = Some((at, discussion));
    }

    /// The discussion as last read, and when.
    pub fn discussion(&self, remote: &RemoteKey, number: u64) -> Option<(i64, Discussion)> {
        let entries = self.entries();
        entries
            .by_pull_request
            .get(&(remote.clone(), number))?
            .discussion
            .clone()
    }

    /// Changes the discussion as it is known, for a change the viewer made on the platform that
    /// the next read will confirm. Does nothing if it was not read.
    pub fn amend_discussion(&self, remote: &RemoteKey, number: u64, change: impl FnOnce(&mut Discussion)) {
        let mut entries = self.entries();
        if let Some((_, discussion)) = entries
            .by_pull_request
            .get_mut(&(remote.clone(), number))
            .and_then(|k| k.discussion.as_mut())
        {
            change(discussion);
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::platform::{FakeHosting, pr};

    pub(in crate::platform) fn remote() -> RemoteKey {
        RemoteKey {
            host: "github.com".to_owned(),
            path: "acme/app".to_owned(),
        }
    }

    pub(in crate::platform) fn titled(number: u64, title: &str, updated_at: i64) -> PullRequest {
        let mut pull_request = pr(number);
        pull_request.title = title.to_owned();
        pull_request.updated_at = updated_at;
        pull_request
    }

    #[test]
    fn what_the_platform_said_last_is_known_and_an_older_answer_does_not_replace_it() {
        let cache = PullRequestCache::default();

        cache.observe(&remote(), &titled(7, "Add keys", 200));
        cache.observe(&remote(), &titled(7, "Add key", 100));
        assert_eq!(cache.head(&remote(), 7, None).unwrap().title, "Add keys");

        cache.observe(&remote(), &titled(7, "Add idempotency keys", 300));
        assert_eq!(cache.head(&remote(), 7, None).unwrap().title, "Add idempotency keys");
        assert_eq!(cache.head(&remote(), 8, None), None);
    }

    fn renamed(title: &str) -> Discussion {
        let mut discussion = FakeHosting::default().0.lock().unwrap().discussion.clone();
        discussion.title = title.to_owned();
        discussion
    }

    #[test]
    fn a_discussion_read_renames_the_pull_request_for_everyone_who_asks() {
        let cache = PullRequestCache::default();
        cache.observe(&remote(), &titled(7, "Add keys", 200));

        cache.record_discussion(&remote(), 7, 1_000, renamed("Add idempotency keys"));

        assert_eq!(cache.head(&remote(), 7, None).unwrap().title, "Add idempotency keys");
        assert_eq!(cache.discussion(&remote(), 7).unwrap().0, 1_000);
    }

    #[test]
    fn what_was_stored_is_headed_with_what_a_discussion_read_since_says() {
        let cache = PullRequestCache::default();
        let stored = PullRequestHead::of(&titled(7, "Old title", 1));
        assert_eq!(cache.head(&remote(), 7, Some(stored.clone())), Some(stored.clone()));

        cache.record_discussion(&remote(), 7, 1_000, renamed("New title"));

        let head = cache.head(&remote(), 7, Some(stored.clone())).unwrap();
        assert_eq!((head.title.as_str(), head.fork), ("New title", stored.fork));
        assert_eq!(cache.head(&remote(), 7, None), None, "nothing to name it from");
    }

    #[test]
    fn the_pull_request_observed_longest_ago_is_the_one_that_goes_when_there_is_no_room() {
        let cache = PullRequestCache::default();
        for number in 1..=KEPT as u64 {
            cache.observe(&remote(), &titled(number, "x", 1));
        }
        cache.observe(&remote(), &titled(1, "x", 2));

        cache.observe(&remote(), &titled(KEPT as u64 + 1, "x", 1));

        assert!(cache.head(&remote(), 1, None).is_some(), "it was seen again");
        assert!(cache.head(&remote(), 2, None).is_none());
        assert!(cache.head(&remote(), KEPT as u64 + 1, None).is_some());
    }

    #[test]
    fn a_change_made_to_the_discussion_is_seen_by_the_next_reader() {
        let cache = PullRequestCache::default();
        let discussion = FakeHosting::default().0.lock().unwrap().discussion.clone();
        cache.amend_discussion(&remote(), 7, |d| d.title = "lost".to_owned());
        assert!(cache.discussion(&remote(), 7).is_none(), "nothing was read to amend");

        cache.record_discussion(&remote(), 7, 5, discussion);
        cache.amend_discussion(&remote(), 7, |d| d.title = "Amended".to_owned());

        assert_eq!(cache.discussion(&remote(), 7).unwrap().1.title, "Amended");
    }
}
