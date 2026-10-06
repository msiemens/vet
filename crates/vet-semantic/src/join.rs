//! Polls two futures concurrently on the caller's task, so base and target
//! queries run in parallel without this crate depending on an async runtime.

use std::future::poll_fn;
use std::pin::pin;
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::testutil::{block_on, yield_once};

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
