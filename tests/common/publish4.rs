//! Fixtures of the Build 2 Part 4 tests (CONTRACT-build2-part4-publish P1, P2): a public mesh (the
//! test mesh, `mesh-test.datumesh.com`, SQLite, clock fixed at MESH_NOW) that takes `POST /bundles`,
//! and stub meshes of invented publisher hosts (`pub.example`, ...), reached through the public mesh's
//! `peers` (the cross-mesh tests' resolver hook): the public mesh fetches
//! `https://<host>/.well-known/datumesh.json` from the stub, as it would from the host.
//!
//! Included by the test files with `#[path = "common/publish4.rs"] mod publish4;` after
//! `#[path = "common/calculus.rs"] mod calculus;` (the computed fixtures are the calculus ones).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use datumesh_core::json::{self, Value};
use datumesh_core::{Cell, FunctionDescriptor, ReceiptBuilder};
use datumesh_http::bundle;
use datumesh_http::{Config, Mesh};
use datumesh_store::Store;

use crate::calculus::*;
use crate::common::lite::*;
use crate::common::pub031::{function_line, opts, write};
use crate::common::readpath::*;
use crate::common::*;

/// The invented publisher host most tests use.
pub const PUB: &str = "pub.example";
/// A signing instant of a receipt: before MESH_NOW.
pub const TX: &str = "2026-11-15T11:00:00.000000Z";
/// The mesh's own namespace for the bundle of its own host.
pub const OWN_NS: &str = "mesh-test.datumesh.com/b4own";

pub fn ns_of(host: &str) -> String {
    format!("{host}/b4")
}

pub const STORED: &str = r#"{"code":{"body":"","lang":"stored"},"inputs":[],"signature":{"args":{"site":{"type":"string","valid":false}},"order":["site"],"returns":{"type":"number","unit":"1"}}}"#;

pub fn stored_fn() -> FunctionDescriptor {
    FunctionDescriptor::from_json(STORED.as_bytes()).unwrap()
}

/// A receipt of `<ns>/level(site)` signed by `host`'s test key, at `tx` (asof = issued = tx).
pub fn level_receipt(host: &str, site: &str, value: &str, tx: &str) -> Vec<u8> {
    let ns = ns_of(host);
    let cell = Cell { ns: ns.clone(), fn_name: "level".into(), fn_hash: stored_fn().hash(), args: args(&[("site", s(site))]) };
    let r = ReceiptBuilder::new()
        .cell(cell)
        .value(n(value))
        .valid(args(&[]))
        .asof(tx)
        .tx(tx)
        .issued(tx)
        .publisher(&ns)
        .signer_host(host)
        .licence("CC0-1.0")
        .summary("test")
        .sign(&key_for(host))
        .unwrap();
    receipt_json(&r)
}

pub fn level_name(host: &str) -> String {
    format!("{}/level", ns_of(host))
}

pub fn level_cell_path(host: &str, site: &str) -> String {
    format!("/cell/{}?site={site}", level_name(host))
}

/// A bundle of the stored function `<host>/b4/level` with one receipt, signed with `seed`;
/// a catalogue entry when `description` is given.
pub fn simple_bundle_with(host: &str, seed: &[u8; 32], receipts: Vec<Vec<u8>>, description: Option<&str>) -> Vec<u8> {
    let name = level_name(host);
    let catalog = match description {
        Some(d) => vec![catalog_entry(&name, "function", "Level", d, serde_json::json!({"site": "the site"}), serde_json::json!({}))],
        None => vec![],
    };
    bundle::build_with_catalog(
        &ns_of(host),
        vec![stored_fn().canonical()],
        vec![(name, stored_fn().hash())],
        receipts,
        catalog,
        seed,
    )
    .unwrap()
}

/// The ordinary good bundle of `host`: signed by its test key.
pub fn simple_bundle(host: &str) -> Vec<u8> {
    simple_bundle_with(host, &seed_for(host), vec![level_receipt(host, "N", "42", TX)], None)
}

/// Edits a built bundle's JSON object (members), re-serialised in JCS, signature untouched.
pub fn edit(bytes: &[u8], f: impl FnOnce(&mut Vec<(String, Value)>)) -> Vec<u8> {
    let mut v = json::parse(bytes).unwrap();
    if let Value::Object(m) = &mut v {
        f(m);
    }
    v.to_jcs()
}

// ---- stub publisher meshes ----------------------------------------------------------------------

