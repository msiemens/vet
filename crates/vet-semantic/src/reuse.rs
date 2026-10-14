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
//! neither is an error. The answers that persist ([`Key::persists`]) are also
//! stored behind a [`SemanticCache`], and read from it before a server is asked.

use std::pin::Pin;
use std::sync::Arc;

use vet_core::content::{BlobId, Hash as Fingerprint, RepoPath, SnapshotId};
use vet_engine::{self as engine, Engine, Kind, Limits, Retention};

use crate::policy::AnalysisPolicy;
use crate::ports::{At, Diagnostic, Location, QueryError, ServerIdentity, SymbolInfo};

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
    Implementations(At),
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

/// An answer computed by a server, and whether the server was free of work of its own while it
/// gave it. Only a complete one is kept.
pub struct Computed<V> {
    pub value: V,
    pub complete: bool,
}

/// What the engine holds under a key: an answer and whether it was complete. Only complete ones
/// are kept, so what is read from memory or the store is complete; an incomplete one is held for
/// the callers that were waiting for it, and for none that come later.
#[derive(Clone, Debug)]
pub struct Held {
    value: Value,
    complete: bool,
}

/// What [`Answers::get`] yields.
pub struct Given<T> {
    pub value: T,
    /// An earlier answer was reused: no server was asked.
    pub from_cache: bool,
    pub complete: bool,
}

impl Key {
    /// Whether an answer under this key is kept between launches: the symbols of a
    /// text, and the answers about a tree whose restore was recorded, so that
    /// another launch can name the same dependencies. Workspace queries are
    /// unbounded in number and cheap to ask again.
    pub fn persists(&self) -> bool {
        match self {
            Key::Text { .. } => true,
            Key::Project { basis, query, .. } => {
                matches!(basis.restore, Restore::Recorded(_)) && !matches!(query, ProjectQuery::Workspace(_))
            }
        }
    }
}

/// Answers kept between launches, behind the in-memory ones (I2, §7.6). A stored
/// value is exactly what was admitted to memory under the same key. It is
/// disposable: whatever it cannot give is asked of a server.
pub trait SemanticCache: Send + Sync {
    /// The answer stored under `key`; none if there is none, or it cannot be read.
    fn get<'a>(&'a self, key: &'a Key) -> Pin<Box<dyn Future<Output = Option<Value>> + Send + 'a>>;

    /// Stores `value` under `key`, later: a query never waits for the disk.
    fn put(&self, key: &Key, value: &Value);
}

impl Kind for Key {
    type Value = Held;
    type Error = QueryError;

    fn retention(&self) -> Retention {
        Retention::Kept
    }
}

/// The answers of the primitive queries, kept by [`Key`]. A question is asked of
/// a server once however many callers ask while it is, and however many of them
/// give up: the others get its answer.
pub struct Answers {
    engine: Engine<Key>,
    store: Option<Arc<dyn SemanticCache>>,
}

/// Answers kept at most.
pub const KEPT: usize = 10_000;

impl Answers {
    /// Keeps at most `limit` answers; 0 keeps none, so every question is asked
    /// unless it is being asked already.
    pub fn new(limit: usize) -> Self {
        Self {
            engine: Engine::new(Limits::entries(limit)),
            store: None,
        }
    }

    /// Looks the answers that persist up in `store` before asking a server, and
    /// stores those a server gave.
    pub fn persisting(mut self, store: Option<Arc<dyn SemanticCache>>) -> Self {
        self.store = store;
        self
    }

    /// Drops the answers kept in memory. Those being computed are left to finish.
    pub fn forget(&self) {
        self.engine.clear();
    }

    /// The answer under `key` if one is kept, in memory or in the store; no server is
    /// asked and nothing is computed. An answer from the store is kept in memory too.
    pub async fn recall<T: Reused>(&self, key: &Key) -> Option<T> {
        if let Some(held) = self.engine.peek(key) {
            return T::from_kept(held.value);
        }
        let store = self.store.as_deref().filter(|_| key.persists())?;
        let value = store.get(key).await?;
        let recalled = T::from_kept(value.clone())?;
        self.engine.keep(key, Held { value, complete: true });
        Some(recalled)
    }

    /// The answer under `key`, whether it was reused and whether it is complete. Without a key,
    /// or if the answer is incomplete and so cannot be kept, `ask` decides. `ask` only builds
    /// the question, which is asked by whichever caller is polled while the key is in flight.
    pub async fn get<T, F>(
        &self,
        key: Option<&Key>,
        ask: impl FnOnce() -> F + Send + 'static,
    ) -> Result<Given<T>, QueryError>
    where
        T: Reused + Send,
        F: Future<Output = Result<Computed<T>, QueryError>> + Send + 'static,
    {
        let Some(key) = key else {
            return ask().await.map(|c| Given {
                value: c.value,
                from_cache: false,
                complete: c.complete,
            });
        };
        let store = self.store.clone().filter(|_| key.persists());
        let owned = key.clone();
        let got = self
            .engine
            .get(key.clone(), move |_| async move {
                if let Some(store) = &store
                    && let Some(found) = store.get(&owned).await.and_then(T::from_kept)
                {
                    let held = Held {
                        value: found.kept(),
                        complete: true,
                    };
                    return Ok(engine::Computed::restored(held));
                }
                let Computed { value, complete } = ask().await?;
                let value = value.kept();
                if let (true, Some(store)) = (complete, &store) {
                    store.put(&owned, &value);
                }
                Ok(engine::Computed::new(Held { value, complete }, complete))
            })
            .await?;
        match T::from_kept(got.value.value) {
            Some(value) => Ok(Given {
                value,
                from_cache: got.reused,
                complete: got.value.complete,
            }),
            None => unreachable!("a key names the type of its answer"),
        }
    }
}
