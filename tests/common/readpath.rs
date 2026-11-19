//! Fixtures and servers of the Build 2 Part 2 read-path tests (CONTRACT-build2-part2-read R3, R5).
//!
//! The fixture is SYNTHETIC: a scripture-shaped function `lexicon/verse(book, chapter, verse)`
//! whose cells carry invented sentences (never real scripture text), with a publisher dictionary of
//! book codes and labels, plus a bird-counting namespace for ranking and a wide dictionary.
//! Everything is published through `publish-stored` lines, including the new `catalog` line kind,
//! and served by the real router on loopback. Responses are read as `serde_json::Value`.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use datumesh_http::{Config, Mesh};
use datumesh_store::Store;
use serde_json::{json, Value};

use super::lite::*;
use super::pub031::{function_line, write};

pub const MESH_NOW: &str = "2026-11-16T00:00:00.000000Z";
pub const NS: &str = "mesh-test.datumesh.com/scratch_b2";
pub const VERSE: &str = "mesh-test.datumesh.com/scratch_b2/lexicon/verse";
pub const CODE: &str = "mesh-test.datumesh.com/scratch_b2/lexicon/code";
pub const BIRD_A: &str = "mesh-test.datumesh.com/scratch_b2/zoo/bird_a";
pub const BIRD_B: &str = "mesh-test.datumesh.com/scratch_b2/zoo/bird_b";
pub const BIRD_C: &str = "mesh-test.datumesh.com/scratch_b2/zoo/bird_c";
/// The canonical template of `VERSE` (R3: `ns/fn(key: <type>, ...)`).
pub const VERSE_TEMPLATE: &str =
    "mesh-test.datumesh.com/scratch_b2/lexicon/verse(book: <string>, chapter: <number>, verse: <number>)";

const STR: &str = r#"{"type":"string","valid":false}"#;
const NUM: &str = r#"{"type":"number","unit":"1","valid":false}"#;

pub fn verse_sig() -> String {
    format!(
        r#"{{"args":{{"book":{STR},"chapter":{NUM},"verse":{NUM}}},"order":["book","chapter","verse"],"returns":"string"}}"#
    )
}

