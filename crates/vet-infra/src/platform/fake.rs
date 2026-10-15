//! A local server that answers like a platform, for the tests of the clients.
#![allow(clippy::disallowed_methods)] // The fake server runs on a thread of its own.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use vet_app::platform::{Credential, CredentialProvider, CredentialSource, Token};

#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

pub struct Canned {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}

impl Canned {
    pub fn json(body: &str) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.to_owned(),
        }
    }

    pub fn status(status: u16, body: &str) -> Self {
        Self {
            status,
            ..Self::json(body)
        }
    }

    pub fn with(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

/// A server on a local port that answers each request as the handler says, one request
/// per connection.
pub struct Fake {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    /// The connection the server thread reads or answers now, if any. Dropping shuts it down so a
    /// read that waits for a request that will not come cannot hold the server thread up.
    current: Arc<Mutex<Option<TcpStream>>>,
    thread: Option<JoinHandle<()>>,
}

impl Fake {
    /// `handler` gets the request and the server's base URL.
    pub fn start(handler: impl Fn(&Seen, &str) -> Canned + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let current = Arc::new(Mutex::new(None));
        let thread = {
            let (seen, stop, current) = (seen.clone(), stop.clone(), current.clone());
            std::thread::spawn(move || {
                let base = format!("http://{addr}");
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    // The stop, the wake-up connection that ends the accept, and the connection to
                    // read next are decided under one lock, so a drop cannot miss either.
                    {
                        let mut current = current.lock().unwrap();
                        if stop.load(Ordering::SeqCst) {
                            return;
                        }
                        match stream.try_clone() {
                            Ok(handle) => *current = Some(handle),
                            Err(_) => continue,
                        }
                    }
                    let Some(request) = read_request(&mut stream) else {
                        *current.lock().unwrap() = None;
                        continue;
                    };
                    let canned = handler(&request, &base);
                    seen.lock().unwrap().push(request);
                    write_response(&mut stream, &canned, &base);
                    *current.lock().unwrap() = None;
                }
            })
        };
        Self {
            addr,
            seen,
            stop,
            current,
            thread: Some(thread),
        }
    }

    pub fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // A read waiting for a request that will not come ends when its socket is shut down.
        if let Some(stream) = self.current.lock().unwrap().take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        // Wakes the accept that waits for a connection, so the loop sees the stop.
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Seen> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let mut headers = HashMap::new();
    loop {
        line.clear();
        reader.read_line(&mut line).ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    let length = headers.get("content-length").and_then(|l| l.parse().ok()).unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Seen {
        method,
        path,
        headers,
        body: String::from_utf8(body).ok()?,
    })
}

fn write_response(stream: &mut TcpStream, canned: &Canned, base: &str) {
    let body = canned.body.replace("{base}", base);
    let mut head = format!(
        "HTTP/1.1 {} X\r\nConnection: close\r\nContent-Length: {}\r\n",
        canned.status,
        body.len()
    );
    for (name, value) in &canned.headers {
        head.push_str(&format!("{name}: {}\r\n", value.replace("{base}", base)));
    }
    let _ = stream.write_all(format!("{head}\r\n{body}").as_bytes());
}

/// Hands out one token, or none.
pub struct Signed(pub Option<&'static str>, pub CredentialSource);

impl Signed {
    pub fn cli(token: &'static str) -> Self {
        Self(Some(token), CredentialSource::Cli)
    }
}

impl CredentialProvider for Signed {
    fn credential(&self, _: &str, _: i64) -> impl Future<Output = Option<Credential>> + Send {
        std::future::ready(self.0.map(|token| Credential {
            token: Token::new(token).unwrap(),
            source: self.1,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    /// A client may connect and then send nothing or half a request (an idle pooled connection);
    /// the server thread then waits in `read_request`. Dropping the fake must not wait for that
    /// read, or a test that finished its body hangs until its deadline.
    #[test]
    fn dropping_the_server_does_not_wait_for_a_connection_that_sends_nothing() {
        let fake = Fake::start(|_, _| Canned::json("{}"));
        let mut idle = TcpStream::connect(fake.addr).unwrap();
        // A request line without the blank line that ends the headers: read_request waits.
        idle.write_all(b"GET /idle HTTP/1.1\r\n").unwrap();
        // Let the server accept the connection and reach the read.
        std::thread::sleep(Duration::from_millis(200));

        let (dropped, finished) = mpsc::channel();
        std::thread::spawn(move || {
            drop(fake);
            let _ = dropped.send(());
        });
        assert!(
            finished.recv_timeout(Duration::from_secs(10)).is_ok(),
            "dropping the fake waited for a connection that sent nothing"
        );
        drop(idle);
    }
}