/// A mesh of an invented host on loopback, serving its key document (and mesh document) as the
/// real thing: what `https://<host>/.well-known/...` reaches through the public mesh's peers.
pub struct Stub {
    pub host: String,
    pub base: String,
    task: tokio::task::JoinHandle<()>,
    _lite: Lite,
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Stub {
    /// The stub of `host` with its test key valid from 2000.
    pub async fn start(host: &str) -> Stub {
        Stub::with_doc(host, key_doc(host, &key_for(host)), &key_for(host)).await
    }

    /// The stub of `host` whose key document is `doc` (a rotation, a key from the future, ...); the
    /// mesh itself signs with `signing`, a key the document holds valid now.
    pub async fn with_doc(host: &str, doc: Vec<u8>, signing: &datumesh_core::SigningKey) -> Stub {
        let lite = Lite::new("p4stub");
        lite.store.put_namespace(host, &doc, K_FROM).unwrap();
        let shared: Arc<dyn Store + Send + Sync> = lite.store.clone();
        let mesh = Mesh::builder(Config::new(host), shared)
            .signing_key(signing.clone())
            .clock(Arc::new(FixedClock(MESH_NOW)))
            .build()
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let _ = mesh.serve(listener, std::future::pending::<()>()).await;
        });
        Stub { host: host.into(), base: format!("http://{addr}"), task, _lite: lite }
    }

    pub fn peer(&self) -> (String, String) {
        (self.host.clone(), self.base.clone())
    }
}

/// An origin nothing listens on (connection refused).
pub fn closed_origin() -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    drop(l);
    format!("http://{a}")
}

/// An origin that answers every request with `status` and `body`.
pub async fn raw_origin(status: u16, body: &'static str) -> (String, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let t = tokio::spawn(async move {
        loop {
            let Ok((mut c, _)) = l.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = c.read(&mut buf).await;
                let r = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = c.write_all(r.as_bytes()).await;
            });
        }
    });
    (format!("http://{addr}"), t)
}

/// An origin that accepts and never answers (for the 5 s timeout).
pub async fn silent_origin() -> (String, tokio::task::JoinHandle<()>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let t = tokio::spawn(async move {
        let mut held = Vec::new();
        loop {
            let Ok((c, _)) = l.accept().await else { return };
            held.push(c);
        }
    });
    (format!("http://{addr}"), t)
}

// ---- the public mesh ----------------------------------------------------------------------------

/// The public mesh's configuration through the environment route: `DATUMESH_PEERS` (host=origin),
/// `DATUMESH_PUBLISH_DOMAINS` (when given) and `DATUMESH_LOG_DIR` (`<dir>/bundles.log`, when given).
pub fn public_config(peers: &[(String, String)], domains: Option<&str>, log_dir: Option<&Path>) -> Config {
    let peers: String = peers.iter().map(|(h, o)| format!("{h}={o}")).collect::<Vec<_>>().join(",");
    let domains = domains.map(str::to_string);
    let log = log_dir.map(|p| p.to_string_lossy().into_owned());
    Config::from_vars(move |k| match k {
        "DATUMESH_HOST" => Some(HOST.to_string()),
        "DATUMESH_PEERS" if !peers.is_empty() => Some(peers.clone()),
        "DATUMESH_PUBLISH_DOMAINS" => domains.clone(),
        "DATUMESH_LOG_DIR" => log.clone(),
        _ => None,
    })
    .unwrap()
}

/// The public mesh, empty, on a store of its own; `log` is where its operator log goes.
pub struct Public {
    pub m: Served,
    pub log_dir: PathBuf,
}

pub async fn public(label: &str, peers: &[(String, String)], domains: Option<&str>) -> Public {
    let lite = Lite::new(label);
    let log_dir = lite.path("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let cfg = public_config(peers, domains, Some(&log_dir));
    let m = build_served(lite, Some(cfg), |_| {}).await;
    Public { m, log_dir }
}

pub struct Reply4 {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: Vec<u8>,
}

impl Reply4 {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&self.body)))
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn header(&self, k: &str) -> Option<String> {
        self.headers.get(k).and_then(|v| v.to_str().ok()).map(str::to_string)
    }
}

impl Public {
    /// `POST /bundles` with `body` (Content-Type application/json).
    pub async fn post(&self, body: Vec<u8>) -> Reply4 {
        let r = self
            .m
            .http
            .post(format!("{}/bundles", self.m.base))
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
        let status = r.status().as_u16();
        let headers = r.headers().clone();
        let body = r.bytes().await.unwrap().to_vec();
        Reply4 { status, headers, body }
    }

