//! The tree is a process group: the run leads its own, and descendants stay in it
//! unless they start a session of their own.

use std::io;

use rustix::process::{Pid, Signal, kill_process_group};
use tokio::process::{Child, Command};

pub(crate) fn prepare(command: &mut Command) {
    command.process_group(0);
}

pub(crate) struct Tree {
    pgid: Pid,
}

impl Tree {
    pub(crate) fn attach(child: &Child) -> io::Result<Tree> {
        let pgid = (child.id().and_then(|id| i32::try_from(id).ok()).and_then(Pid::from_raw))
            .ok_or_else(|| io::Error::other("the process already exited"))?;
        Ok(Tree { pgid })
    }

    /// Asks the tree to end (SIGTERM).
    pub(crate) fn terminate(&self) {
        let _ = kill_process_group(self.pgid, Signal::TERM);
    }

    /// Ends the tree without asking (SIGKILL); a no-op once it is gone.
    pub(crate) fn kill(&self) {
        let _ = kill_process_group(self.pgid, Signal::KILL);
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        self.kill();
    }
}
