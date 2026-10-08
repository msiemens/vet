//! The session's single writer and the state it publishes (design-concurrency §1.2).
//!
//! Review state is changed by commands, one at a time: each takes the write token, works on a copy
//! of the state it finds published, and publishes the copy when it is done. Readers take the
//! published pointer and never wait for a command, however long it runs. The published state is
//! only ever replaced by whoever holds the token, so a reader sees the state as one command left it
//! and never between two of its steps.

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, PoisonError};

use tokio::sync::MutexGuard;

use super::state::State;
use crate::changeset::Source;

/// The state readers see. The lock is held to clone or to swap a pointer and for nothing else, so
/// it is never held across an await.
pub(super) struct Published(std::sync::Mutex<Arc<State>>);

impl Published {
    pub(super) fn new(state: State) -> Self {
        Self(std::sync::Mutex::new(Arc::new(state)))
    }

    /// The state as the last command left it.
    pub(super) fn read(&self) -> Arc<State> {
        Arc::clone(&self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn publish(&self, state: &Arc<State>) {
        let mut published = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !Arc::ptr_eq(&published, state) {
            *published = Arc::clone(state);
        }
    }
}

/// What the write token guards: the review database's write connection, and the changesets that
/// were on view and are not now, by source. Only a command reaches them, and only through [`Held`].
pub(super) struct Writer<W> {
    writes: W,
    parked: HashMap<Source, Arc<State>>,
}

impl<W> Writer<W> {
    pub(super) fn new(writes: W) -> Self {
        Self {
            writes,
            parked: HashMap::new(),
        }
    }
}

/// A command's hold of the session: the write token and a copy of the state to change. Reading it
/// reads the state the command started from; changing it copies that state first, unless nobody
/// else has it. It is published by [`Held::publish`] or when the hold ends, whatever the command
/// made of it: a command that fails after it changed something leaves that behind, as it always did.
pub(super) struct Held<'a, W> {
    writer: MutexGuard<'a, Writer<W>>,
    state: Arc<State>,
    published: &'a Published,
}

impl<'a, W> Held<'a, W> {
    pub(super) fn new(writer: MutexGuard<'a, Writer<W>>, published: &'a Published) -> Self {
        Self {
            writer,
            state: published.read(),
            published,
        }
    }

    /// The review database's write connection, which only a command that holds the token has.
    pub(super) fn writes(&self) -> W
    where
        W: Clone,
    {
        self.writer.writes.clone()
    }

    /// Makes the state readers see the one this command has now. A command that announces what it
    /// did (a new revision) publishes first, so that whoever hears of it finds it.
    pub(super) fn publish(&self) {
        self.published.publish(&self.state);
    }

    pub(super) fn parked(&mut self) -> &mut HashMap<Source, Arc<State>> {
        &mut self.writer.parked
    }

    /// Puts `state` in place of the one this command works on, and answers that one.
    pub(super) fn replace(&mut self, state: Arc<State>) -> Arc<State> {
        std::mem::replace(&mut self.state, state)
    }
}

impl<W> Deref for Held<'_, W> {
    type Target = State;

    fn deref(&self) -> &State {
        &self.state
    }
}

impl<W> DerefMut for Held<'_, W> {
    fn deref_mut(&mut self) -> &mut State {
        Arc::make_mut(&mut self.state)
    }
}

impl<W> Drop for Held<'_, W> {
    fn drop(&mut self) {
        self.publish();
    }
}
