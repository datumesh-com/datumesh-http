//! A fake embedding service for the Build 2 Part 2 tests (CONTRACT-build2-part2-read R2).
//!
//! It speaks the one route of Text Embeddings Inference that http uses: `POST /embed` with
//! `{"inputs": ["query: ...", "passage: ..."]}`, answered with `[[f32 x 384], ...]` (one
//! normalised vector per input). Like e5 it wants a prefix: an input that starts with neither
//! `query: ` nor `passage: ` is answered `400`. The vector of a text is a hash-based bag of words:
//! each lowercase alphanumeric word adds +-1 at a place chosen by its SHA-256, then the sum is
//! normalised. So two texts that share words are near, and `alias("falcon", "kestrel")` makes the
//! word `falcon` count as `kestrel`, which is how a test makes a "semantic" neighbour that shares
//! no word.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const DIM: usize = 384;

#[derive(Default)]
struct State {
    inputs: Vec<String>,
    aliases: HashMap<String, String>,
    delay_ms: u64,
    fail: bool,
}

pub struct FakeEmbed {
    pub addr: SocketAddr,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect()
}

/// The vector of `text` (prefix already removed) under `aliases`.
pub fn vector(text: &str, aliases: &HashMap<String, String>) -> Vec<f32> {
    let mut v = vec![0f32; DIM];
    for w in words(text) {
        let w = aliases.get(&w).cloned().unwrap_or(w);
        let h = datumesh_core::sha256(w.as_bytes());
        let idx = (u16::from_be_bytes([h[0], h[1]]) as usize) % DIM;
        v[idx] += if h[2] & 1 == 0 { 1.0 } else { -1.0 };
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

impl FakeEmbed {
    pub fn start() -> FakeEmbed {
        FakeEmbed::start_with(&[])
    }

    pub fn start_with(aliases: &[(&str, &str)]) -> FakeEmbed {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(State {
            aliases: aliases.iter().map(|(a, b)| (a.to_lowercase(), b.to_lowercase())).collect(),
            ..State::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, sp) = (state.clone(), stop.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if sp.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(c) = conn {
                    let st = st.clone();
                    std::thread::spawn(move || handle(c, st));
                }
            }
        });
        FakeEmbed { addr, state, stop }
    }

    /// The base URL, as `DATUMESH_EMBED_URL` has it.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Every input received so far, prefix included, in order.
    pub fn inputs(&self) -> Vec<String> {
        self.state.lock().unwrap().inputs.clone()
    }

    pub fn queries(&self) -> Vec<String> {
        self.inputs().into_iter().filter(|i| i.starts_with("query: ")).collect()
    }

    pub fn passages(&self) -> Vec<String> {
        self.inputs().into_iter().filter(|i| i.starts_with("passage: ")).collect()
    }

    /// Answer each request only after `ms` (to test the client's 2 s timeout).
    pub fn set_delay(&self, ms: u64) {
        self.state.lock().unwrap().delay_ms = ms;
    }

    /// Answer every request `503`.
    pub fn set_fail(&self, fail: bool) {
        self.state.lock().unwrap().fail = fail;
    }
}

impl Drop for FakeEmbed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
    }
}

/// A URL where nothing listens (the port was free a moment ago): the service is down.
pub fn dead_url() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    drop(l);
    format!("http://{a}")
}

fn handle(mut c: TcpStream, st: Arc<Mutex<State>>) {
    let _ = c.set_read_timeout(Some(Duration::from_secs(10)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let (head_end, len) = loop {
        let n = match c.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
            let len = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0);
            break (p + 4, len);
        }
    };
    while buf.len() < head_end + len {
        match c.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let first = head.lines().next().unwrap_or("").to_string();
    let reply = |c: &mut TcpStream, status: &str, body: &str| {
        let _ = write!(
            c,
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
    };
    if first.starts_with("GET /health") {
        return reply(&mut c, "200 OK", "{}");
    }
    if !first.starts_with("POST /embed ") {
        return reply(&mut c, "404 Not Found", "{}");
    }
    let (delay, fail, aliases) = {
        let s = st.lock().unwrap();
        (s.delay_ms, s.fail, s.aliases.clone())
    };
    let parsed: Result<serde_json::Value, _> = serde_json::from_slice(&buf[head_end..head_end + len]);
    let inputs: Vec<String> = match parsed.as_ref().ok().and_then(|v| v.get("inputs")) {
        Some(serde_json::Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        _ => return reply(&mut c, "422 Unprocessable Entity", r#"{"error":"inputs"}"#),
    };
    st.lock().unwrap().inputs.extend(inputs.iter().cloned());
    if delay > 0 {
        std::thread::sleep(Duration::from_millis(delay));
    }
    if fail {
        return reply(&mut c, "503 Service Unavailable", r#"{"error":"down"}"#);
    }
    let mut out = Vec::new();
    for i in &inputs {
        let Some(text) = i.strip_prefix("query: ").or_else(|| i.strip_prefix("passage: ")) else {
            return reply(&mut c, "400 Bad Request", r#"{"error":"input needs a query: or passage: prefix"}"#);
        };
        let v = vector(text, &aliases);
        out.push(format!("[{}]", v.iter().map(|x| format!("{x:.6}")).collect::<Vec<_>>().join(",")));
    }
    reply(&mut c, "200 OK", &format!("[{}]", out.join(",")));
}
