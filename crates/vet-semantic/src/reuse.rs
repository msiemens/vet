//! Reuse of answers (I2, §7.6). An answer is kept under a key that names
//! everything it depends on, and is reused if and only if a recomputed key is
//! equal. Nothing is ever judged valid by time, by events or by a guess: a changed
//! input is a different key, and the old answer is left to age out.
//!
//! - Symbols of a text depend on the text, the language and the server alone
//!   ([`Key::Text`]), so they are reused on every tree and both sides.
//! - Answers that resolve across files depend on a whole immutable tree: its
//!   snapshot, the analysis policy and how its dependencies were restored
//!   ([`Key::Project`]).
//! - Both name the server that gave them: binary, version, settings, toolchain.
//! - Answers of the live tree, which changes under the server, have no key.
//!
//! An answer the server gave while it was at work on its own is not kept, and
//! neither is an error.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use vet_core::content::{BlobId, Hash as Fingerprint, RepoPath, SnapshotId};

use crate::once::{Producer, Shared};
use crate::policy::AnalysisPolicy;
use crate::ports::{At, Diagnostic, Location, ServerIdentity, SymbolInfo};

/// How the dependencies of an immutable tree were restored.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Restore {
    /// Equal fingerprints mean equal dependencies.
    Recorded(Fingerprint),
    /// Nothing was recorded of the restore, so nothing is known about whether another
    /// server's tree has the same dependencies: only the server that restored it
    /// (the numbered instance) may reuse what it was told.
    Instance(u64),
}

/// What an immutable tree is, as far as project-wide answers go.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Basis {
    pub snapshot: SnapshotId,
    pub policy: AnalysisPolicy,
    pub restore: Restore,
}

/// A question about the whole tree.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ProjectQuery {
    DocumentSymbols(RepoPath),
    Definition(At),
    References(At),
    Hover(At),
    Incoming(At),
    Outgoing(At),
    Workspace(String),
    Diagnostics(RepoPath),
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    /// Depends on one text: the symbols it declares.
    Text {
        server: ServerIdentity,
        /// The file name's extension, which tells the server the language.
        extension: Vec<u8>,
        /// The text that was sent.
        blob: BlobId,
    },
    /// Depends on a whole immutable tree.
    Project {
        server: ServerIdentity,
        basis: Basis,
        query: ProjectQuery,
    },
}

/// What is kept: the answers of the primitive queries.
#[derive(Clone, Debug)]
pub enum Value {
    Symbols(Vec<SymbolInfo>),
    Locations(Vec<Location>),
    Hover(Option<String>),
    Diagnostics(Vec<Diagnostic>),
}

/// A type of answer that can be kept.
pub trait Reused: Clone {
    fn kept(self) -> Value;
    fn from_kept(value: Value) -> Option<Self>;
}

macro_rules! reused {
    ($ty:ty, $variant:ident) => {
        impl Reused for $ty {
            fn kept(self) -> Value {
                Value::$variant(self)
            }

            fn from_kept(value: Value) -> Option<Self> {
                match value {
                    Value::$variant(inner) => Some(inner),
                    _ => None,
                }
            }
        }
    };
}

reused!(Vec<SymbolInfo>, Symbols);
reused!(Vec<Location>, Locations);
reused!(Option<String>, Hover);
reused!(Vec<Diagnostic>, Diagnostics);

/// A computed value and whether it may be kept.
pub struct Computed<V> {
    pub value: V,
    pub keep: bool,
}

enum Slot<V> {
    /// Someone is computing it; others wait for that.
    Pending(Arc<Shared<V>>),
    Done {
        value: V,
        used: u64,
    },
}

struct State<K, V> {
    tick: u64,
    slots: HashMap<K, Slot<V>>,
}

/// Values by key. A value is computed once, however many callers ask while it is;
/// if the one computing gives up, or the value may not be kept, the others compute
/// it themselves. The least recently used values go beyond `limit`; a limit of 0
/// keeps nothing and shares nothing.
pub struct Memo<K, V> {
    limit: usize,
    state: Mutex<State<K, V>>,
}

