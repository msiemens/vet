//! The PATH of the user's login shell. An app started from the Finder or a launcher gets the
//! service manager's minimal PATH, which lacks the tools the backend runs (`gh`, `rustup`, language
//! servers); a terminal has the PATH the user's profile builds.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// How long the shell may take to print its PATH; profiles that wait on the network are cut off.
pub const TIMEOUT: Duration = Duration::from_secs(5);

const START: &str = "__VET_PATH_START__";
const END: &str = "__VET_PATH_END__";

/// The shell to ask: `$SHELL`, else the platform's default.
pub fn default_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                "/bin/zsh"
            } else {
                "/bin/sh"
            }
            .into()
        })
}

/// Runs `shell` as an interactive login shell and returns the PATH it prints. Profile files may print
/// anything, so the PATH is taken from between markers. Kills the shell after `timeout`.
#[allow(
    clippy::disallowed_methods,
    reason = "the output is read on a thread of its own, so the wait can time out"
)]
pub fn from_login_shell(shell: &str, timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(shell)
        .args(["-l", "-i", "-c", &format!(r#"printf '%s' "{START}${{PATH}}{END}""#)])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot start {shell}: {error}"))?;
    let mut stdout = child.stdout.take().ok_or("the shell has no stdout")?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let read = stdout.read_to_end(&mut output).map(|_| output);
        let _ = sender.send(read);
    });
    let read = match receiver.recv_timeout(timeout) {
        Ok(read) => read.map_err(|error| format!("cannot read the output of {shell}: {error}")),
        Err(_) => Err(format!("{shell} did not print its PATH within {timeout:?}")),
    };
    let output = read.inspect_err(|_| {
        let _ = child.kill();
        let _ = child.wait();
    })?;
    let status = child
        .wait()
        .map_err(|error| format!("cannot wait for {shell}: {error}"))?;
    if !status.success() {
        return Err(format!("{shell} exited with {status}"));
    }
    parse(&String::from_utf8_lossy(&output)).ok_or_else(|| format!("{shell} printed no PATH"))
}

fn parse(output: &str) -> Option<String> {
    let start = output.find(START)? + START.len();
    let len = output[start..].find(END)?;
    let path = &output[start..start + len];
    (!path.is_empty()).then(|| path.to_owned())
}