pub fn code_sig() -> String {
    format!(r#"{{"args":{{"code":{STR}}},"order":["code"],"returns":"string"}}"#)
}

pub fn site_sig() -> String {
    format!(r#"{{"args":{{"site":{STR}}},"order":["site"],"returns":"string"}}"#)
}

fn line(v: Value) -> String {
    v.to_string()
}

/// One `catalog` line of `publish-stored` (R5): `{"catalog": "<name>", "kind", "label",
/// "description", "args": {<arg>: <text>}, "examples": [{"call", "expected"}], "dictionary":
/// {<key>: [{"value", "label", "aliases"?}]}}`.
pub fn catalog_line(name: &str, kind: &str, label: &str, description: &str, args: Value, dict: Value) -> String {
    line(json!({
        "catalog": name, "kind": kind, "label": label, "description": description,
        "args": args, "examples": [], "dictionary": dict,
    }))
}

/// The same entry as a member of a bundle's `catalog` array (`name` instead of `catalog`).
pub fn catalog_entry(name: &str, kind: &str, label: &str, description: &str, args: Value, dict: Value) -> Vec<u8> {
    json!({
        "name": name, "kind": kind, "label": label, "description": description,
        "args": args, "examples": [], "dictionary": dict,
    })
    .to_string()
    .into_bytes()
}

pub fn verse_dictionary() -> Value {
    json!({"book": [
        {"value": "John", "label": "John"},
        {"value": "Gen", "label": "Genesis", "aliases": ["Gn"]},
        {"value": "2Chr", "label": "2 Chronicles"},
        {"value": "Jer", "label": "J\u{e9}r\u{e9}mie"},
        {"value": "Prim", "label": "Bible Primer"},
    ]})
}

pub const VERSE_DESC: &str = "A verse of the synthetic bible edition, by book, chapter and verse";

fn cell(function: &str, book: &str, chapter: &str, verse: &str, text: &str) -> String {
    line(json!({
        "cell": {"fn": function, "args": {
            "book": {"type": "string", "value": book},
            "chapter": {"type": "number", "value": chapter},
            "verse": {"type": "number", "value": verse},
        }},
        "licence": "CC0-1.0", "summary": "Synthetic test data.",
        "value": {"type": "string", "value": text},
    }))
}

/// The scripture-shaped fixture (3 cells, invented text) and the wide dictionary function.
pub fn lexicon_lines() -> Vec<String> {
    let wide: Vec<Value> =
        (0..60).map(|i| json!({"value": format!("c{i:02}"), "label": format!("Code {i:02}")})).collect();
    vec![
        function_line(VERSE, &verse_sig(), None),
        catalog_line(
            VERSE,
            "function",
            "Synthetic Verses",
            VERSE_DESC,
            json!({"book": "the book code", "chapter": "the chapter number", "verse": "the verse number, pilgrimage order"}),
            verse_dictionary(),
        ),
        cell(VERSE, "John", "3", "16", "Invented words, alpha beta gamma, line one."),
        cell(VERSE, "Gen", "1", "1", "Invented words, delta epsilon zeta, line two."),
        cell(VERSE, "2Chr", "1", "1", "Invented words, eta theta iota, line three."),
        function_line(CODE, &code_sig(), None),
        catalog_line(
            CODE,
            "function",
            "Coded entries",
            "Lookup of coded entries",
            json!({"code": "the entry code"}),
            json!({"code": wide}),
        ),
    ]
}

/// Three bird counters for ranking: A has the label `Kestrel` (exact for the query `kestrel`), B has the
/// word only in its description (text), C is about falcons (the fake embedder aliases falcon to
/// kestrel: vector). `more` extra functions `flock_NN` all describing "flock counts".
pub fn zoo_lines(more: usize) -> Vec<String> {
    let mut v = vec![
        function_line(BIRD_A, &site_sig(), None),
        catalog_line(BIRD_A, "function", "Kestrel", "Counts of birds at a site", json!({"site": "the site"}), json!({})),
        function_line(BIRD_B, &site_sig(), None),
        catalog_line(BIRD_B, "function", "Weekly tally", "Weekly kestrel tally at a site", json!({"site": "the site"}), json!({})),
        function_line(BIRD_C, &site_sig(), None),
        catalog_line(BIRD_C, "function", "Falcon watch", "Falcon sightings at a site", json!({"site": "the site"}), json!({})),
    ];
    for i in 0..more {
        let name = format!("{NS}/zoo/flock_{i:02}");
        v.push(function_line(&name, &site_sig(), None));
        v.push(catalog_line(&name, "function", &format!("Flock {i:02}"), "flock counts", json!({}), json!({})));
    }
    v
}

pub fn lexicon_and_zoo() -> Vec<String> {
    let mut l = lexicon_lines();
    l.extend(zoo_lines(0));
    l
}

/// A configuration with `DATUMESH_EMBED_URL` set (through `Config::from_vars`, the env route).
pub fn config_with_embed(url: &str) -> Config {
    let url = url.to_string();
    Config::from_vars(move |k| match k {
        "DATUMESH_HOST" => Some(HOST.to_string()),
        "DATUMESH_EMBED_URL" => Some(url.clone()),
        _ => None,
    })
    .unwrap()
}

/// A mesh on an SQLite store, serving on loopback.
pub struct Served {
    pub base: String,
    pub http: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
    pub lite: Lite,
}

impl Drop for Served {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Served {
    pub async fn raw(&self, path: &str) -> (u16, Vec<u8>) {
        let r = self.http.get(format!("{}{path}", self.base)).send().await.unwrap();
        (r.status().as_u16(), r.bytes().await.unwrap().to_vec())
    }

    /// GET `path`, 200, JSON.
    pub async fn get(&self, path: &str) -> Value {
        let (status, body) = self.raw(path).await;
        assert_eq!(status, 200, "{path}: {}", String::from_utf8_lossy(&body));
        serde_json::from_slice(&body).unwrap_or_else(|e| panic!("{path} is not JSON ({e})"))
    }

    /// `/find?q=<q>` (the query percent-encoded).
    pub async fn find(&self, q: &str) -> Value {
        self.get(&format!("/find?q={}", super::world::qenc(q))).await
    }
}

/// Publishes `lines` (one file, at NOW) into a fresh SQLite store and serves it. `cfg` edits the
/// configuration; `base` is `Config::new(HOST)` unless given.
pub async fn serve_lite(label: &str, lines: &[String], base: Option<Config>, cfg: impl FnOnce(&mut Config)) -> Served {
    let lite = Lite::new(label);
    let f = write(&lite, "fixture.jsonl", lines);
    super::pub031::run(&lite, &f, NOW).unwrap_or_else(|e| panic!("fixture refused: {e}"));
    build_served(lite, base, cfg).await
}

/// Serves an already-filled store (a bundle imported, say).
pub async fn build_served(lite: Lite, base: Option<Config>, cfg: impl FnOnce(&mut Config)) -> Served {
    let mut config = base.unwrap_or_else(|| Config::new(HOST));
    cfg(&mut config);
    let shared: Arc<dyn Store + Send + Sync> = lite.store.clone();
    let mesh = Mesh::builder(config, shared)
        .signing_key(lite.key.clone())
        .clock(Arc::new(FixedClock(MESH_NOW)))
        .build()
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let _ = mesh.serve(listener, std::future::pending::<()>()).await;
    });
    Served { base: format!("http://{addr}"), http: super::client(), task, lite }
}

// ---- reading a /find answer -------------------------------------------------------------------

pub fn results(v: &Value) -> Vec<&Value> {
    v["results"].as_array().unwrap_or_else(|| panic!("no results array in {v}")).iter().collect()
}

pub fn names(v: &Value) -> Vec<String> {
    results(v).iter().map(|r| r["name"].as_str().unwrap_or("").to_string()).collect()
}

pub fn pos(v: &Value, name: &str) -> usize {
    names(v).iter().position(|n| n == name).unwrap_or_else(|| panic!("{name} not in {:?}", names(v)))
}

pub fn result<'a>(v: &'a Value, name: &str) -> &'a Value {
    results(v)[pos(v, name)]
}

