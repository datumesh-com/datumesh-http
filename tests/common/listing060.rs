//! Fixtures of the listing tests (CONTRACT-http-listing.md, http 0.6.0; `tests/listing_060_*.rs`): a second mesh in process
//! under the host `source.example` (its own SQLite store, key and catalogue), a recording proxy in front of it (the origin
//! `DATUMESH_PEERS` maps the host to, which can rewrite, replace, delay or fail answers), and the listing mesh, the 1.8b
//! SQLite world of `mesh-test.datumesh.com` with a relay of the source in the subject finance · rate · interest_rate · gb.
//!
//! Include after `catworld`, `catjob` and `crworld`:
//!
//! ```text
//! #[path = "common/catjob.rs"] mod catjob;
//! #[path = "common/catworld.rs"] mod catworld;
//! mod common;
//! #[path = "common/crworld.rs"] mod crworld;
//! #[path = "common/listing060.rs"] mod listing060;
//! ```
//!
//! # The interfaces these tests pin (see `BRIEF-listing-060-implement.md`)
//!
//! * `datumesh_http::listing`: `fetch`, `apply`, `run_host`, `origin_of`, `Limits`, `Fetch`, `FetchedName`, `Cut`, `Outcome`,
//!   `Applied`, the constants `DEFAULT_INTERVAL`, `MAX_HOSTS`, `UNREACHABLE_DROP_AFTER`.
//! * `datumesh_http::find_order`: `FindKey`, `SubjectRank`, `today_cmp`, `order`, `sort_keys`, `cmp_sort_keys`.
//! * `Config::listed_meshes`, `Config::listing_interval`, `Config::listing_first_delay` (`DATUMESH_LISTED_MESHES`,
//!   `DATUMESH_LISTING_INTERVAL_SECS`, `DATUMESH_LISTING_FIRST_DELAY_SECS`).
//! * The members of a listed result (find, describe, `/fn` as JSON): `listed: true`, `mesh: "https://<host>"`,
//!   `last_seen: "<YYYY-MM-DD>"`, `publisher: "<host>"`, `address: "https://<host>/fn/<name>"`; no `cell` and no `slice` member.
//! * A read of a listed name: 404, `{"outcome":"not_covered","error":"E_LISTED_ELSEWHERE","mesh":"https://<host>",...}`.
//! * Metrics: `datumesh_listed_names{host="<h>"} N`, `datumesh_listing_outcome{host="<h>",outcome="<o>"} 1` (the last run's
//!   outcome only), `datumesh_listing_last_run_seconds{host="<h>"} <unix seconds>`, `datumesh_listed_names_total N`.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use datumesh_core::FunctionDescriptor;
use datumesh_http::listing::{self, Applied, Fetch, FetchedName, Limits};
use datumesh_http::Config;
use datumesh_store::{migrate, ListedHost, Listing, SqliteStore, Store, SubjectKeys};
use serde_json::{json, Value};

use crate::catjob::*;
use crate::catworld::*;
use crate::common::lite::*;
use crate::common::pub031::{function_line, opts, T1, T2};
use crate::common::readpath::*;
use crate::common::*;
use crate::crworld::*;

/// The listed host.
pub const SRC: &str = "source.example";
/// Its origin, as listed results and `E_LISTED_ELSEWHERE` name it (the peers mapping is local routing, not the origin).
pub const SRC_ORIGIN: &str = "https://source.example";
/// A third publisher, whose name the source's catalogue lists from elsewhere (not followed).
pub const THIRD: &str = "third.example";

/// The listing runs' instants: RUN1 is the mesh clock (MESH_NOW); RUN1B later the same day; RUN2 the next day.
pub const RUN1: &str = "2026-11-16T00:00:00.000000Z";
pub const RUN1B: &str = "2026-11-16T06:00:00.000000Z";
pub const RUN2: &str = "2026-11-17T01:00:00.000000Z";
pub const RUN3: &str = "2026-11-17T02:00:00.000000Z";
pub const DAY1: &str = "2026-11-16";
pub const DAY2: &str = "2026-11-17";

