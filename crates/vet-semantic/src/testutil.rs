//! Test-only executor: the fakes never block, so a no-op waker is enough.

use std::pin::pin;
use std::task::{Context, Poll, Waker};

pub fn block_on<T>(fut: impl Future<Output = T>) -> T {
    let mut fut = pin!(fut);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
    }
}