/// The `resolution` status of an answer, `none` when the member is absent.
pub fn status(v: &Value) -> String {
    v.get("resolution").and_then(|r| r["status"].as_str()).unwrap_or("none").to_string()
}

/// A call with the spaces after `:` and `,` normalised, so that tests do not depend on the
/// implementation's choice of one space or none.
pub fn norm(call: &str) -> String {
    call.replace(": ", ":").replace(", ", ",")
}

// ---- PostgreSQL with pgvector (DATUMESH_PGVECTOR_URL) -----------------------------------------

pub fn pgvector_url() -> Option<String> {
    std::env::var("DATUMESH_PGVECTOR_URL").ok().filter(|v| !v.is_empty())
}

pub fn out_path(name: &str) -> PathBuf {
    super::out_dir().join(name)
}

// ---- a mesh on its own PostgreSQL database (DATABASE_URL, or DATUMESH_PGVECTOR_URL) -----------

use datumesh_store::pg_unstable::postgres::{Config as PgConfig, NoTls};
use datumesh_store::{migrate, PgStore, PoolOptions};

pub struct PgServed {
    pub served_base: String,
    pub http: reqwest::Client,
    pub store: Arc<PgStore>,
    pub db_url: String,
    name: String,
    admin: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for PgServed {
    fn drop(&mut self) {
        self.task.abort();
        let admin = self.admin.clone();
        let name = self.name.clone();
        super::bl(|| {
            let cfg: PgConfig = admin.parse().unwrap();
            if let Ok(mut c) = cfg.connect(NoTls) {
                let _ = c.batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"));
            }
        });
    }
}

fn db_url(admin: &str, name: &str) -> String {
    let (path, query) = match admin.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (admin, None),
    };
    let (base, _) = path.rsplit_once('/').unwrap();
    match query {
        Some(q) => format!("{base}/{name}?{q}"),
        None => format!("{base}/{name}"),
    }
}

