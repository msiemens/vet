//! What is known of pull requests: the one place the platform's answers about a pull request are
//! kept, however they came (a list, one pull request, its discussion), so that every reader sees the
//! same, most recent, state of it.

use std::collections::HashMap;
use std::sync::Mutex;

use vet_core::num::sat_u32;

use crate::mutex::lock;

use super::discussion::{Discussion, ReviewerState};
use super::model::{ChangesAsked, ChangesRequest, PullRequest, PullRequestHead, ReviewDecision};
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

/// What the queue shows of a pull request that a list of some platforms leaves to its discussion.
#[derive(Default, PartialEq, Eq)]
struct Details {
    decision: ReviewDecision,
    blocked: bool,
    unresolved_threads: u32,
    changes_requested: Vec<ChangesRequest>,
}

impl Details {
    fn of(discussion: &Discussion) -> Self {
        Self {
            decision: discussion.decision,
            blocked: discussion.blocked,
            unresolved_threads: sat_u32(discussion.threads.iter().filter(|t| !t.resolved).count()),
            changes_requested: discussion
                .reviewers
                .iter()
                .filter(|r| r.state == ReviewerState::ChangesRequested)
                .map(|r| ChangesRequest {
                    reviewer: r.name.clone(),
                    how: ChangesAsked::Requested,
                })
                .collect(),
        }
    }
}

/// The pull requests the platforms were asked about, by remote and number. Shared by everything
/// that reads or learns of one.
#[derive(Default)]
pub struct PullRequestCache {
    entries: Mutex<Entries>,
}

impl PullRequestCache {
    fn entries(&self) -> std::sync::MutexGuard<'_, Entries> {
        lock(&self.entries)
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
    /// Answers whether that changes what [`Self::fill`] puts into a listed pull request.
    pub fn record_discussion(&self, remote: &RemoteKey, number: u64, at: i64, discussion: Discussion) -> bool {
        let mut entries = self.entries();
        let known = entries.entry(remote, number);
        if let Some((head, _, _)) = known.head.as_mut() {
            named_by(head, &discussion);
        }
        let shown = known
            .discussion
            .as_ref()
            .map_or_else(Details::default, |(_, d)| Details::of(d));
        let changed = shown != Details::of(&discussion);
        known.discussion = Some((at, discussion));
        changed
    }

    /// Completes `pull_request`, as a platform whose lists leave out its details listed it, with
    /// what its discussion said when it was last read. A pull request changed since shows the
    /// details as of that read until the next one.
    pub fn fill(&self, remote: &RemoteKey, pull_request: &mut PullRequest) {
        let entries = self.entries();
        let read = entries
            .by_pull_request
            .get(&(remote.clone(), pull_request.number))
            .and_then(|k| k.discussion.as_ref());
        if let Some((_, discussion)) = read {
            let details = Details::of(discussion);
            pull_request.decision = details.decision;
            pull_request.blocked = details.blocked;
            pull_request.unresolved_threads = details.unresolved_threads;
            pull_request.changes_requested = details.changes_requested;
        }
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

    pub(in crate::platform) fn reviewed(unresolved: usize, resolved: usize, asking_changes: &[&str]) -> Discussion {
        use crate::platform::{DiffSide, PlatformThreadData, Reviewer};
        let thread = |resolved| PlatformThreadData {
            id: String::new(),
            path: "a.rs".to_owned(),
            side: DiffSide::Right,
            lines: None,
            outdated: false,
            resolved,
            resolved_by: None,
            comments: Vec::new(),
        };
        let mut discussion = renamed("Add keys");
        discussion.threads = (0..unresolved)
            .map(|_| thread(false))
            .chain((0..resolved).map(|_| thread(true)))
            .collect();
        discussion.reviewers = asking_changes
            .iter()
            .map(|name| Reviewer {
                login: (*name).to_owned(),
                name: (*name).to_owned(),
                state: ReviewerState::ChangesRequested,
            })
            .chain([Reviewer {
                login: "ada".to_owned(),
                name: "Ada".to_owned(),
                state: ReviewerState::Approved,
            }])
            .collect();
        discussion
    }

    #[test]
    fn a_listed_pull_request_is_completed_with_the_review_state_its_discussion_had() {
        let cache = PullRequestCache::default();
        let mut listed = pr(7);
        cache.fill(&remote(), &mut listed);
        assert_eq!((listed.unresolved_threads, listed.changes_requested.len()), (0, 0));

        let mut read = reviewed(2, 1, &["Linus"]);
        read.blocked = true;
        read.decision = ReviewDecision::ChangesRequested;
        cache.record_discussion(&remote(), 7, 1_000, read);
        cache.fill(&remote(), &mut listed);

        assert_eq!(
            (listed.blocked, listed.decision),
            (true, ReviewDecision::ChangesRequested)
        );
        assert_eq!(listed.unresolved_threads, 2, "the resolved one is not counted");
        assert_eq!(
            listed.changes_requested,
            [ChangesRequest {
                reviewer: "Linus".to_owned(),
                how: ChangesAsked::Requested
            }],
            "who asked for changes, not who approved"
        );
        let mut other = pr(8);
        cache.fill(&remote(), &mut other);
        assert_eq!(
            other.unresolved_threads, 0,
            "another pull request's discussion is not this one's"
        );
    }

    #[test]
    fn reading_a_discussion_says_whether_the_list_shows_something_else_afterwards() {
        let cache = PullRequestCache::default();

        let first = cache.record_discussion(&remote(), 7, 1, reviewed(2, 0, &[]));
        let again = cache.record_discussion(&remote(), 7, 2, reviewed(2, 3, &[]));
        let resolved = cache.record_discussion(&remote(), 7, 3, reviewed(0, 5, &[]));
        let nothing_to_show = cache.record_discussion(&remote(), 8, 4, reviewed(0, 5, &[]));
        let mut held_back = reviewed(0, 5, &[]);
        held_back.blocked = true;
        let blocked = cache.record_discussion(&remote(), 7, 5, held_back);

        assert_eq!(
            (first, again, resolved, nothing_to_show, blocked),
            (true, false, true, false, true),
            "only a different count, reviewer, decision or merge state changes what the list shows"
        );
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
