//! A bound on how many things happen at once, handed out in the order they were asked
//! for. The pool gives each server one, so the requests in flight on it stay few
//! however many callers ask. Like the engine it needs no runtime: a waiter is woken by
//! the permit that is given back.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

pub struct Gate {
    state: Mutex<State>,
}

struct State {
    free: usize,
    next: u64,
    queue: VecDeque<Waiting>,
}

struct Waiting {
    id: u64,
    waker: Waker,
}

impl State {
    /// The waker of the caller next in line, if units are free for it.
    fn front(&self) -> Option<Waker> {
        if self.free == 0 {
            return None;
        }
        self.queue.front().map(|w| w.waker.clone())
    }
}

/// One unit of a gate, given back when dropped.
#[must_use]
pub struct Permit(Arc<Gate>);

impl Gate {
    /// A gate that lets `limit` permits out at once.
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                free: limit,
                next: 0,
                queue: VecDeque::new(),
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        crate::mutex::lock(&self.state)
    }

    /// A permit, once all those asked for before are given out and one is free.
    pub fn acquire(self: &Arc<Self>) -> Acquire {
        Acquire {
            gate: self.clone(),
            waiting: None,
        }
    }
}

pub struct Acquire {
    gate: Arc<Gate>,
    /// The place in line, once queued.
    waiting: Option<u64>,
}

impl Future for Acquire {
    type Output = Permit;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Permit> {
        let gate = self.gate.clone();
        let mut state = gate.lock();
        match self.waiting {
            None if state.free > 0 && state.queue.is_empty() => {
                state.free -= 1;
                Poll::Ready(Permit(gate.clone()))
            }
            None => {
                let id = state.next;
                state.next += 1;
                state.queue.push_back(Waiting {
                    id,
                    waker: cx.waker().clone(),
                });
                self.waiting = Some(id);
                Poll::Pending
            }
            Some(id) if state.free > 0 && state.queue.front().is_some_and(|w| w.id == id) => {
                state.queue.pop_front();
                state.free -= 1;
                self.waiting = None;
                // The next in line may find a unit free as well.
                let next = state.front();
                drop(state);
                if let Some(next) = next {
                    next.wake();
                }
                Poll::Ready(Permit(gate.clone()))
            }
            Some(id) => {
                if let Some(place) = state.queue.iter_mut().find(|w| w.id == id) {
                    place.waker.clone_from(cx.waker());
                }
                Poll::Pending
            }
        }
    }
}

impl Drop for Acquire {
    /// A caller that gives up leaves the line, and the one behind it moves up.
    fn drop(&mut self) {
        let Some(id) = self.waiting else { return };
        let next = {
            let mut state = self.gate.lock();
            state.queue.retain(|w| w.id != id);
            state.front()
        };
        if let Some(next) = next {
            next.wake();
        }
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let next = {
            let mut state = self.0.lock();
            state.free += 1;
            state.front()
        };
        if let Some(next) = next {
            next.wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::join::bounded;
    use crate::testutil::{block_on, yield_once};

    #[test]
    fn no_more_permits_are_out_than_the_gate_allows_whoever_asks() {
        let gate = Gate::new(3);
        let (out, peak) = (Cell::new(0), Cell::new(0));
        block_on(bounded(20, 0..20, |n| {
            let (gate, out, peak) = (gate.clone(), &out, &peak);
            async move {
                let _permit = gate.acquire().await;
                out.set(out.get() + 1);
                peak.set(peak.get().max(out.get()));
                for _ in 0..(n % 4) {
                    yield_once().await;
                }
                out.set(out.get() - 1);
            }
        }));
        assert_eq!(peak.get(), 3);
        assert_eq!(out.get(), 0);
    }

    #[test]
    fn permits_go_to_callers_in_the_order_they_asked() {
        let gate = Gate::new(1);
        let held = block_on(gate.acquire());
        let order = Cell::new(Vec::new());
        let ask = |tag: u8| {
            let gate = gate.clone();
            let order = &order;
            async move {
                let _permit = gate.acquire().await;
                let mut seen = order.take();
                seen.push(tag);
                order.set(seen);
                yield_once().await;
            }
        };
        let mut waiting = Vec::new();
        let mut cx = Context::from_waker(Waker::noop());
        for tag in 0..4 {
            let mut asking = Box::pin(ask(tag));
            assert!(asking.as_mut().poll(&mut cx).is_pending());
            waiting.push(asking);
        }
        drop(held);
        while !waiting.is_empty() {
            waiting.retain_mut(|asking| asking.as_mut().poll(&mut cx).is_pending());
        }
        assert_eq!(order.take(), [0, 1, 2, 3]);
    }

    #[test]
    fn a_caller_that_gives_up_does_not_hold_up_those_behind_it() {
        let gate = Gate::new(1);
        let held = block_on(gate.acquire());
        let mut cx = Context::from_waker(Waker::noop());
        let mut first = Box::pin(gate.acquire());
        let mut second = Box::pin(gate.acquire());
        assert!(first.as_mut().poll(&mut cx).is_pending());
        assert!(second.as_mut().poll(&mut cx).is_pending());
        drop(held);
        drop(first);
        assert!(second.as_mut().poll(&mut cx).is_ready(), "the second moved up");
    }
}
