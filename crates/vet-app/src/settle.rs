//! Settling (§6.2) as a pure state machine over an injected clock reading.
//! After a write burst, a snapshot settles once the debounce window passes with
//! no further changes. The caller reconciles before declaring it settled.

use std::collections::BTreeSet;

use vet_core::content::RepoPath;

use crate::ports::Millis;

pub struct Settler {
    debounce_ms: u64,
    dirty: BTreeSet<RepoPath>,
    last_change: Option<Millis>,
}

impl Settler {
    pub fn new(debounce_ms: u64) -> Self {
        Self { debounce_ms, dirty: BTreeSet::new(), last_change: None }
    }

    pub fn record(&mut self, path: RepoPath, now: Millis) {
        self.dirty.insert(path);
        self.last_change = Some(now);
    }

    /// When the caller should next call [`Settler::take_settled`], if anything is pending.
    pub fn deadline(&self) -> Option<Millis> {
        self.last_change.map(|at| Millis(at.0 + self.debounce_ms))
    }

    /// The touched paths, once the debounce window has passed with no changes.
    pub fn take_settled(&mut self, now: Millis) -> Option<BTreeSet<RepoPath>> {
        if now < self.deadline()? {
            return None;
        }
        self.last_change = None;
        Some(std::mem::take(&mut self.dirty))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> RepoPath {
        RepoPath(name.as_bytes().to_vec())
    }

    #[test]
    fn a_burst_settles_once_after_the_window() {
        let mut settler = Settler::new(1500);
        settler.record(path("a.rs"), Millis(0));
        settler.record(path("b.rs"), Millis(1000));
        assert_eq!(settler.take_settled(Millis(2000)), None, "b.rs touched 1000 ms ago");
        assert_eq!(settler.deadline(), Some(Millis(2500)));
        let settled = settler.take_settled(Millis(2500)).unwrap();
        assert_eq!(settled.len(), 2);
        assert_eq!(settler.take_settled(Millis(9999)), None, "nothing pending");
    }
}
