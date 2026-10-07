use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::Instant;
use vet_app::ports::{Clock, Millis};

/// Whole unix seconds of `t`; negative before the epoch.
pub fn unix_secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// Unix seconds now.
pub fn unix_now() -> i64 {
    unix_secs(SystemTime::now())
}

pub struct TokioClock {
    start: Instant,
}

impl TokioClock {
    pub fn new() -> Self {
        Self { start: Instant::now() }
    }
}

impl Default for TokioClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for TokioClock {
    fn now(&self) -> Millis {
        Millis(self.start.elapsed().as_millis() as u64)
    }

    fn sleep_until(&self, deadline: Millis) -> impl Future<Output = ()> + Send {
        tokio::time::sleep_until(self.start + Duration::from_millis(deadline.0))
    }
}
