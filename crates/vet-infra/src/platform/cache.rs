//! What an adapter remembers of the platform's answers: a bounded map whose entries carry the
//! time they were stored, so the caller says how old an entry may be.

use std::collections::HashMap;
use std::hash::Hash;

pub(super) struct Cache<K, V> {
    capacity: usize,
    entries: HashMap<K, (i64, V)>,
}

impl<K: Hash + Eq + Clone, V> Cache<K, V> {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: HashMap::new(),
        }
    }

    pub(super) fn get(&self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|(_, value)| value)
    }

    /// The entry, if it was stored less than `max_age` seconds before `now`.
    pub(super) fn fresh(&self, key: &K, now: i64, max_age: i64) -> Option<&V> {
        self.entries
            .get(key)
            .filter(|(at, _)| now - at < max_age)
            .map(|(_, value)| value)
    }

    /// Stores `value`; at capacity the entry stored longest ago makes room. Returns what the key
    /// held before.
    pub(super) fn insert(&mut self, key: K, value: V, now: i64) -> Option<V> {
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&key) {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key, (now, value)).map(|(_, old)| old)
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&K, &V) -> bool) {
        self.entries.retain(|key, (_, value)| keep(key, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_is_fresh_for_less_than_its_max_age() {
        let mut cache = Cache::new(4);
        cache.insert("a", 1, 100);
        assert_eq!(cache.fresh(&"a", 109, 10), Some(&1));
        assert_eq!(cache.fresh(&"a", 110, 10), None);
        assert_eq!(cache.get(&"a"), Some(&1));
    }

    #[test]
    fn a_full_cache_drops_the_entry_stored_longest_ago_and_never_the_one_replaced() {
        let mut cache = Cache::new(2);
        cache.insert("old", 1, 1);
        cache.insert("new", 2, 2);
        assert_eq!(cache.insert("new", 3, 3), Some(2));
        assert_eq!(cache.get(&"old"), Some(&1));
        cache.insert("newest", 4, 4);
        assert_eq!(cache.get(&"old"), None);
        assert_eq!(cache.get(&"new"), Some(&3));
        assert_eq!(cache.get(&"newest"), Some(&4));
    }
}
