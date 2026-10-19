use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use vet_host::login_path::from_login_shell;

const TIMEOUT: Duration = Duration::from_secs(3);

/// A script standing in for the login shell: it runs the `-c` command after `body`.
fn fake_shell(dir: &tempfile::TempDir, body: &str) -> String {
    let path = dir.path().join("shell");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn the_path_is_taken_from_between_the_markers_despite_noise() {
    let dir = tempfile::tempdir().unwrap();
    let shell = fake_shell(
        &dir,
        r#"echo "Welcome back"
PATH=/opt/fake/bin:/usr/bin
eval "$4"
echo "bye""#,
    );
    assert_eq!(from_login_shell(&shell, TIMEOUT).unwrap(), "/opt/fake/bin:/usr/bin");
}

#[test]
fn a_shell_that_hangs_is_killed_at_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let shell = fake_shell(&dir, "sleep 30");
    let started = Instant::now();
    assert!(from_login_shell(&shell, Duration::from_millis(300)).is_err());
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_failing_shell_gives_no_path() {
    let dir = tempfile::tempdir().unwrap();
    let shell = fake_shell(&dir, r#"PATH=/opt/fake/bin; eval "$4"; exit 1"#);
    assert!(from_login_shell(&shell, TIMEOUT).is_err());
}

#[test]
fn output_without_a_path_gives_none() {
    let dir = tempfile::tempdir().unwrap();
    let shell = fake_shell(&dir, "echo only noise");
    assert!(from_login_shell(&shell, TIMEOUT).is_err());
}
