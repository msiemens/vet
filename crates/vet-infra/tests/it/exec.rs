//! A run's whole process tree ends with it: on Stop, and when the leader exits.
#![cfg(unix)]
// The test spawns a child of its own to have a zombie, which nothing else does.
#![allow(clippy::disallowed_methods)]

use std::process::Command as StdCommand;
use std::time::{Duration, Instant};

use rustix::io::Errno;
use rustix::process::{Pid, WaitId, WaitIdOptions, WaitOptions, test_kill_process, waitid, waitpid};
use vet_app::ports::{Command, ProcessEvent, ProcessRunner, RunningProcess};
use vet_infra::exec::{Process, TokioRunner};

fn sh(script: &str) -> Command {
    Command {
        program: "sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: std::env::temp_dir(),
        env: vec![],
    }
}

/// Reads the grandchild's pid, which every script prints first.
async fn grandchild(process: &mut Process) -> Pid {
    let mut out = Vec::new();
    while !out.contains(&b'\n') {
        match process.next().await {
            ProcessEvent::Stdout(bytes) => out.extend(bytes),
            ProcessEvent::Stderr(_) => {}
            ProcessEvent::Exit(code) => panic!("exited with {code:?} before printing the pid"),
        }
    }
    let pid: i32 = String::from_utf8(out).unwrap().trim().parse().unwrap();
    Pid::from_raw(pid).unwrap()
}

async fn until_exit(process: &mut Process) -> ProcessEvent {
    loop {
        match process.next().await {
            ProcessEvent::Stdout(_) | ProcessEvent::Stderr(_) => {}
            exit => return exit,
        }
    }
}

/// Waits until `pid` is gone. A process that exited but was never reaped (a zombie) is gone: it
/// still answers a liveness probe, so the process table is asked instead of `kill`.
async fn assert_gone(pid: Pid) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !gone(pid) {
        assert!(Instant::now() < deadline, "process {pid:?} is still running");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Whether `pid` is gone: reaped, or exited and only waiting to be reaped.
fn gone(pid: Pid) -> bool {
    // Our own child: reaping it is the surest sign, and it is what a zombie is waiting for.
    match waitpid(Some(pid), WaitOptions::NOHANG) {
        Ok(Some(_)) => return true,
        Ok(None) => return false,
        // Not our child: a leader's children are reparented, and only their new parent reaps them.
        Err(_) => {}
    }
    if is_zombie(pid) {
        return true;
    }
    matches!(test_kill_process(pid), Err(Errno::SRCH))
}

/// Whether `pid` has exited but was not reaped yet, as the process table has it. Nothing portable
/// says so; on Linux the state is read, and elsewhere reaping is prompt enough that the probe is.
#[cfg(target_os = "linux")]
fn is_zombie(pid: Pid) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{}/stat", pid.as_raw_nonzero())) else {
        return false;
    };
    // The state follows the parenthesised name, which may hold spaces of its own.
    stat.rsplit_once(')')
        .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
}

#[cfg(not(target_os = "linux"))]
fn is_zombie(_: Pid) -> bool {
    false
}

#[tokio::test]
async fn a_child_that_exited_without_being_reaped_is_gone() {
    // A liveness probe alone cannot tell this child from a running one, which is the whole point:
    // nothing reaps it, and `assert_gone` must still answer.
    let child = StdCommand::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
    let pid = Pid::from_raw(i32::try_from(child.id()).unwrap()).unwrap();
    loop {
        // WNOWAIT observes the exit without reaping it, so the child is a zombie from here on.
        match waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        ) {
            Ok(Some(_)) => break,
            Ok(None) => tokio::time::sleep(Duration::from_millis(5)).await,
            Err(error) => panic!("waitid: {error}"),
        }
    }
    assert!(
        test_kill_process(pid).is_ok(),
        "a zombie answers a liveness probe; the test is only worth something if it does"
    );
    tokio::time::timeout(Duration::from_secs(2), assert_gone(pid))
        .await
        .expect("an exited child is gone without waiting out the grace");
    drop(child);
}

#[tokio::test]
async fn stop_ends_the_grandchildren_too() {
    let mut process = TokioRunner.spawn(&sh("sleep 100 & echo $!; sleep 100")).unwrap();
    let grandchild = grandchild(&mut process).await;
    assert!(test_kill_process(grandchild).is_ok());
    process.kill();
    assert_eq!(until_exit(&mut process).await, ProcessEvent::Exit(None));
    assert_gone(grandchild).await;
}

#[tokio::test]
async fn stop_kills_a_tree_that_ignores_sigterm() {
    let mut process = TokioRunner
        .spawn(&sh("trap '' TERM; sleep 100 & echo $!; sleep 100"))
        .unwrap();
    let grandchild = grandchild(&mut process).await;
    process.kill();
    until_exit(&mut process).await;
    assert_gone(grandchild).await;
}

#[tokio::test]
async fn a_run_that_exits_takes_its_leftovers_with_it() {
    let mut process = TokioRunner.spawn(&sh("sleep 100 >/dev/null 2>&1 & echo $!")).unwrap();
    let grandchild = grandchild(&mut process).await;
    assert_eq!(until_exit(&mut process).await, ProcessEvent::Exit(Some(0)));
    assert_gone(grandchild).await;
}
