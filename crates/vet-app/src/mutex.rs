use std::sync::{Mutex, MutexGuard, PoisonError};

/// The guard of `mutex`, whether or not a thread panicked while holding it: the state behind
/// these locks is a cache, a list or a handle, never left half-written across a panic.
#[allow(clippy::disallowed_methods, reason = "the one place that locks a std mutex")]
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