/// A computation under way. If its caller is dropped before it ended, the slot is
/// freed before the waiters are woken, so one of them computes the value next.
struct Computing<'m, K: Eq + Hash, V> {
    memo: &'m Memo<K, V>,
    key: &'m K,
    producer: Option<Producer<V>>,
}

impl<K: Eq + Hash, V> Drop for Computing<'_, K, V> {
    fn drop(&mut self) {
        if self.producer.is_some() {
            self.memo.lock().slots.remove(self.key);
        }
    }
}

impl<K, V> Memo<K, V> {
    fn lock(&self) -> std::sync::MutexGuard<'_, State<K, V>> {
        self.state.lock().expect("no panic while the memo is locked")
    }
}

impl<K: Eq + Hash + Clone, V: Clone> Memo<K, V> {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            state: Mutex::new(State {
                tick: 0,
                slots: HashMap::new(),
            }),
        }
    }

    /// The value for `key` and whether it was reused. Otherwise `compute` runs.
    pub async fn get<E>(&self, key: &K, compute: impl Future<Output = Result<Computed<V>, E>>) -> Result<(V, bool), E> {
        if self.limit == 0 {
            return compute.await.map(|c| (c.value, false));
        }
        enum Plan<V> {
            Hit(V),
            Wait(Arc<Shared<V>>),
            Compute(Producer<V>),
        }
        let mut compute = Some(compute);
        loop {
            let plan = {
                let mut state = self.lock();
                state.tick += 1;
                let tick = state.tick;
                match state.slots.get_mut(key) {
                    Some(Slot::Done { value, used }) => {
                        *used = tick;
                        Plan::Hit(value.clone())
                    }
                    Some(Slot::Pending(shared)) => Plan::Wait(shared.clone()),
                    None => {
                        let (shared, producer) = Shared::start();
                        state.slots.insert(key.clone(), Slot::Pending(shared));
                        Plan::Compute(producer)
                    }
                }
            };
            match plan {
                Plan::Hit(value) => return Ok((value, true)),
                Plan::Wait(shared) => {
                    if let Some(value) = shared.wait().await {
                        return Ok((value, true));
                    }
                }
                Plan::Compute(producer) => {
                    let mut computing = Computing {
                        memo: self,
                        key,
                        producer: Some(producer),
                    };
                    let Computed { value, keep } = compute.take().expect("computed once").await?;
                    let producer = computing.producer.take().expect("not ended yet");
                    {
                        let mut state = self.lock();
                        if keep {
                            let used = state.tick;
                            state.slots.insert(
                                key.clone(),
                                Slot::Done {
                                    value: value.clone(),
                                    used,
                                },
                            );
                            self.evict(&mut state);
                        } else {
                            state.slots.remove(key);
                        }
                    }
                    if keep {
                        producer.finish(value.clone());
                    }
                    return Ok((value, false));
                }
            }
        }
    }

    fn evict(&self, state: &mut State<K, V>) {
        let kept = state.slots.values().filter(|s| matches!(s, Slot::Done { .. })).count();
        if kept <= self.limit {
            return;
        }
        let oldest = state
            .slots
            .iter()
            .filter_map(|(k, s)| match s {
                Slot::Done { used, .. } => Some((k, *used)),
                Slot::Pending(_) => None,
            })
            .min_by_key(|(_, used)| *used)
            .map(|(k, _)| k.clone());
        if let Some(oldest) = oldest {
            state.slots.remove(&oldest);
        }
    }
}

/// The answers of the primitive queries, kept by [`Key`].
pub struct Answers(Memo<Key, Value>);

/// Answers kept at most.
pub const KEPT: usize = 10_000;

impl Answers {
    /// Keeps at most `limit` answers; 0 keeps none, so every question is asked.
    pub fn new(limit: usize) -> Self {
        Self(Memo::new(limit))
    }

