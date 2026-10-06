use std::time::Duration;

use tokio::time::Instant;
use vet_app::ports::{Clock, Millis};

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
