//! Fetching over smart HTTP, from a local `git http-backend` behind a minimal HTTP/1.1 server.

#![allow(clippy::disallowed_methods)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use tempfile::TempDir;
use vet_app::ports::{Fetcher, Via};
use vet_infra::git::GixRepo;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args([
            "-c",
            "user.name=Tess",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// Serves the repositories below `root` with `git http-backend` on a free local port, one thread
/// per connection, until the test ends. Answers with its base URL.
fn serve(root: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let root = root.clone();
            std::thread::spawn(move || while answer(&root, &stream).is_some() {});
        }
    });
    base
}

/// Reads one request from `stream` and answers it; `None` once the client closed the connection.
fn answer(root: &Path, stream: &TcpStream) -> Option<()> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).ok()? == 0 {
        return None;
    }
    let mut parts = request_line.split_whitespace();
    let (method, target) = (parts.next()?.to_owned(), parts.next()?.to_owned());
    let (mut length, mut chunked, mut content_type, mut protocol) = (0, false, String::new(), None);
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':')?;
        let value = value.trim().to_owned();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse().ok()?,
            "transfer-encoding" => chunked = value.eq_ignore_ascii_case("chunked"),
            "content-type" => content_type = value,
            "git-protocol" => protocol = Some(value),
            _ => {}
        }
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            let mut size = String::new();
            reader.read_line(&mut size).ok()?;
            let size = usize::from_str_radix(size.trim(), 16).ok()?;
            let mut chunk = vec![0; size + 2];
            reader.read_exact(&mut chunk).ok()?;
            if size == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..size]);
        }
    } else {
        body.resize(length, 0);
        reader.read_exact(&mut body).ok()?;
    }

    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let mut backend = Command::new("git");
    backend
        .arg("http-backend")
        .env("GIT_PROJECT_ROOT", root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REQUEST_METHOD", &method)
        .env("PATH_INFO", path)
        .env("QUERY_STRING", query)
        .env("CONTENT_TYPE", &content_type)
        .env("CONTENT_LENGTH", body.len().to_string())
        .env("REMOTE_ADDR", "127.0.0.1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if let Some(protocol) = protocol {
        backend.env("GIT_PROTOCOL", protocol);
    }
    let mut child = backend.spawn().ok()?;
    child.stdin.take()?.write_all(&body).ok()?;
    let out = child.wait_with_output().ok()?.stdout;

    let split = out.windows(4).position(|w| w == b"\r\n\r\n")?;
    let (head, payload) = (String::from_utf8_lossy(&out[..split]), &out[split + 4..]);
    let mut status = "200 OK".to_owned();
    let mut response = String::new();
    for line in head.lines() {
        match line.split_once(':') {
            Some((name, value)) if name.eq_ignore_ascii_case("status") => status = value.trim().to_owned(),
            Some(_) => response.push_str(&format!("{line}\r\n")),
            None => {}
        }
    }
    let mut stream = stream;
    write!(
        stream,
        "HTTP/1.1 {status}\r\n{response}Content-Length: {}\r\n\r\n",
        payload.len()
    )
    .ok()?;
    stream.write_all(payload).ok()?;
    Some(())
}

/// An `origin.git` with more branches than gix sends haves in its first round, and a clone of it
/// that is one commit behind on `main`: the server says `ready` before the client says `done`.
struct Fixture {
    _dir: TempDir,
    work: PathBuf,
    clone: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (remote, work, clone) = (
            dir.path().join("origin.git"),
            dir.path().join("work"),
            dir.path().join("clone"),
        );
        std::fs::create_dir_all(&work).unwrap();
        git(
            dir.path(),
            &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()],
        );
        git(&work, &["init", "-q", "-b", "main"]);
        for i in 0..24 {
            std::fs::write(work.join("a.txt"), format!("{i}")).unwrap();
            git(&work, &["add", "-A"]);
            git(&work, &["commit", "-q", "-m", &format!("c{i}")]);
            git(&work, &["branch", &format!("b{i}")]);
        }
        git(&work, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git(&work, &["push", "-q", "origin", "--all"]);
        git(
            dir.path(),
            &[
                "clone",
                "-q",
                "--bare",
                remote.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );
        git(
            &clone,
            &["config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"],
        );
        git(&clone, &["fetch", "-q", "origin"]);
        git(&clone, &["config", "user.name", "Tess"]);
        git(&clone, &["config", "user.email", "t@t"]);
        Self { _dir: dir, work, clone }
    }

    fn advance_remote(&self) -> String {
        std::fs::write(self.work.join("a.txt"), "new").unwrap();
        git(&self.work, &["commit", "-q", "-am", "new"]);
        git(&self.work, &["push", "-q", "origin", "main"]);
        git(&self.work, &["rev-parse", "HEAD"])
    }
}

#[test]
fn a_protocol_v1_fetch_over_http_completes_when_the_server_is_ready_before_done() {
    let fx = Fixture::new();
    let new_main = fx.advance_remote();
    let base = serve(fx._dir.path().to_owned());
    git(
        &fx.clone,
        &["config", "remote.origin.url", &format!("{base}/origin.git")],
    );
    // Azure DevOps speaks V1 only; the local server would otherwise answer the V2 gix asks for.
    git(&fx.clone, &["config", "protocol.version", "1"]);
    let repo = GixRepo::for_test(&fx.clone);

    // A hung fetch holds its blocking thread forever; the runtime is left without waiting for it.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let fetched = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(30), repo.fetch("origin", Via::Configured(None))).await
    });
    runtime.shutdown_background();

    let updated = fetched.expect("the fetch finishes").unwrap();
    assert_eq!(updated, 1, "only main moved");
    assert_eq!(git(&fx.clone, &["rev-parse", "refs/remotes/origin/main"]), new_main);
}
