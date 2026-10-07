//! Polls futures concurrently on the caller's task, so base and target
//! queries, or the queries of several files, run in parallel without this
//! crate depending on an async runtime.

use std::future::poll_fn;
use std::pin::{Pin, pin};
use std::task::Poll;

pub async fn join<A: Future, B: Future>(a: A, b: B) -> (A::Output, B::Output) {
    let mut a = pin!(a);
    let mut b = pin!(b);
    let (mut out_a, mut out_b) = (None, None);
    poll_fn(|cx| {
        if out_a.is_none()
            && let Poll::Ready(v) = a.as_mut().poll(cx)
        {
            out_a = Some(v);
        }
        if out_b.is_none()
            && let Poll::Ready(v) = b.as_mut().poll(cx)
        {
            out_b = Some(v);
        }
        if out_a.is_some() && out_b.is_some() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
    (
        out_a.expect("polled to completion"),
        out_b.expect("polled to completion"),
    )
}

/// Which of two raced futures finished first.
#[derive(PartialEq, Eq, Debug)]
pub enum Either<A, B> {
    Left(A),
    Right(B),
}

/// Polls both futures on the caller's task until one finishes, and drops the
/// other, which stops whatever it was waiting for. If both are ready at once,
/// `a` wins.
pub async fn race<A: Future, B: Future>(a: A, b: B) -> Either<A::Output, B::Output> {
    let mut a = pin!(a);
    let mut b = pin!(b);
    poll_fn(|cx| {
        if let Poll::Ready(v) = a.as_mut().poll(cx) {
            return Poll::Ready(Either::Left(v));
        }
        if let Poll::Ready(v) = b.as_mut().poll(cx) {
            return Poll::Ready(Either::Right(v));
        }
        Poll::Pending
    })
    .await
}

/// Runs `f` over `items` with at most `limit` futures in flight, polled on the
/// caller's task. The outputs come back in the order of `items`.
pub async fn bounded<T, F: Future>(
    limit: usize,
    items: impl IntoIterator<Item = T>,
    mut f: impl FnMut(T) -> F,
) -> Vec<F::Output> {
    let mut waiting = items.into_iter().enumerate();
    let mut running: Vec<(usize, Pin<Box<F>>)> = Vec::new();
    let mut done: Vec<(usize, F::Output)> = Vec::new();
    poll_fn(|cx| {
        loop {
            while running.len() < limit
                && let Some((index, item)) = waiting.next()
            {
                running.push((index, Box::pin(f(item))));
            }
            if running.is_empty() {
                return Poll::Ready(());
            }
            let before = running.len();
            let mut at = 0;
            while at < running.len() {
                if let Poll::Ready(out) = running[at].1.as_mut().poll(cx) {
                    let (index, _) = running.swap_remove(at);
                    done.push((index, out));
                } else {
                    at += 1;
                }
            }
            if running.len() == before {
                return Poll::Pending;
            }
        }
    })
    .await;
    done.sort_by_key(|(index, _)| *index);
    done.into_iter().map(|(_, out)| out).collect()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::testutil::{block_on, yield_once};

    #[test]
    fn bounded_keeps_the_limit_in_flight_and_the_order_of_the_items() {
        let running = Cell::new(0);
        let peak = Cell::new(0);
        let out = block_on(bounded(3, 0..8, |n| {
            let (running, peak) = (&running, &peak);
            async move {
                running.set(running.get() + 1);
                peak.set(peak.get().max(running.get()));
                // The first items take the longest, so they finish last.
                for _ in 0..(8 - n) {
                    yield_once().await;
                }
                running.set(running.get() - 1);
                n * 10
            }
        }));
        assert_eq!(out, [0, 10, 20, 30, 40, 50, 60, 70]);
        assert_eq!(peak.get(), 3);
    }

    #[test]
    fn bounded_over_nothing_is_empty() {
        let out = block_on(bounded(3, Vec::<u8>::new(), |n| async move { n }));
        assert!(out.is_empty());
    }

    /// Sets a flag when dropped.
    struct Dropped<'a>(&'a Cell<bool>);

    impl Drop for Dropped<'_> {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    #[test]
    fn the_first_to_finish_wins_and_the_other_is_dropped_unfinished() {
        let dropped = Cell::new(false);
        let slow = async {
            let _guard = Dropped(&dropped);
            loop {
                yield_once().await;
            }
        };
        let quick = async {
            yield_once().await;
            "quick"
        };
        assert_eq!(block_on(race(slow, quick)), Either::Right("quick"));
        assert!(dropped.get());
    }

    #[test]
    fn a_future_that_is_ready_at_once_wins_over_one_that_is_too() {
        assert_eq!(block_on(race(async { 1 }, async { 2 })), Either::Left(1));
    }

    #[test]
    fn both_futures_make_progress_before_either_finishes() {
        let order = Cell::new(Vec::new());
        let log = |tag| {
            let mut v = order.take();
            v.push(tag);
            order.set(v);
        };
        let (x, y) = block_on(join(
            async {
                log("a1");
                yield_once().await;
                log("a2");
                1
            },
            async {
                log("b1");
                yield_once().await;
                log("b2");
                2
            },
        ));
        assert_eq!((x, y), (1, 2));
        assert_eq!(order.take(), ["a1", "b1", "a2", "b2"]);
    }
}
