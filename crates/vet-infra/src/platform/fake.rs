//! A local server that answers like a platform, for the tests of the clients.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use vet_app::platform::{Credential, CredentialProvider, CredentialSource, Token};

#[derive(Clone, Debug)]
pub(crate) struct Seen {
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

pub(crate) struct Canned {
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
pub(crate) struct Fake {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Fake {
    /// `handler` gets the request and the server's base URL.
    pub fn start(handler: impl Fn(&Seen, &str) -> Canned + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (seen, stop) = (seen.clone(), stop.clone());
            std::thread::spawn(move || {
                let base = format!("http://{addr}");
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(mut stream) = stream else { continue };
                    let Some(request) = read_request(&mut stream) else {
                        continue;
                    };
                    let canned = handler(&request, &base);
                    seen.lock().unwrap().push(request);
                    write_response(&mut stream, &canned, &base);
                }
            })
        };
        Self {
            addr,
            seen,
            stop,
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
pub(crate) struct Signed(pub Option<&'static str>, pub CredentialSource);

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
