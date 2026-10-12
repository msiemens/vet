//! What the platforms' rate limits have in common: a limit that a response announced holds until
//! its end, requests that meet it are not sent, and each limit window is logged once. How a
//! platform's responses announce a limit is its own business.

use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::sync::{Mutex, PoisonError};

/// Until when each limited thing (a host and bucket, an account) is used up.
pub(super) struct Gate<K> {
    until: Mutex<HashMap<K, i64>>,
}

impl<K> Default for Gate<K> {
    fn default() -> Self {
        Self {
            until: Mutex::default(),
        }
    }
}

impl<K: Hash + Eq + Clone> Gate<K> {
    /// When the limit of `key` ends, if it is used up at `now`.
    pub(super) fn blocked_until(&self, key: &K, now: i64) -> Option<i64> {
        let until = self.until.lock().unwrap_or_else(PoisonError::into_inner);
        until.get(key).copied().filter(|&at| at > now)
    }

    /// `key` is used up until `until`. Whether this report begins a window: the first report of a
    /// limit does, the ones for the same window do not.
    pub(super) fn block(&self, key: &K, until: i64, now: i64) -> bool {
        let mut windows = self.until.lock().unwrap_or_else(PoisonError::into_inner);
        let current = windows.entry(key.clone()).or_insert(0);
        let begins = *current <= now;
        *current = (*current).max(until);
        begins
    }
}

/// A request that was not sent: the limit it counts against is used up until `until`.
#[derive(Debug)]
pub(super) struct Blocked {
    pub(super) until: i64,
}

impl fmt::Display for Blocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the rate limit is used up until {}", self.until)
    }
}

impl std::error::Error for Blocked {}