    /// The answer under `key` and whether it was reused. Without a key, or if the
    /// answer cannot be kept, `ask` decides: its second value says whether the
    /// answer may be kept.
    pub async fn get<T: Reused, E>(
        &self,
        key: Option<&Key>,
        ask: impl Future<Output = Result<Computed<T>, E>>,
    ) -> Result<(T, bool), E> {
        let Some(key) = key else {
            return ask.await.map(|c| (c.value, false));
        };
        let asked = async {
            let Computed { value, keep } = ask.await?;
            Ok(Computed {
                value: value.kept(),
                keep,
            })
        };
        let (value, reused) = self.0.get(key, asked).await?;
        match T::from_kept(value) {
            Some(value) => Ok((value, reused)),
            None => unreachable!("a key names the type of its answer"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::task::{Context, Poll, Waker};

    use super::*;
    use crate::join::join;
    use crate::testutil::{block_on, yield_once};

    fn kept(value: u8) -> Result<Computed<u8>, ()> {
        Ok(Computed { value, keep: true })
    }

    #[test]
    fn a_value_is_computed_once_for_the_callers_that_ask_while_it_is() {
        let memo = Memo::new(4);
        let runs = Cell::new(0);
        let ask = || async {
            runs.set(runs.get() + 1);
            yield_once().await;
            kept(7)
        };
        let (a, b) = block_on(join(memo.get(&"k", ask()), memo.get(&"k", ask())));
        assert_eq!((a, b), (Ok((7, false)), Ok((7, true))));
        assert_eq!(runs.get(), 1);
        assert_eq!(block_on(memo.get(&"k", ask())), Ok((7, true)));
        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn a_value_that_may_not_be_kept_is_computed_again_by_everyone_who_asks() {
        let memo = Memo::new(4);
        let runs = Cell::new(0);
        let ask = || async {
            runs.set(runs.get() + 1);
            yield_once().await;
            Ok::<_, ()>(Computed {
                value: runs.get(),
                keep: false,
            })
        };
        let (a, b) = block_on(join(memo.get(&"k", ask()), memo.get(&"k", ask())));
        assert_eq!((a, b), (Ok((1, false)), Ok((2, false))));
        assert_eq!(block_on(memo.get(&"k", ask())), Ok((3, false)));
    }

    #[test]
    fn an_error_is_not_kept() {
        let memo = Memo::new(4);
        assert_eq!(
            block_on(memo.get(&"k", async { Err::<Computed<u8>, _>("no") })),
            Err("no")
        );
        assert_eq!(block_on(memo.get(&"k", async { kept(1) })), Ok((1, false)));
    }

    #[test]
    fn a_waiter_computes_the_value_when_the_caller_that_was_computing_it_gave_up() {
        let memo = Memo::new(4);
        let mut cx = Context::from_waker(Waker::noop());
        let mut computing = Box::pin(memo.get(&"k", async {
            std::future::pending::<()>().await;
            kept(1)
        }));
        let mut waiting = Box::pin(memo.get(&"k", async { kept(2) }));
        assert!(computing.as_mut().poll(&mut cx).is_pending());
        assert!(waiting.as_mut().poll(&mut cx).is_pending());
        drop(computing);
        match waiting.as_mut().poll(&mut cx) {
            Poll::Ready(done) => assert_eq!(done, Ok((2, false))),
            Poll::Pending => panic!("the waiter was not woken to compute"),
        }
    }

    #[test]
    fn the_least_recently_used_value_goes_beyond_the_limit() {
        let memo = Memo::new(2);
        let ask = |v| async move { kept(v) };
        for (key, v) in [("a", 1), ("b", 2)] {
            block_on(memo.get(&key, ask(v))).unwrap();
        }
        // `a` is used again, so `b` is the least recently used.
        assert_eq!(block_on(memo.get(&"a", ask(9))), Ok((1, true)));
        block_on(memo.get(&"c", ask(3))).unwrap();
        assert_eq!(block_on(memo.get(&"a", ask(9))), Ok((1, true)));
        assert_eq!(block_on(memo.get(&"b", ask(8))), Ok((8, false)));
    }

    #[test]
    fn nothing_is_kept_or_shared_with_a_limit_of_zero() {
        let memo = Memo::new(0);
        assert_eq!(block_on(memo.get(&"k", async { kept(1) })), Ok((1, false)));
        assert_eq!(block_on(memo.get(&"k", async { kept(2) })), Ok((2, false)));
    }
}
