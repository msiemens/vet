//! Review (§5): owns the `Session`, which holds a `PinId` from Acquisition rather
//! than the pin itself. Remapping runs per changed file, triggered when either
//! the base or the target blob id changes (the base case is `SeenAgainstOldBase`).

use vet_core::content::{Changeset, ReviewFilter};
use vet_core::review::{ReviewedSpan, Thread};

use crate::acquisition::PinId;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SessionId(pub u64);

pub struct Session {
    pub id: SessionId,
    pub pin: PinId,
    pub changeset: Changeset,
    pub filter: ReviewFilter,
    pub spans: Vec<ReviewedSpan>,
    pub threads: Vec<Thread>,
}
