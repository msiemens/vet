//! The external programs a restore runs, behind a trait so that the decisions
//! around them (what is kept, what runs again) can be exercised without installing any.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

use crate::budget::Budget;
use crate::exec::{Captured, capture};

pub(super) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// How long asking a tool for its version may take.
const VERSION_TIMEOUT: Duration = Duration::from_secs(20);

/// What a version probe returns for a tool that cannot be run.
pub(super) const UNAVAILABLE: &str = "unavailable";

pub(super) trait Tools: Send + Sync {
    /// The first line `program --version` prints.
    fn version<'a>(&'a self, program: &'a str) -> BoxFuture<'a, String>;

    fn run<'a>(
        &'a self,
        program: &'a str,
        args: &'a [String],
        cwd: &'a Path,
        env: &'a [(String, String)],
        timeout: Duration,
    ) -> BoxFuture<'a, Result<Captured, String>>;
}

/// The programs on this host's `PATH`. Version probes and restore steps each draw on a budget.
pub(super) struct Programs {
    pub probes: Budget,
    pub restores: Budget,
}

impl Tools for Programs {
    fn version<'a>(&'a self, program: &'a str) -> BoxFuture<'a, String> {
        Box::pin(async move {
            let args = ["--version".to_owned()];
            match capture(&self.probes, program, &args, None, &[], VERSION_TIMEOUT).await {
                Ok(out) if out.ok => out.stdout.lines().next().unwrap_or_default().trim().to_owned(),
                _ => UNAVAILABLE.to_owned(),
            }
        })
    }

    fn run<'a>(
        &'a self,
        program: &'a str,
        args: &'a [String],
        cwd: &'a Path,
        env: &'a [(String, String)],
        timeout: Duration,
    ) -> BoxFuture<'a, Result<Captured, String>> {
        Box::pin(capture(&self.restores, program, args, Some(cwd), env, timeout))
    }
}
