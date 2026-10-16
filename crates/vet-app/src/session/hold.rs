//! The session's single writer and the state it publishes.
//!
//! Review state is changed by commands, one at a time: each takes the write token, works on a copy
//! of the state it finds published, and publishes the copy when it is done. Readers take the
//! published pointer and never wait for a command, however long it runs. The published state is
//! only ever replaced by whoever holds the token, so a reader sees the state as one command left it
//! and never between two of its steps.

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use tokio::sync::MutexGuard;

use crate::mutex::lock;

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
        Arc::clone(&lock(&self.0))
    }

    fn publish(&self, state: &Arc<State>) {
        let mut published = lock(&self.0);
        if !Arc::ptr_eq(&published, state) {
            *published = Arc::clone(state);
        }
    }
}

/// What the write token guards: the review database's write connection, and the one changeset that
/// is kept while it is not on view. Only a command reaches them, and only through [`Held`].
pub(super) struct Writer<W> {
    writes: W,
    parked: Parked,
    /// The project's change counter. Every change to a state through [`Held`] takes the next
    /// value and stamps the state with it, so that what is read from a state says which changes
    /// it reflects, whichever changeset was on view.
    seq: u64,
}

impl<W> Writer<W> {
    pub(super) fn new(writes: W) -> Self {
        Self {
            writes,
            parked: Parked::new(),
            seq: 0,
        }
    }
}

/// The changeset kept while it is not on view: the working tree, whose settle loop needs its state
/// whenever the tree is not shown. A branch or pull request is rebuilt from the store by `enter`
/// when it comes back, so it is not kept here.
pub(super) struct Parked(Option<Arc<State>>);

impl Parked {
    pub(super) fn new() -> Self {
        Self(None)
    }

    /// The working tree kept off view.
    pub(super) fn tree(&self) -> Option<&Arc<State>> {
        self.0.as_ref()
    }

    /// The working tree kept off view, to change in place.
    pub(super) fn tree_mut(&mut self) -> Option<&mut Arc<State>> {
        self.0.as_mut()
    }

    /// Takes the working tree kept off view.
    pub(super) fn take(&mut self) -> Option<Arc<State>> {
        self.0.take()
    }

    /// Keeps the working tree off view.
    pub(super) fn keep(&mut self, tree: Arc<State>) {
        debug_assert_eq!(tree.source, Source::WorkingTree);
        self.0 = Some(tree);
    }

    /// Whether the working tree kept off view has an open review, which keeps it settling.
    pub(super) fn review_open(&self) -> bool {
        self.0.as_ref().is_some_and(|tree| tree.review.is_some())
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

    /// The state this command works on, shared as it is.
    pub(super) fn shared(&self) -> Arc<State> {
        Arc::clone(&self.state)
    }

    pub(super) fn parked(&mut self) -> &mut Parked {
        &mut self.writer.parked
    }

    /// Puts `state` in place of the one this command works on, and answers that one.
    pub(super) fn replace(&mut self, mut state: Arc<State>) -> Arc<State> {
        self.writer.seq += 1;
        Arc::make_mut(&mut state).seq = self.writer.seq;
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
        self.writer.seq += 1;
        let state = Arc::make_mut(&mut self.state);
        state.seq = self.writer.seq;
        state
    }
}

impl<W> Drop for Held<'_, W> {
    fn drop(&mut self) {
        self.publish();
    }
}
