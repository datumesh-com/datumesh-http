//! http 0.4.7: the routes added in 0.4.5 and 0.4.6 read the store, and the store of the mesh is the synchronous
//! PostgreSQL client, which panics on an async (runtime) thread. The series tests run on SQLite, which does not
//! care; these run the same routes on a PostgreSQL store (`DATABASE_URL`), as mesh-test does.
//!
//! 0.4.6 resolved the call form of a slice address (`/slice/<fn>(date: *)`, Accept led by `text/html`) with
//! `call::resolve` on the async thread: the request's task panicked and the proxy answered 502. These tests
//! fail on 0.4.6 (a dropped connection, or a 5xx) and pass when every store read is on the pool.

#[path = "common/calculus.rs"]
mod calculus;
mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::lite::{FixedClock, HOST, K_FROM, NOW};
use common::pub031::{function_line, opts};
use common::readpath::{catalog_line, MESH_NOW};
use common::{key_for, on_thread, TestDb};
use datumesh_http::publish::publish_stored_with;
use datumesh_http::{Config, Mesh};
use datumesh_store::{PgStore, Store};
use serde_json::{json, Value};

const FXR: &str = "mesh-test.datumesh.com/relay/ecb.europa.eu/fx_reference";
const BANK: &str = "mesh-test.datumesh.com/relay/bankofengland.co.uk/bank_rate";
const DERIVED: &str = "mesh-test.datumesh.com/earth/bank_doubled";
const BOUND_CLOCK: &str = "2026-01-01T00:00:00.000000Z";
const BROWSER: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,*/*;q=0.8";

fn fx_sig() -> &'static str {
    r#"{"args":{"currency":{"type":"string","valid":false},"date":{"type":"date","valid":true}},"order":["currency","date"],"returns":{"type":"number","unit":"1"}}"#
}
fn bank_sig() -> &'static str {
    r#"{"args":{"date":{"type":"date","valid":true}},"order":["date"],"returns":{"type":"number","unit":"1"}}"#
}

fn stored(f: &str, args: Value, value: &str, tx: &str, domain: &str) -> String {
    json!({"cell": {"fn": f, "args": args}, "licence": "OGL-UK-3.0", "summary": "Synthetic test data.",
        "value": {"type": "number", "value": value}, "tx": tx, "asof": tx, "issued": tx,
        "source": {"name": domain, "url": format!("https://{domain}/data"), "retrieved": BOUND_CLOCK,
            "takeover": format!("https://{HOST}/takeover/{domain}")}})
    .to_string()
}

fn date(v: &str) -> Value {
    json!({"type": "date", "value": v})
}

fn early_lines() -> Vec<String> {
    vec![function_line(FXR, fx_sig(), None), function_line(BANK, bank_sig(), None)]
}

fn lines() -> Vec<String> {
    let mut v = Vec::new();
    for (d, cur, val) in [("2026-03-02", "USD", "1.0832"), ("2026-03-02", "JPY", "160.25"), ("2026-03-03", "USD", "1.0791")] {
        v.push(stored(
            FXR,
            json!({"currency": {"type": "string", "value": cur}, "date": date(d)}),
            val,
            &format!("{d}T15:00:00.000000Z"),
            "ecb.europa.eu",
        ));
    }
    for (d, val) in [("2026-03-19", "4.5"), ("2026-05-07", "4.25")] {
        v.push(stored(BANK, json!({"date": date(d)}), val, &format!("{d}T12:00:00.000000Z"), "bankofengland.co.uk"));
    }
    v.push(calculus::formula_line(DERIVED, &format!("{BANK}(date: date) * 2"), bank_sig(), None));
    v.push(catalog_line(
        FXR,
        "function",
        "Euro reference rate",
        "The euro foreign exchange reference rates.",
        json!({"currency": "the currency", "date": "the day"}),
        json!({"currency": [{"value": "USD", "label": "US dollar"}, {"value": "JPY", "label": "Japanese yen"}]}),
    ));
    v.push(catalog_line(BANK, "function", "Bank rate", "The bank's rate.", json!({"date": "the day"}), json!({})));
    v
}

fn series_json() -> String {
    json!([
        {"function": FXR, "granularity": "month", "date_arg": "date", "column_arg": "currency", "unit": "units per EUR", "kind": "relayed"},
        {"function": BANK, "granularity": "month", "date_arg": "date", "unit": "percent", "kind": "relayed"},
    ])
    .to_string()
}

/// A mesh on a PostgreSQL store, serving on loopback.
struct Pg {
    base: String,
    http: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
    dir: PathBuf,
    _db: TestDb,
}

impl Drop for Pg {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Reply {
    status: u16,
    headers: reqwest::header::HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
    fn header(&self, k: &str) -> Option<String> {
        self.headers.get(k).and_then(|v| v.to_str().ok()).map(str::to_string)
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.text()))
    }
}

async fn serve_pg(label: &str) -> Pg {
    let db = on_thread(TestDb::migrated);
    let dir = common::out_dir().join(format!("pg-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let series = dir.join("series.json");
    std::fs::write(&series, series_json()).unwrap();
    let url = db.url();
    let (d2, series2) = (dir.clone(), series.clone());
    let mesh = on_thread(move || {
        let store = Arc::new(PgStore::connect_with(&url, datumesh_store::PoolOptions::new().max_size(8)).unwrap());
        let key = key_for(HOST);
        store.put_namespace(HOST, &datumesh_http::keydoc::with_key(HOST, None, &key, K_FROM).unwrap(), K_FROM).unwrap();
        let owned = vec![HOST.to_string()];
        for (name, ls, now) in [("functions.jsonl", early_lines(), BOUND_CLOCK), ("fixture.jsonl", lines(), NOW)] {
            let p = d2.join(name);
            std::fs::write(&p, ls.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
            publish_stored_with(&*store, &key, HOST, &owned, &p, &opts(now)).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let sp = series2.to_string_lossy().into_owned();
        let config = Config::from_vars(move |k| match k {
            "DATUMESH_HOST" => Some(HOST.to_string()),
            "DATUMESH_SERIES_FILE" => Some(sp.clone()),
            _ => None,
        })
        .unwrap();
        let shared: Arc<dyn Store + Send + Sync> = store;
        Mesh::builder(config, shared).signing_key(key).clock(Arc::new(FixedClock(MESH_NOW))).build().unwrap()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let _ = mesh.serve(listener, std::future::pending::<()>()).await;
    });
    let http = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    Pg { base: format!("http://{addr}"), http, task, dir, _db: db }
}

impl Pg {
    async fn send(&self, rb: reqwest::RequestBuilder, what: &str) -> Reply {
        let r = rb.send().await.unwrap_or_else(|e| panic!("{what}: no answer ({e}): the request's task died"));
        let (status, headers) = (r.status().as_u16(), r.headers().clone());
        let body = r.bytes().await.unwrap_or_else(|e| panic!("{what}: body lost ({e})")).to_vec();
        Reply { status, headers, body }
    }

    async fn get(&self, path: &str, accept: Option<&str>) -> Reply {
        let mut rb = self.http.get(format!("{}{path}", self.base));
        if let Some(a) = accept {
            rb = rb.header("accept", a);
        }
        self.send(rb, path).await
    }

    async fn ok(&self, path: &str, accept: Option<&str>) -> Reply {
        let r = self.get(path, accept).await;
        assert_eq!(r.status, 200, "{path}: {}", r.text());
        r
    }

    /// The mesh still answers.
    async fn alive(&self) {
        let r = self.get("/health", None).await;
        assert_eq!(r.status, 200, "/health: {}", r.text());
    }
}

fn is_to(r: &Reply, path: &str) -> bool {
    r.status == 303
        && r.header("cache-control").as_deref() == Some("no-store")
        && r.header("location").is_some_and(|l| l.ends_with(path))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_call_form_redirect_on_postgres_keeps_the_mesh_serving() {
    let m = serve_pg("redirect").await;
    let cases = [
        // unkeyed, percent-encoded, a date and `*`
        (format!("/slice/{BANK}(date:%202026-05-07)"), format!("/series/{BANK}/2026-05")),
        (format!("/slice/{BANK}(date:%20*)"), format!("/series/{BANK}/")),
        (format!("/slice/{BANK}%28date%3A%20%2A%29"), format!("/series/{BANK}/")),
        // not percent-encoded
        (format!("/slice/{BANK}(date: 2026-03-19)"), format!("/series/{BANK}/2026-03")),
        // keyed
        (format!("/slice/{FXR}(currency:%20*,%20date:%202026-03-02)"), format!("/series/{FXR}/2026-03")),
        (format!("/slice/{FXR}(currency:%20%22USD%22,%20date:%202026-03-03)"), format!("/series/{FXR}/2026-03")),
        (format!("/slice/{FXR}(currency:%20*,%20date:%20*)"), format!("/series/{FXR}/")),
        (format!("/slice/{FXR}%28currency%3A%20%2A%2C%20date%3A%202026-03-03%29"), format!("/series/{FXR}/2026-03")),
    ];
    for accept in ["text/html", BROWSER] {
        for (from, to) in &cases {
            let r = m.get(from, Some(accept)).await;
            assert!(is_to(&r, to), "{from} ({accept}): {} {:?} {}", r.status, r.header("location"), r.text());
            m.ok(to, None).await;
        }
    }
    m.alive().await;
    // the query form, and a call that is not a listed series' or not a call at all, are not redirected to a page
    let r = m.get(&format!("/slice/{BANK}?date=2026-05-07"), Some("text/html")).await;
    assert!(is_to(&r, &format!("/series/{BANK}/2026-05")), "{}", r.status);
    let r = m.get("/slice/no.such.host/fn(date:%20*)", Some("text/html")).await;
    assert_ne!(r.status, 303);
    assert!(r.status < 500, "{}", r.text());
    m.alive().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_json_client_on_the_call_form_still_gets_json_on_postgres() {
    let m = serve_pg("json-call").await;
    for accept in [None, Some("application/json")] {
        let r = m.ok(&format!("/slice/{FXR}(currency:%20*,%20date:%202026-03-02)"), accept).await;
        assert!(r.header("content-type").unwrap_or_default().starts_with("application/json"));
        assert_eq!(r.json()["members"].as_array().map(Vec::len), Some(2), "{}", r.text());
    }
    let r = m.ok(&format!("/slice/{BANK}(date:%20*)"), None).await;
    assert_eq!(r.json()["members"].as_array().map(Vec::len), Some(2), "{}", r.text());
    // a browser on a call that is not a series' is refused as before, not left to die
    let r = m.get("/slice/no.such.host/fn(date:%20*)", Some("text/html")).await;
    assert!(r.status < 500, "{} {}", r.status, r.text());
    m.alive().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_series_routes_read_the_store_on_postgres() {
    let m = serve_pg("series").await;
    for p in [
        format!("/series/{FXR}/2026-03"),
        format!("/series/{FXR}/2026-03.csv"),
        format!("/series/{FXR}/"),
        format!("/series/{BANK}/2026-05"),
        "/series/sitemap.xml".to_string(),
    ] {
        let r = m.ok(&p, None).await;
        assert!(!r.body.is_empty(), "{p}");
    }
    assert_eq!(m.get(&format!("/series/{BANK}/2026-07"), None).await.status, 404);
    m.alive().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn x_interval_reads_the_store_on_postgres() {
    let m = serve_pg("interval").await;
    let r = m.ok(&format!("/x/interval/{FXR}?currency=*&from=2026-03-01&to=2026-03-31"), None).await;
    assert_eq!(r.json()["count"], 3, "{}", r.text());
    let r = m.ok(&format!("/x/interval/{FXR}?currency=*&from=2026-03-01&to=2026-03-31&format=csv"), None).await;
    assert!(r.text().contains("receipt"), "{}", r.text());
    let r = m.ok(&format!("/x/interval/{BANK}?from=2026-03-01&to=2026-05-31&format=arrow"), None).await;
    assert!(!r.body.is_empty());
    m.alive().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn x_calc_reads_the_store_on_postgres() {
    let m = serve_pg("calc").await;
    let body = serde_json::to_vec(&json!({"formula": format!("{BANK}(date: 2026-05-07) * 2")})).unwrap();
    let rb = m.http.post(format!("{}/x/calc", m.base)).header("content-type", "application/json").body(body);
    let r = m.send(rb, "/x/calc").await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = r.json();
    let v = v.get("calculation").unwrap_or(&v);
    assert_eq!(v["value"], "8.5", "{v}");
    m.alive().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_function_and_takeover_pages_read_the_store_on_postgres() {
    let m = serve_pg("pages").await;
    for accept in [None, Some("text/html"), Some("application/json")] {
        for p in [format!("/fn/{BANK}"), "/takeover/bankofengland.co.uk".to_string()] {
            m.ok(&p, accept).await;
        }
    }
    m.ok(&format!("/fn/{BANK}?format=json"), None).await;
    m.ok("/takeover/ecb.europa.eu?format=json", None).await;
    m.alive().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn find_summary_by_name_reads_the_store_on_postgres() {
    let m = serve_pg("find").await;
    let r = m.ok("/find?summary=1", None).await;
    assert!(r.json().get("results").is_some() || r.json().is_object(), "{}", r.text());
    m.ok("/.well-known/datumesh-mesh.json", None).await;
    m.alive().await;
}
