//! A map that holds at most a fixed number of entries: a full one drops the entry used longest ago.

use std::collections::HashMap;
use std::hash::Hash;

/// Entries by key, at most `capacity` of them. Ordering is by use, not by insertion, so a value
/// read again is not the next to go.
pub(crate) struct Lru<K, V> {
    capacity: usize,
    clock: u64,
    entries: HashMap<K, (u64, V)>,
}

impl<K: Eq + Hash + Clone, V> Lru<K, V> {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            clock: 0,
            entries: HashMap::new(),
        }
    }

    /// The value under `key`, noted as used now.
    pub(crate) fn get(&mut self, key: &K) -> Option<&V> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self.entries.get_mut(key)?;
        entry.0 = clock;
        Some(&entry.1)
    }

    /// Keeps `value` under `key`; a full map drops the entry used longest ago first.
    pub(crate) fn insert(&mut self, key: K, value: V) {
        self.clock += 1;
        let clock = self.clock;
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&key) {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key, (clock, value));
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_used_recently_survives_the_map_filling() {
        let mut lru = Lru::new(3);
        lru.insert("a", 1);
        lru.insert("b", 2);
        lru.insert("c", 3);
        // "a" is the oldest, but using it makes "b" the one to go when the next entry lands.
        assert_eq!(lru.get(&"a"), Some(&1));
        lru.insert("d", 4);
        assert_eq!(lru.len(), 3);
        assert_eq!(lru.get(&"a"), Some(&1), "the entry used recently is kept");
        assert_eq!(lru.get(&"b"), None, "the entry used longest ago is dropped");
        assert_eq!(lru.get(&"c"), Some(&3));
        assert_eq!(lru.get(&"d"), Some(&4));
    }

    #[test]
    fn replacing_a_key_keeps_the_map_size() {
        let mut lru = Lru::new(2);
        lru.insert("a", 1);
        lru.insert("b", 2);
        lru.insert("a", 10);
        assert_eq!(lru.len(), 2);
        assert_eq!(lru.get(&"a"), Some(&10));
        assert_eq!(lru.get(&"b"), Some(&2));
    }
}