    pub async fn get(&self, path: &str, headers: &[(&str, &str)]) -> Reply4 {
        get(&self.m, path, headers).await
    }

    /// Asserts a refusal: one of the contract's statuses (400, 403, 422), `{"error": code, "detail"}`.
    #[track_caller]
    pub fn refused(&self, r: &Reply4, code: &str) {
        assert!([400, 403, 422].contains(&r.status), "{code}: status {}: {}", r.status, r.text());
        let j = r.json();
        assert_eq!(j["error"], code, "{}", r.text());
        assert!(j.get("detail").is_some(), "no detail: {}", r.text());
    }

    /// Nothing of the bundle of `host` (the `level` function and its receipt) is in the store.
    #[track_caller]
    pub fn nothing_of(&self, host: &str) {
        let name = level_name(host);
        assert!(self.m.lite.store.resolve_name(&name, END).unwrap().is_none(), "{name} is bound");
        assert!(self.m.lite.store.descriptor(&stored_fn().hash()).is_none(), "the descriptor is stored");
    }
}

pub async fn get(m: &Served, path: &str, headers: &[(&str, &str)]) -> Reply4 {
    let mut rb = m.http.get(format!("{}{path}", m.base));
    for (k, v) in headers {
        rb = rb.header(*k, *v);
    }
    let r = rb.send().await.unwrap();
    let status = r.status().as_u16();
    let headers = r.headers().clone();
    let body = r.bytes().await.unwrap().to_vec();
    Reply4 { status, headers, body }
}

/// The lines of the operator log of bundles, waiting briefly for the write.
pub async fn bundle_log(dir: &Path, want: impl Fn(&str) -> bool) -> String {
    let p = dir.join("bundles.log");
    for _ in 0..40 {
        let t = std::fs::read_to_string(&p).unwrap_or_default();
        if want(&t) {
            return t;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

// ---- computed functions in a bundle ------------------------------------------------------------

/// A store of the publisher `host` (its key document held), the calculus fixture `c` (names under
/// `host`) published on it by `publish-stored` without the public rule: the descriptors and names of
/// `reading` (stored) and `per_unit` (a formula), as a publisher's tooling takes them.
pub fn computed_parts(c: &Calc, host: &str) -> (Vec<Vec<u8>>, Vec<(String, String)>) {
    let lite = Lite::new("p4src");
    lite.store.put_namespace(host, &key_doc(host, &key_for(host)), K_FROM).unwrap();
    let f = write(&lite, "f.jsonl", &[function_line(&c.reading, &reading_sig(), None), per_unit_line(c, None)]);
    datumesh_http::publish::publish_stored_with(&*lite.store, &key_for(host), host, &[host.to_string()], &f, &opts(NOW))
        .unwrap_or_else(|e| panic!("source refused: {e}"));
    let mut descriptors = Vec::new();
    let mut names = Vec::new();
    for nm in [&c.reading, &c.per_unit] {
        let h = lite.store.resolve_binding(nm, END).unwrap().unwrap().fn_hash;
        let d = lite.store.descriptor(&h).unwrap().into_owned();
        descriptors.push(FunctionDescriptor::from_json(&d).unwrap().canonical());
        names.push((nm.to_string(), h));
    }
    (descriptors, names)
}

/// The tests of `per_unit` that need no data (supplied inputs, an absent site).
pub fn per_unit_bundle_tests(c: &Calc) -> Vec<String> {
    let f = &c.per_unit;
    let reading = |v: &str| Some(serde_json::json!({ format!("{}(site: \"N\")", c.reading): num(v) }));
    vec![
        test_line(f, &pu_call(c, "N", "2"), reading("10"), None, num("5")),
        test_line(f, &pu_call(c, "N", "0"), reading("10"), None, num("0")),
        test_line(f, &pu_call(c, "N", "-3"), reading("10"), None, num("0")),
        test_line(f, &pu_call(c, "ZZ", "2"), None, None, fault("E_FORMULA_ABSENT")),
    ]
}

/// The fixture of the computed bundle: names under `<host>/b4c`.
pub fn computed_calc(host: &str) -> Calc {
    Calc::new(&format!("{host}/b4c"))
}

/// A bundle of `reading` and `per_unit` without tests (`build_with_catalog`).
pub fn computed_bundle_without_tests(host: &str) -> Vec<u8> {
    let c = computed_calc(host);
    let (d, nm) = computed_parts(&c, host);
    bundle::build_with_catalog(&format!("{host}/b4c"), d, nm, vec![], vec![], &seed_for(host)).unwrap()
}
