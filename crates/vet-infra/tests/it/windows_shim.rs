//! A `.cmd` shim (how npm installs `typescript-language-server`) is started directly, and
//! what looks like shell syntax in an argument stays an argument.
#![cfg(windows)]

use vet_infra::exec::command;

#[tokio::test]
async fn a_cmd_shim_gets_an_argument_with_shell_syntax_as_one_argument() {
    let dir = tempfile::tempdir().unwrap();
    let shim = dir.path().join("show-first.cmd");
    std::fs::write(&shim, "@echo off\r\necho %~1\r\n").unwrap();

    let out = command(shim.to_str().unwrap(), &["a&echo injected".to_owned()], None, &[])
        .output()
        .await
        .unwrap();

    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "a&echo injected");
}
