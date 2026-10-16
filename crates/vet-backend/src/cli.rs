//! Where the platforms' CLIs (`gh`, `az`) are looked for.

use std::path::PathBuf;

use vet_app::ports::{Command, ProcessRunner};
use vet_infra::exec::{Host, Process, TokioRunner, is_file};

/// Runs the CLIs of the accounts. Without `dirs` they are found on the process's `PATH`; with
/// them, only in those directories, so what is installed on the machine does not matter.
pub(crate) struct CliRunner {
    pub dirs: Option<Vec<PathBuf>>,
}

impl ProcessRunner for CliRunner {
    type Process = Process;

    fn spawn(&self, command: &Command) -> Result<Process, String> {
        let Some(dirs) = &self.dirs else {
            return TokioRunner.spawn(command);
        };
        let host = Host {
            path: dirs.clone(),
            ..Host::current()
        };
        let found = host
            .find(&command.program, is_file)
            .ok_or_else(|| format!("{}: not found", command.program))?;
        TokioRunner.spawn(&Command {
            program: found.to_string_lossy().into_owned(),
            args: command.args.clone(),
            cwd: command.cwd.clone(),
            env: command.env.clone(),
        })
    }
}
