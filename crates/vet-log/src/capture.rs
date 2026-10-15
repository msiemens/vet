//! A subscriber that keeps what is logged in memory, for tests that check what a log line says
//! or leaves out.

use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tracing::subscriber::DefaultGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

/// What was logged, formatted as in the log file, spans included.
#[derive(Clone, Default)]
pub struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    /// Logs here, filtered by `filter`, on this thread until the guard drops. A test on a
    /// current-thread runtime captures what its futures log.
    pub fn install(&self, filter: EnvFilter) -> DefaultGuard {
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(self.clone())
            .finish();
        tracing::subscriber::set_default(subscriber)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes()).into_owned()
    }

    /// The buffer, whether or not a thread panicked while holding it.
    #[allow(clippy::disallowed_methods, reason = "the one place that locks this std mutex")]
    fn bytes(&self) -> MutexGuard<'_, Vec<u8>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
