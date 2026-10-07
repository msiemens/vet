//! A value one caller computes while others wait for it. Runtime-free: waiters
//! are woken through their wakers.
//!
//! The computing side holds a [`Producer`]. If it is dropped before it
//! finished, because the caller gave up, the waiters are woken with `None`
//! and decide for themselves: one of them computes the value next. Nobody is
//! failed by someone else's cancellation.

use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

pub struct Shared<T> {
    state: Mutex<State<T>>,
}

enum State<T> {
    Pending(Vec<Waker>),
    /// `None`: the producer was dropped without a value.
    Settled(Option<T>),
}

/// The right to fill a [`Shared`]. Dropping it unfinished abandons the value.
pub struct Producer<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Shared<T> {
    /// A cell nobody has filled, and the handle that fills it.
    pub fn start() -> (Arc<Self>, Producer<T>) {
        let shared = Arc::new(Self {
            state: Mutex::new(State::Pending(Vec::new())),
        });
        let producer = Producer { shared: shared.clone() };
        (shared, producer)
    }

    fn settle(&self, value: Option<T>) {
        let wakers = {
            let mut state = self.state.lock().expect("not poisoned");
            match &mut *state {
                State::Settled(_) => return,
                State::Pending(wakers) => {
                    let wakers = std::mem::take(wakers);
                    *state = State::Settled(value);
                    wakers
                }
            }
        };
        wakers.into_iter().for_each(Waker::wake);
    }
}

impl<T: Clone> Shared<T> {
    /// The value once the producer finished, or `None` if it gave up.
    pub async fn wait(&self) -> Option<T> {
        std::future::poll_fn(|cx| {
            let mut state = self.state.lock().expect("not poisoned");
            match &mut *state {
                State::Settled(value) => Poll::Ready(value.clone()),
                State::Pending(wakers) => {
                    if !wakers.iter().any(|w| w.will_wake(cx.waker())) {
                        wakers.push(cx.waker().clone());
                    }
                    Poll::Pending
                }
            }
        })
        .await
    }
}

impl<T> Producer<T> {
    pub fn finish(self, value: T) {
        self.shared.settle(Some(value));
    }
}

impl<T> Drop for Producer<T> {
    fn drop(&mut self) {
        self.shared.settle(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::join::join;
    use crate::testutil::{block_on, yield_once};

    #[test]
    fn every_waiter_gets_the_finished_value() {
        let (shared, producer) = Shared::start();
        let finish = async {
            yield_once().await;
            producer.finish(7);
        };
        let ((a, b), ()) = block_on(join(join(shared.wait(), shared.wait()), finish));
        assert_eq!((a, b), (Some(7), Some(7)));
        assert_eq!(block_on(shared.wait()), Some(7));
    }

    #[test]
    fn waiters_of_an_abandoned_value_are_woken_with_none() {
        let (shared, producer) = Shared::<u8>::start();
        let (waited, ()) = block_on(join(shared.wait(), async {
            yield_once().await;
            drop(producer);
        }));
        assert_eq!(waited, None);
        assert_eq!(block_on(shared.wait()), None);
    }
}