/// The subject of the relay and the source: finance · rate · interest_rate · gb.
pub const GB: (&str, &str, &str, &str) = ("finance", "rate", "interest_rate", "gb");

pub fn gb() -> SubjectKeys {
    SubjectKeys::new(GB.0, GB.1, GB.2, GB.3)
}

pub fn gb_facets() -> Value {
    fac(&[GB.0], &[GB.1], &[GB.2], &[GB.3], &["daily"])
}

pub fn subject_facets(s: &SubjectKeys) -> Value {
    fac(&[&s.domain], &[&s.thing], &[&s.measure], &[&s.region], &["daily"])
}

/// The source's own function.
pub fn bank_rate() -> String {
    format!("{SRC}/bank_rate")
}

/// The listing mesh's relay of it: `<HOST>/relay/source.example/bank_rate`.
pub fn relay() -> String {
    host_name(&format!("relay/{SRC}/bank_rate"))
}

pub const LABEL: &str = "Bank Rate";
pub const DESCRIPTION: &str = "The bank rate as the source publishes it under its own name.";

// ---------------------------------------------------------------------------------------------------------------------
// Descriptors and fetches for the store-side tests (no network)

/// A stored descriptor of one date argument returning a number; `body` tells descriptors apart. (bytes, hash)
pub fn descriptor(body: &str) -> (Vec<u8>, String) {
    let text = format!(r#"{{"code":{{"body":"{body}","lang":"stored"}},"inputs":[],"signature":{}}}"#, date_sig());
    let d = FunctionDescriptor::from_json(text.as_bytes()).unwrap();
    (d.canonical().to_vec(), d.hash())
}

/// A verified name of the source in `subjects`, its descriptor told apart by `body`.
pub fn name_of(name: &str, body: &str, subjects: Vec<SubjectKeys>) -> FetchedName {
    let (bytes, hash) = descriptor(body);
    FetchedName {
        name: name.to_string(),
        fn_hash: hash,
        subjects,
        descriptor: bytes,
        label: LABEL.to_string(),
        description: DESCRIPTION.to_string(),
    }
}

/// The key id of the source's test key.
pub fn src_key_id() -> String {
    key_for(SRC).key_id().to_string()
}

/// A successful fetch of `names`, uncut.
pub fn fetched(names: Vec<FetchedName>) -> Fetch {
    Fetch::Fetched { key_id: src_key_id(), names, refused: vec![], cut: None }
}

pub fn unreachable() -> Fetch {
    Fetch::Unreachable { detail: "connection refused".into() }
}

/// `listing::apply` on the listing mesh's store, unwrapped.
pub fn apply(lite: &Lite, f: &Fetch, now: &str) -> Applied {
    listing::apply(&*lite.store, SRC, f, now).unwrap_or_else(|e| panic!("apply at {now}: {e}"))
}

/// A listing written to the store directly (the rank tests).
pub fn put_listing(lite: &Lite, host: &str, name: &str, subjects: Vec<SubjectKeys>, listed_at: &str) {
    let (bytes, hash) = descriptor(name);
    lite.store
        .put_listing(&Listing {
            host: host.to_string(),
            name: name.to_string(),
            fn_hash: hash,
            subjects,
            descriptor: bytes,
            key_id: key_for(host).key_id().to_string(),
            label: LABEL.to_string(),
            description: DESCRIPTION.to_string(),
            fetched_at: listed_at.to_string(),
            listed_at: listed_at.to_string(),
        })
        .unwrap_or_else(|e| panic!("put_listing {name}: {e}"));
    lite.store
        .put_listed_host(&ListedHost {
            host: host.into(),
            last_seen: Some(listed_at[..10].to_string()),
            unreachable_since: None,
        })
        .unwrap();
}

// ---------------------------------------------------------------------------------------------------------------------
// The listing mesh

/// The lines of a relay of `name` in `facets` (a relayed cell, its catalogue entry labelled `label`).
pub fn relay_lines(name: &str, facets: Value, label: &str) -> Vec<String> {
    vec![
        fn_line(name),
        relay_cell(name),
        catalog_line_of(
            name,
            "function",
            json!({"facets": facets, "label": label, "description": "A relay of the bank rate."}),
        ),
    ]
}

/// The listing mesh's store: the 1.8b world with the relay of the source's bank rate in GB, labelled `Bank Rate` (so an
/// exact match of the query `bank rate`, ahead of the listed source by today's key).
pub fn listing_lite(label: &str) -> Lite {
    let lite = Lite::new(label);
    publish(&lite, "relay.jsonl", &relay_lines(&relay(), gb_facets(), LABEL), T_RELAY)
        .unwrap_or_else(|e| panic!("relay: {e}"));
    lite
}

/// The rows of a SQL count on a test store's SQLite file.
pub fn rows(lite: &Lite, sql: &str) -> i64 {
    let c = rusqlite::Connection::open(lite.path("store.db")).unwrap();
    c.query_row(sql, [], |r| r.get(0)).unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// The counts of the tables a listing never writes: cells, receipts, versions, name_bindings, class_bindings, namespaces.
pub fn untouched(lite: &Lite) -> Vec<i64> {
    ["cells", "receipts", "versions", "name_bindings", "class_bindings", "namespaces"]
        .iter()
        .map(|t| rows(lite, &format!("SELECT count(*) FROM {t}")))
        .collect()
}

/// The rows that would show a listed name hosted here: its cells, bindings, and the namespace of its host.
pub fn hosted_rows(lite: &Lite, host: &str) -> i64 {
    rows(lite, &format!("SELECT count(*) FROM cells WHERE ns LIKE '{host}%'"))
        + rows(lite, &format!("SELECT count(*) FROM name_bindings WHERE fn_name LIKE '{host}/%'"))
        + rows(lite, &format!("SELECT count(*) FROM namespaces WHERE host = '{host}'"))
}

/// `updated_at` of every catalogue entry, in order: a run that writes no catalogue row leaves it equal.
pub fn catalogue_stamps(lite: &Lite) -> Vec<String> {
    let c = rusqlite::Connection::open(lite.path("store.db")).unwrap();
    let mut st =
        c.prepare("SELECT kind || ' ' || name || ' ' || updated_at FROM catalog_entries ORDER BY kind, name").unwrap();
    st.query_map([], |r| r.get::<_, String>(0)).unwrap().map(|r| r.unwrap()).collect()
}

/// The sources of a subject in rank order, read as of the mesh clock (a named asof: no cache).
pub async fn sources_at(m: &Served, s: (&str, &str, &str, &str)) -> Vec<String> {
    let n = ask(m, &path(&sc(s.0, s.1, s.2, s.3, "sources()"), Some(MESH_NOW))).await;
    assert_eq!((n.status, n.outcome()), (200, "ok"), "sources(): {}", n.text);
    let n: u32 = n.value().unwrap().parse().unwrap();
    let mut out = Vec::new();
    for r in 1..=n {
        let a = source_at(m, s, r).await;
        assert_eq!(a.outcome(), "ok", "source({r}): {}", a.text);
        out.push(a.value().unwrap().to_string());
    }
    out
}

/// `source(rank)` of a subject as of the mesh clock.
pub async fn source_at(m: &Served, s: (&str, &str, &str, &str), rank: u32) -> Ans {
    ask(m, &path(&sc(s.0, s.1, s.2, s.3, &format!("source({rank})")), Some(MESH_NOW))).await
}

/// The configuration of the listing mesh: `vars` plus the peers mapping of the source to `origin` (when given).
pub fn listing_cfg(origin: Option<&str>, vars: &[(&str, &str)]) -> Config {
    let mut v: Vec<(String, String)> = vars.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    if let Some(o) = origin {
        v.push(("DATUMESH_PEERS".into(), format!("{SRC}={o}")));
    }
    let refs: Vec<(&str, &str)> = v.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    cfg(&refs)
}

// ---------------------------------------------------------------------------------------------------------------------
// The source mesh: another host, in process

/// A store of its own for `host`, migrated, holding `host`'s key document (its test key from K_FROM).
pub fn host_lite(host: &str, label: &str) -> Lite {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = out_dir().join(format!("lite-{label}-{host}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = SqliteStore::open(dir.join("store.db")).unwrap();
    migrate::sqlite::apply(&store).unwrap();
    let key = key_for(host);
    store.put_namespace(host, &key_doc(host, &key), K_FROM).unwrap();
    Lite { store: Arc::new(store), key, dir }
}

/// Publishes `lines` on `lite` as `host` (its key, its namespace owned) at `clock`.
pub fn publish_as(lite: &Lite, host: &str, file: &str, lines: &[String], clock: &'static str) {
    let f = crate::common::pub031::write(lite, file, lines);
    datumesh_http::publish::publish_stored_with(&*lite.store, &lite.key, host, &[host.to_string()], &f, &opts(clock))
        .unwrap_or_else(|e| panic!("publish as {host}: {e}"));
}

/// Runs the catalogue job on `lite` as `host`.
pub fn build_as(lite: &Lite, host: &str, clock: &'static str) -> u64 {
    datumesh_http::catalog_build::catalog_build_with(&*lite.store, &lite.key, host, LICENCE, &opts(clock))
        .unwrap_or_else(|e| panic!("catalog-build as {host}: {e}"))
        .cells
}

/// A function of the source with its catalogue entry (label, description, facets).
pub fn src_fn_lines(name: &str, label: &str, description: &str, facets: Value) -> Vec<String> {
    vec![
        function_line(name, &date_sig(), None),
        catalog_line_of(name, "function", json!({"facets": facets, "label": label, "description": description})),
    ]
}

/// The source's store: `source.example/bank_rate` in GB, a name of a third publisher in GB imported into it (its catalogue
/// lists it, from elsewhere), `more` lines, and its catalogue built at T3.
pub fn source_lite(label: &str, more: &[String]) -> Lite {
    let lite = host_lite(SRC, label);
    let mut lines = src_fn_lines(&bank_rate(), LABEL, DESCRIPTION, gb_facets());
    lines.extend(more.iter().cloned());
    publish_as(&lite, SRC, "source.jsonl", &lines, T1);
    lite.store.put_namespace(THIRD, &key_doc(THIRD, &key_for(THIRD)), K_FROM).unwrap();
    let third =
        foreign_bundle(THIRD, &format!("{THIRD}/rates"), &format!("{THIRD}/rates/bank_rate"), Some(gb_facets()));
    datumesh_http::bundle::import_signed(&*lite.store, SRC, &[THIRD.to_string()], &FixedClock(T2), &third)
        .unwrap_or_else(|e| panic!("third: {e}"));
    let n = build_as(&lite, SRC, T3);
    assert!(n > 0, "the source's catalogue is built");
    lite
}

/// Serves a store as `host`.
pub async fn serve_as(lite: Lite, host: &str) -> Served {
    let mut env: Vec<(String, String)> = vec![("DATUMESH_HOST".into(), host.into())];
    for k in ["DATUMESH_RATE_PER_IP", "DATUMESH_RATE_IP_BURST", "DATUMESH_RATE_SERIES_PER_IP"] {
        env.push((k.into(), "1000000".into()));
    }
    let c = Config::from_vars(move |k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).unwrap();
    build_served(lite, Some(c), |_| {}).await
}

/// The source mesh served, with the proxy in front.
pub struct Source {
    pub mesh: Served,
    pub proxy: Proxy,
}

/// The source world served behind a proxy.
pub async fn source(label: &str, more: &[String]) -> Source {
    let mesh = serve_as(source_lite(label, more), SRC).await;
    let proxy = Proxy::start(&mesh.base).await;
    Source { mesh, proxy }
}

/// The source's function hash of `name`.
pub fn src_hash(s: &Source, name: &str) -> String {
    s.mesh
        .lite
        .store
        .resolve_binding(name, END)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} unbound on the source"))
        .fn_hash
}

/// `listing::fetch` on the blocking pool.
pub async fn fetch_at(origin: &str, limits: Limits, now: &'static str) -> Fetch {
    let origin = origin.to_string();
    tokio::task::spawn_blocking(move || listing::fetch(SRC, &origin, &limits, now)).await.unwrap()
}

/// `listing::run_host` on the listing mesh's store, on the blocking pool.
pub async fn run_at(lite: &Lite, origin: &str, limits: Limits, now: &'static str) -> Applied {
    let store = lite.store.clone();
    let origin = origin.to_string();
    tokio::task::spawn_blocking(move || listing::run_host(&*store, SRC, &origin, &limits, now))
        .await
        .unwrap()
        .unwrap_or_else(|e| panic!("run_host at {now}: {e}"))
}

/// The names of a fetch, when it fetched.
pub fn fetched_names(f: &Fetch) -> Vec<String> {
    match f {
        Fetch::Fetched { names, .. } => names.iter().map(|n| n.name.clone()).collect(),
        other => panic!("not fetched: {other:?}"),
    }
}

/// An origin nothing listens on (connection refused).
pub fn dead_origin() -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    drop(l);
    format!("http://{a}")
}

// ---------------------------------------------------------------------------------------------------------------------
// The proxy: records each request and forwards it; can replace, rewrite, delay or fail answers

#[derive(Default)]
struct Rules {
    /// An exact path (no query) answered with this status and body.
    overrides: Vec<(String, u16, Vec<u8>)>,
    /// Bytes replaced by others (same length) in every body but the key and mesh documents, describe and descriptors.
    rewrite: Option<(Vec<u8>, Vec<u8>)>,
    /// Requests whose path starts with this wait so long first.
    delay: Option<(String, u64)>,
    /// Every request answered with this status.
    fail_all: Option<u16>,
}

struct ProxyState {
    upstream: String,
    log: Mutex<Vec<(String, usize)>>,
    rules: Mutex<Rules>,
    client: reqwest::Client,
}

pub struct Proxy {
    pub base: String,
    state: Arc<ProxyState>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn excluded(path: &str) -> bool {
    path.starts_with("/.well-known/") || path.starts_with("/describe/") || path.starts_with("/descriptor/")
}

fn replace_all(body: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        if !from.is_empty() && body[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(body[i]);
            i += 1;
        }
    }
    out
}

async fn handle(State(st): State<Arc<ProxyState>>, req: Request) -> Response {
    let pq = req.uri().path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| "/".into());
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let accept = req.headers().get(header::ACCEPT).and_then(|v| v.to_str().ok()).map(str::to_string);
    let body = axum::body::to_bytes(req.into_body(), 64 << 20).await.unwrap_or_default();
    let (fail, over, delay, rewrite) = {
        let r = st.rules.lock().unwrap();
        (
            r.fail_all,
            r.overrides.iter().find(|(p, _, _)| *p == path).map(|(_, s, b)| (*s, b.clone())),
            r.delay.clone().filter(|(p, _)| path.starts_with(p.as_str())).map(|(_, ms)| ms),
            r.rewrite.clone(),
        )
    };
    if let Some(ms) = delay {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
    let (status, ctype, out) = if let Some(s) = fail {
        (s, "application/json".to_string(), br#"{"error":"down"}"#.to_vec())
    } else if let Some((s, b)) = over {
        (s, "application/json".to_string(), b)
    } else {
        let url = format!("{}{pq}", st.upstream);
        let mut rb = if method == axum::http::Method::POST {
            st.client.post(&url).body(body.to_vec()).header("content-type", "application/json")
        } else {
            st.client.get(&url)
        };
        if let Some(a) = accept {
            rb = rb.header("accept", a);
        }
        match rb.send().await {
            Ok(r) => {
                let s = r.status().as_u16();
                let ct = r
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("application/octet-stream")
                    .to_string();
                let mut b = r.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
                if let Some((from, to)) = rewrite.filter(|_| !excluded(&path)) {
                    b = replace_all(&b, &from, &to);
                }
                (s, ct, b)
            }
            Err(_) => (502, "application/json".to_string(), br#"{"error":"upstream"}"#.to_vec()),
        }
    };
    st.log.lock().unwrap().push((pq, out.len()));
    let mut resp = (StatusCode::from_u16(status).unwrap(), out).into_response();
    resp.headers_mut().insert(header::CONTENT_TYPE, ctype.parse().unwrap());
    resp
}

impl Proxy {
    /// A proxy on loopback forwarding to `upstream` (an origin, `http://127.0.0.1:port`).
    pub async fn start(upstream: &str) -> Proxy {
        let state = Arc::new(ProxyState {
            upstream: upstream.trim_end_matches('/').to_string(),
            log: Mutex::new(Vec::new()),
            rules: Mutex::new(Rules::default()),
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
        });
        let app = axum::Router::new().fallback(handle).with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Proxy { base: format!("http://{addr}"), state, task }
    }

    /// The path and query of each request, in order.
    pub fn log(&self) -> Vec<String> {
        self.state.log.lock().unwrap().iter().map(|(p, _)| p.clone()).collect()
    }

    /// The bytes of every answer's body, summed.
    pub fn bytes(&self) -> usize {
        self.state.log.lock().unwrap().iter().map(|(_, n)| n).sum()
    }

    pub fn clear_log(&self) {
        self.state.log.lock().unwrap().clear();
    }

    /// `path` (exact, no query) answered with `status` and `body` from now on.
    pub fn override_path(&self, path: &str, status: u16, body: Vec<u8>) {
        self.state.rules.lock().unwrap().overrides.push((path.to_string(), status, body));
    }

    /// `from` replaced by `to` in every body but the key and mesh documents, describe and descriptors.
    pub fn rewrite(&self, from: &str, to: &str) {
        assert_eq!(from.len(), to.len(), "a rewrite keeps the length (an Arrow stream stays well formed)");
        self.state.rules.lock().unwrap().rewrite = Some((from.as_bytes().to_vec(), to.as_bytes().to_vec()));
    }

    /// Requests whose path starts with `prefix` wait `ms` first.
    pub fn delay(&self, prefix: &str, ms: u64) {
        self.state.rules.lock().unwrap().delay = Some((prefix.to_string(), ms));
    }

    /// Every request answered with `status` (None: forwarded again).
    pub fn fail_all(&self, status: Option<u16>) {
        self.state.rules.lock().unwrap().fail_all = status;
    }
}

/// The listing mesh's world for the route tests: the source behind its proxy; the listing store with the relay, the source
/// listed by one run at RUN1 and the catalogue built at T3. Not yet served.
pub struct World {
    pub src: Source,
    pub lite: Lite,
}

pub async fn listed_world(label: &str) -> World {
    let src = source(label, &[]).await;
    let lite = listing_lite(label);
    let a = run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await;
    assert_eq!(a.added, vec![bank_rate()], "the first run lists the source's name: {a:?}");
    build(&lite, T3);
    World { src, lite }
}

/// The listing mesh serving `w.lite` with `vars`, peers to the proxy, the listing thread held off for an hour.
pub async fn serve_listed(w: World, vars: &[(&str, &str)]) -> (Served, Source) {
    let mut v: Vec<(&str, &str)> = vec![("DATUMESH_LISTED_MESHES", SRC), ("DATUMESH_LISTING_FIRST_DELAY_SECS", "3600")];
    v.extend_from_slice(vars);
    let c = listing_cfg(Some(&w.src.proxy.base), &v);
    let m = build_served(w.lite, Some(c), |_| {}).await;
    (m, w.src)
}
