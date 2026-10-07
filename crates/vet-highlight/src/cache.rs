use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex, PoisonError};

use crate::{Language, encode, highlight};

/// Encoded highlights by the identity of the text (a blob id) and its language, so a text
/// seen again is never parsed again. Bounded by the size of what it holds: the entry used
/// longest ago goes first.
pub struct Cache<K> {
    capacity: usize,
    inner: Mutex<Inner<K>>,
}

struct Inner<K> {
    entries: HashMap<(K, Language), Entry>,
    /// Bytes held.
    size: usize,
    /// Counts uses, to order them.
    clock: u64,
}

struct Entry {
    tokens: Arc<[u32]>,
    used: u64,
}

fn size_of(tokens: &[u32]) -> usize {
    std::mem::size_of_val(tokens)
}

impl<K: Hash + Eq + Clone> Cache<K> {
    /// A cache holding at most `capacity` bytes of highlights.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                size: 0,
                clock: 0,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner<K>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The encoded highlights of `text`, known by `key` and `language`. The lock is not held
    /// while parsing, so two threads asking for the same text may both parse it.
    pub fn get(&self, key: &K, text: &str, language: Language) -> Arc<[u32]> {
        let entry = (key.clone(), language);
        {
            let mut inner = self.lock();
            inner.clock += 1;
            let clock = inner.clock;
            if let Some(hit) = inner.entries.get_mut(&entry) {
                hit.used = clock;
                return hit.tokens.clone();
            }
        }
        let tokens: Arc<[u32]> = encode(&highlight(text, language)).into();
        let mut inner = self.lock();
        // An entry bigger than the cache would only push everything else out.
        if size_of(&tokens) <= self.capacity {
            inner.clock += 1;
            let used = inner.clock;
            inner.size += size_of(&tokens);
            let replaced = inner.entries.insert(
                entry,
                Entry {
                    tokens: tokens.clone(),
                    used,
                },
            );
            inner.size -= replaced.map_or(0, |e| size_of(&e.tokens));
            while inner.size > self.capacity {
                let Some(oldest) = inner.entries.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| k.clone()) else {
                    break;
                };
                if let Some(gone) = inner.entries.remove(&oldest) {
                    inner.size -= size_of(&gone.tokens);
                }
            }
        }
        tokens
    }

    /// The bytes of highlights held.
    pub fn size(&self) -> usize {
        self.lock().size
    }
}
