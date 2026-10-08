//! A run's whole process tree ends with it: on Stop, and when the leader exits.
#![cfg(unix)]

use std::time::{Duration, Instant};

use rustix::io::Errno;
use rustix::process::{Pid, test_kill_process};
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

async fn assert_gone(pid: Pid) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !matches!(test_kill_process(pid), Err(Errno::SRCH)) {
        assert!(Instant::now() < deadline, "process {pid:?} is still running");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
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