/// A new database on the server `admin` names, migrated; `lines` published at NOW; a mesh serving
/// it with `config` (default `Config::new(HOST)`), clock MESH_NOW.
pub async fn serve_pg(admin: &str, lines: &[String], config: Option<Config>) -> PgServed {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let name = format!("dm_b2_{}_{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let url = db_url(admin, &name);
    let key = super::key_for(HOST);
    let store = super::bl(|| {
        let cfg: PgConfig = admin.parse().unwrap();
        cfg.connect(NoTls).unwrap().batch_execute(&format!("CREATE DATABASE {name}")).unwrap();
        let store = PgStore::connect_with(&url, PoolOptions::new().max_size(8)).unwrap();
        migrate::apply(&store).unwrap();
        store.put_namespace(HOST, &super::key_doc(HOST, &key), K_FROM).unwrap();
        store
    });
    let store = Arc::new(store);
    let lite_dir = out_path(&format!("pg-{name}"));
    std::fs::create_dir_all(&lite_dir).unwrap();
    let f = lite_dir.join("fixture.jsonl");
    std::fs::write(&f, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    super::bl(|| {
        datumesh_http::publish::publish_stored_with(
            &*store,
            &key,
            HOST,
            &[HOST.to_string()],
            &f,
            &super::pub031::opts(NOW),
        )
        .unwrap_or_else(|e| panic!("fixture refused: {e}"));
    });
    let _ = std::fs::remove_dir_all(&lite_dir);
    let shared: Arc<dyn Store + Send + Sync> = store.clone();
    let mesh = super::bl(|| {
        Mesh::builder(config.unwrap_or_else(|| Config::new(HOST)), shared)
            .signing_key(key)
            .clock(Arc::new(FixedClock(MESH_NOW)))
            .build()
            .unwrap()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let _ = mesh.serve(listener, std::future::pending::<()>()).await;
    });
    PgServed {
        served_base: format!("http://{addr}"),
        http: super::client(),
        store,
        db_url: url,
        name,
        admin: admin.to_string(),
        task,
    }
}

impl PgServed {
    pub async fn get(&self, path: &str) -> Value {
        let r = self.http.get(format!("{}{path}", self.served_base)).send().await.unwrap();
        let status = r.status().as_u16();
        let body = r.bytes().await.unwrap();
        assert_eq!(status, 200, "{path}: {}", String::from_utf8_lossy(&body));
        serde_json::from_slice(&body).unwrap()
    }

    pub async fn find(&self, q: &str) -> Value {
        self.get(&format!("/find?q={}", super::world::qenc(q))).await
    }

    /// Asks `/find?q=` until `ok` holds of the answer (the index is built in the background after
    /// start), for up to 20 s; panics with the last answer.
    pub async fn find_until(&self, q: &str, ok: impl Fn(&Value) -> bool) -> Value {
        let mut last = Value::Null;
        for _ in 0..80 {
            last = self.find(q).await;
            if ok(&last) {
                return last;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        panic!("find {q:?} never reached the wanted state; last answer: {last}");
    }

    /// Rows of a query on the database (first column of each row as text).
    pub fn column(&self, sql: &str) -> Vec<String> {
        let (url, sql) = (self.db_url.clone(), sql.to_string());
        super::bl(move || {
            let cfg: PgConfig = url.parse().unwrap();
            let mut c = cfg.connect(NoTls).unwrap();
            c.query(sql.as_str(), &[]).unwrap().iter().map(|r| r.get::<_, String>(0)).collect()
        })
    }
}
