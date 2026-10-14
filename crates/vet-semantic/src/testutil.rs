//! Test-only executor: the fakes never block, so a no-op waker is enough.

use std::future::poll_fn;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use vet_core::content::Hash;

use crate::ports::{Answered, BackendId, Provenance, ServerIdentity};
use crate::query::Asked;

/// An answer from a fake server called "fake".
pub fn answered<T>(value: T) -> Asked<T> {
    Ok(Answered {
        value,
        provenance: Provenance {
            server: ServerIdentity {
                backend: BackendId("fake".into()),
                fingerprint: Hash([0; 32]),
                label: "fake".into(),
            },
            from_cache: false,
        },
        complete: true,
    })
}

pub fn block_on<T>(fut: impl Future<Output = T>) -> T {
    let mut fut = pin!(fut);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

/// Pending once, so another future gets polled in between.
pub async fn yield_once() {
    let mut yielded = false;
    poll_fn(|cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;
}
