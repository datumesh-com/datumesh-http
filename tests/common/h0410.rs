//! Helpers of the 0.4.10 tests (CONTRACT-round-0.7-P HP1-HP8).
//!
//! # Why a read probe in the database, not a counting `Store` wrapper
//!
//! `datumesh_store::Store` is sealed (store 0.3.0 on: a private supertrait), so no test can wrap a store and
//! count its calls. The tests count in the database instead. A [`Fx`] on PostgreSQL connects the mesh as a
//! role that does not own the tables; [`Probe::arm`] then turns on row-level security on the tables of a
//! group, with a `SELECT` policy that draws one number from a sequence for every row a statement visits.
//! [`Probe::reads`] is the count of rows visited since the arm: the same figure as "calls that read a cell",
//! scaled by what each call reads, and exact (no statistics lag). Writes are not counted. The probe is armed
//! after the fixture is published, so the fixture's own reads are not in it.
//!
//! Where only the answer matters (what the index holds, the caps, the JSON of a page) the tests use SQLite
//! ([`Fx::lite`]), which needs no database.
//!
//! Needs `#[path = "common/calculus.rs"] mod calculus; mod common; #[path = "common/publish4.rs"] mod publish4;`
//! at the crate root when the test also uses `calculus`; this module itself needs only `mod common;`.
#![allow(dead_code)]

use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use datumesh_core::{Cell, KeyValue, ObjectKey, ReceiptBuilder, Scalar, SigningKey, Type};
use datumesh_http::publish::publish_stored_with;
use datumesh_http::{Config, Mesh};
use datumesh_store::pg_unstable::postgres::{Client, Config as PgConfig, NoTls};
use datumesh_store::{PgStore, PoolOptions, Store};
use serde_json::{json, Value};

use crate::common::lite::*;
use crate::common::pub031::*;
use crate::common::readpath::MESH_NOW;
use crate::common::world::NONE_KEYS_PAIR;
use crate::common::{key_doc, key_for, on_thread, TestDb};

// ---- configuration ------------------------------------------------------------------------------------

/// A configuration for [`HOST`] with no rate limits, the given variables, and the given aliases
/// (`name -> target`; an alias to `<class>/<method>` makes the object index hear of the class).
pub fn config(vars: &[(&str, &str)], aliases: &[(&str, &str)]) -> Config {
    let mut env: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    env.push(("DATUMESH_HOST".into(), HOST.into()));
    for k in ["DATUMESH_RATE_PER_IP", "DATUMESH_RATE_IP_BURST", "DATUMESH_RATE_SERIES_PER_IP"] {
        env.push((k.into(), "1000000".into()));
    }
    let mut c = Config::from_vars(move |k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).unwrap();
    for (a, t) in aliases {
        c.aliases.insert(a.to_string(), t.to_string());
    }
    c
}

/// The three background builds held back for an hour, so that nothing but the request under test reads the
/// store (the index, the find thread, the series and takeover thread). Pass the names to free.
pub fn held(free: &[&str]) -> Vec<(&'static str, &'static str)> {
    let mut v = Vec::new();
    for (name, var) in [
        ("index", "DATUMESH_INDEX_FIRST_BUILD_DELAY_SECS"),
        ("find", "DATUMESH_FIND_FIRST_BUILD_DELAY_SECS"),
        ("series", "DATUMESH_SERIES_FIRST_BUILD_DELAY_SECS"),
    ] {
        if !free.contains(&name) {
            v.push((var, "3600"));
        }
    }
    v
}

// ---- the read probe -----------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub enum Group {
    /// `cells`, `cell_args`, `versions`, `computed_versions`: every way to read a cell.
    Cells,
    /// `function_tests`: the stored test rows.
    Tests,
    /// `catalog_entries`.
    Catalog,
    /// `key_values`: the dictionary values.
    KeyValues,
}

impl Group {
    fn seq(self) -> &'static str {
        match self {
            Group::Cells => "probe_cells",
            Group::Tests => "probe_tests",
            Group::Catalog => "probe_catalog",
            Group::KeyValues => "probe_key_values",
        }
    }

    fn tables(self) -> &'static [&'static str] {
        match self {
            Group::Cells => &["cells", "cell_args", "versions", "computed_versions"],
            Group::Tests => &["function_tests"],
            Group::Catalog => &["catalog_entries"],
            Group::KeyValues => &["key_values"],
        }
    }
}

const ALL: [Group; 4] = [Group::Cells, Group::Tests, Group::Catalog, Group::KeyValues];

pub struct Probe {
    pub db_url: String,
    pub role: String,
}

fn admin_client(url: &str) -> Client {
    let cfg: PgConfig = url.parse().unwrap();
    cfg.connect(NoTls).unwrap()
}

impl Probe {
    /// Row-level security on the tables of every group, each row a statement visits counted.
    pub fn arm(&self) {
        let (url, role) = (self.db_url.clone(), self.role.clone());
        on_thread(move || {
            let mut c = admin_client(&url);
            // statistics as production's autovacuum keeps them: a freshly loaded fixture has none, and the
            // planner then chooses plans production does not
            c.batch_execute("ANALYZE").unwrap();
            for g in ALL {
                c.batch_execute(&format!("CREATE SEQUENCE IF NOT EXISTS {}", g.seq())).unwrap();
            }
            c.batch_execute(&format!("GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO \"{role}\""))
                .unwrap();
            for g in ALL {
                for t in g.tables() {
                    c.batch_execute(&format!(
                        "ALTER TABLE {t} ENABLE ROW LEVEL SECURITY;\
                         CREATE POLICY probe_sel ON {t} FOR SELECT USING (nextval('{}') > 0);\
                         CREATE POLICY probe_ins ON {t} FOR INSERT WITH CHECK (true);\
                         CREATE POLICY probe_upd ON {t} FOR UPDATE USING (true) WITH CHECK (true);\
                         CREATE POLICY probe_del ON {t} FOR DELETE USING (true)",
                        g.seq()
                    ))
                    .unwrap_or_else(|e| panic!("arming {t}: {e}"));
                }
            }
        });
    }

    /// Rows of the group's tables visited since [`Probe::arm`].
    pub fn reads(&self, g: Group) -> u64 {
        let url = self.db_url.clone();
        let seq = g.seq();
        on_thread(move || {
            let mut c = admin_client(&url);
            let row = c
                .query_one(&format!("SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM {seq}"), &[])
                .unwrap_or_else(|e| panic!("the probe is not armed: {e}"));
            row.get::<_, i64>(0) as u64
        })
    }
}

/// Removes the role of a probe database after the database itself is dropped.
struct RoleGuard {
    role: String,
    db_name: String,
}

impl Drop for RoleGuard {
    fn drop(&mut self) {
        let (role, db) = (self.role.clone(), self.db_name.clone());
        let _ = std::thread::spawn(move || {
            if let Ok(admin) = std::env::var("DATABASE_URL") {
                if let Ok(mut c) = admin.parse::<PgConfig>().unwrap().connect(NoTls) {
                    let _ = c.batch_execute(&format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)"));
                    let _ = c.batch_execute(&format!("DROP ROLE IF EXISTS \"{role}\""));
                }
            }
        })
        .join();
    }
}

// ---- a world to publish into and serve --------------------------------------------------------------

/// A store with the key of [`HOST`], a directory for files, and (on PostgreSQL) a probe.
pub struct Fx {
    pub store: Arc<dyn Store + Send + Sync>,
    pub key: SigningKey,
    pub dir: PathBuf,
    pub probe: Option<Probe>,
    _keep: Vec<Box<dyn Any + Send>>,
}

impl Fx {
    /// On SQLite: no probe.
    pub fn lite(label: &str) -> Fx {
        let lite = Lite::new(label);
        Fx {
            store: lite.store.clone(),
            key: lite.key.clone(),
            dir: lite.dir.clone(),
            probe: None,
            _keep: vec![Box::new(lite)],
        }
    }

    /// On PostgreSQL (`DATABASE_URL`), as a role that owns nothing, with a probe (not yet armed).
    pub fn pg(label: &str) -> Fx {
        let db = on_thread(TestDb::migrated);
        let role = format!("r_{}", db.name);
        let (db_url, db_name) = (db.url(), db.name.clone());
        {
            let (role, db_url, db_name) = (role.clone(), db_url.clone(), db_name.clone());
            on_thread(move || {
                let admin = std::env::var("DATABASE_URL").unwrap();
                admin_client(&admin)
                    .batch_execute(&format!(
                        "CREATE ROLE \"{role}\" LOGIN PASSWORD 'probe' NOSUPERUSER;\
                         GRANT CONNECT ON DATABASE {db_name} TO \"{role}\""
                    ))
                    .unwrap();
                admin_client(&db_url)
                    .batch_execute(&format!(
                        "GRANT USAGE ON SCHEMA public TO \"{role}\";\
                         GRANT ALL ON ALL TABLES IN SCHEMA public TO \"{role}\";\
                         GRANT ALL ON ALL SEQUENCES IN SCHEMA public TO \"{role}\""
                    ))
                    .unwrap();
            });
        }
        // postgres://user:pass@host/db -> postgres://role:probe@host/db
        let role_url = {
            let (scheme, rest) = db_url.split_once("://").unwrap();
            let (_, after) = rest.split_once('@').unwrap();
            format!("{scheme}://{role}:probe@{after}")
        };
        let store: Arc<PgStore> =
            on_thread(move || Arc::new(PgStore::connect_with(&role_url, PoolOptions::new().max_size(8)).unwrap()));
        let key = key_for(HOST);
        {
            let (store, key) = (store.clone(), key.clone());
            on_thread(move || {
                store
                    .put_namespace(HOST, &datumesh_http::keydoc::with_key(HOST, None, &key, K_FROM).unwrap(), K_FROM)
                    .unwrap();
            });
        }
        let dir = crate::common::out_dir().join(format!("h0410-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let guard = RoleGuard { role: role.clone(), db_name };
        Fx {
            store,
            key,
            dir,
            probe: Some(Probe { db_url: admin_db_url(&db.url()), role }),
            // dropped in this order: the store's connections go with `store`, then the database and the role
            _keep: vec![Box::new(guard), Box::new(db)],
        }
    }

    pub fn probe(&self) -> &Probe {
        self.probe.as_ref().expect("a PostgreSQL fixture")
    }

    /// Writes `lines` as a file and publishes it at clock `now` (any thread: PostgreSQL's client is
    /// synchronous).
    pub fn publish(&self, lines: &[String], now: &'static str) {
        self.try_publish(lines, now).unwrap_or_else(|e| panic!("{e}"));
    }

    pub fn try_publish(&self, lines: &[String], now: &'static str) -> Result<(), String> {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = self.dir.join(format!("f{n}.jsonl"));
        std::fs::write(&path, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
        let (store, key) = (self.store.clone(), self.key.clone());
        on_thread(move || {
            let owned = vec![HOST.to_string()];
            publish_stored_with(&*store, &key, HOST, &owned, &path, &opts(now)).map(|_| ()).map_err(|e| e.to_string())
        })
    }

    /// The hash `name` is bound to now.
    pub fn hash_of(&self, name: &str) -> String {
        let (store, name) = (self.store.clone(), name.to_string());
        on_thread(move || {
            store.resolve_binding(&name, END).unwrap().unwrap_or_else(|| panic!("{name} unbound")).fn_hash
        })
    }

    pub fn class_hash(&self, class: &str) -> String {
        let (store, class) = (self.store.clone(), class.to_string());
        on_thread(move || store.resolve_class(&class, END).unwrap().unwrap_or_else(|| panic!("{class} unbound")))
    }

    /// Serves the store on loopback with the real router.
    pub async fn serve(self, cfg: Config) -> W {
        let (store, key) = (self.store.clone(), self.key.clone());
        let mesh = on_thread(move || {
            Mesh::builder(cfg, store).signing_key(key).clock(Arc::new(FixedClock(MESH_NOW))).build().unwrap()
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let m = mesh.clone();
        let task = tokio::spawn(async move {
            let _ = m.serve(listener, std::future::pending::<()>()).await;
        });
        W { base: format!("http://{addr}"), http: crate::common::client(), mesh, task, fx: self }
    }
}

fn admin_db_url(db_url: &str) -> String {
    db_url.to_string()
}

pub struct W {
    pub base: String,
    pub http: reqwest::Client,
    pub mesh: Mesh,
    task: tokio::task::JoinHandle<()>,
    pub fx: Fx,
}

impl Drop for W {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub struct Rep {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: Vec<u8>,
}

impl Rep {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.text()))
    }
    pub fn header(&self, k: &str) -> Option<String> {
        self.headers.get(k).and_then(|v| v.to_str().ok()).map(str::to_string)
    }
}

impl W {
    pub async fn get(&self, path: &str) -> Rep {
        self.get_h(path, &[]).await
    }

    pub async fn get_h(&self, path: &str, headers: &[(&str, &str)]) -> Rep {
        let mut rb = self.http.get(format!("{}{path}", self.base));
        for (k, v) in headers {
            rb = rb.header(*k, *v);
        }
        let r = rb.send().await.unwrap_or_else(|e| panic!("{path}: no answer ({e})"));
        let (status, headers) = (r.status().as_u16(), r.headers().clone());
        let body = r.bytes().await.unwrap().to_vec();
        Rep { status, headers, body }
    }

    pub async fn post(&self, path: &str, body: &Value) -> Rep {
        let r = self
            .http
            .post(format!("{}{path}", self.base))
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        let (status, headers) = (r.status().as_u16(), r.headers().clone());
        let body = r.bytes().await.unwrap().to_vec();
        Rep { status, headers, body }
    }

    /// `Mesh::describe(id)` called in process (two calls in a row are microseconds apart, which a request
    /// pair over HTTP is not): status and body.
    pub fn describe_now(&self, id: &str) -> (u16, String) {
        let (m, id) = (self.mesh.clone(), id.to_string());
        on_thread(move || {
            let r = m.describe(&id);
            (r.status, String::from_utf8_lossy(&r.body).into_owned())
        })
    }

    /// Polls `GET /describe/<id>` until it is neither 503 nor 404 (60 s at most) and returns the last reply.
    pub async fn describe_until_found(&self, id: &str) -> Rep {
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            let r = self.get(&format!("/describe/{id}")).await;
            if (r.status != 503 && r.status != 404) || Instant::now() > end {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn metric(&self, name: &str) -> Option<f64> {
        let r = self.get("/metrics").await;
        let mut total: Option<f64> = None;
        for line in r.text().lines() {
            if line.starts_with('#') || !line.starts_with(name) {
                continue;
            }
            if let Some(v) = line.rsplit(' ').next().and_then(|v| v.parse::<f64>().ok()) {
                *total.get_or_insert(0.0) += v;
            }
        }
        total
    }
}

// ---- classes ------------------------------------------------------------------------------------------

/// What a method of a test class is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Keys only, boolean: the class's `extent.keys` function.
    Extent,
    /// Keys and a date, number.
    Rate,
    /// Keys only, string.
    Note,
    /// Keys only, number (with `Note`: a second method, a different hash).
    Level,
}

fn str_arg() -> &'static str {
    r#"{"type":"string","valid":false}"#
}

pub fn sig_of(k: Kind) -> String {
    match k {
        Kind::Extent => format!(
            r#"{{"args":{{"base":{s},"quote":{s}}},"order":["base","quote"],"returns":"boolean"}}"#,
            s = str_arg()
        ),
        Kind::Rate => format!(
            r#"{{"args":{{"base":{s},"date":{{"type":"date","valid":true}},"quote":{s}}},"order":["base","quote","date"],"returns":{{"type":"number","unit":"1"}}}}"#,
            s = str_arg()
        ),
        Kind::Note => format!(
            r#"{{"args":{{"base":{s},"quote":{s}}},"order":["base","quote"],"returns":"string"}}"#,
            s = str_arg()
        ),
        Kind::Level => format!(
            r#"{{"args":{{"base":{s},"quote":{s}}},"order":["base","quote"],"returns":{{"type":"number","unit":"1"}}}}"#,
            s = str_arg()
        ),
    }
}

/// A stored cell of a method `fn_name` (the full name), for the object (`base`, `quote`).
pub fn method_cell(fn_name: &str, k: Kind, base: &str, quote: &str) -> String {
    let mut args = json!({"base": {"type": "string", "value": base}, "quote": {"type": "string", "value": quote}});
    let value = match k {
        Kind::Extent => json!({"type": "boolean", "value": true}),
        Kind::Rate => {
            args["date"] = json!({"type": "date", "value": "2026-03-02"});
            json!({"type": "number", "value": "1.5"})
        }
        Kind::Note => json!({"type": "string", "value": "n"}),
        Kind::Level => json!({"type": "number", "value": "2"}),
    };
    json!({"cell": {"fn": fn_name, "args": args}, "licence": "CC0-1.0", "summary": "Synthetic test data.", "value": value})
        .to_string()
}

pub struct Cls {
    pub name: String,
    pub hash: String,
    /// method name -> function hash
    pub methods: Vec<(String, String)>,
}

impl Cls {
    pub fn method_hash(&self, m: &str) -> &str {
        &self.methods.iter().find(|(n, _)| n == m).unwrap().1
    }

    /// The object id of (`base`, `quote`).
    pub fn id(&self, base: &str, quote: &str) -> String {
        object_id(&self.name, &self.hash, base, quote)
    }

    pub fn encoding(&self, base: &str, quote: &str) -> String {
        format!(r#"{}(base:"{base}",quote:"{quote}")"#, self.name)
    }

    pub fn cell_id(&self, method: &str, kind: Kind, base: &str, quote: &str) -> String {
        let mut a = crate::common::args(&[("base", crate::common::s(base)), ("quote", crate::common::s(quote))]);
        if kind == Kind::Rate {
            a.insert("date".into(), datumesh_core::Arg::Value(Scalar::from_lexical(Type::Date, "2026-03-02").unwrap()));
        }
        Cell { ns: self.name.clone(), fn_name: method.into(), fn_hash: self.method_hash(method).into(), args: a }.id()
    }
}

pub fn object_id(class: &str, class_hash: &str, base: &str, quote: &str) -> String {
    let key = ObjectKey::new(
        class,
        [
            ("base".to_string(), KeyValue::Scalar(Scalar::from_lexical(Type::String, base).unwrap())),
            ("quote".to_string(), KeyValue::Scalar(Scalar::from_lexical(Type::String, quote).unwrap())),
        ],
    )
    .unwrap();
    key.object_id(class_hash).unwrap()
}

/// Defines the class `class` over keys (`base`, `quote`) with the methods `(name, kind)`; the one of kind
/// `Extent`, if any, is its `extent.keys`. Cells are not written.
pub fn define_class(fx: &Fx, class: &str, methods: &[(&str, Kind)]) -> Cls {
    define_class_with(fx, class, methods, &[])
}

/// A `formula` line: a computed function with the given source and signature.
pub fn formula_of(name: &str, source: &str, signature: &str) -> String {
    json!({"formula": name, "source": source, "signature": serde_json::from_str::<Value>(signature).unwrap()})
        .to_string()
}

/// As [`define_class`], with more methods that are computed functions already published under the names
/// `(method, full function name)`.
pub fn define_class_with(fx: &Fx, class: &str, methods: &[(&str, Kind)], formulas: &[(&str, &str)]) -> Cls {
    let sigs: Vec<(String, Kind, String)> = methods
        .iter()
        .map(|(m, k)| {
            let d = stored_descriptor(&sig_of(*k));
            (m.to_string(), *k, d)
        })
        .collect();
    let f1: Vec<String> = sigs.iter().map(|(_, _, d)| descriptor_line(d)).collect();
    // the descriptors alone are not a publishable file in every version: ignore the verdict, store them
    let _ = fx.try_publish(&f1, NOW);
    let store = fx.store.clone();
    let canon: Vec<Vec<u8>> = sigs.iter().map(|(_, _, d)| descriptor_canonical(d)).collect();
    on_thread(move || {
        for c in canon {
            store.put_descriptor(&c).unwrap();
        }
    });
    let mut hashes: Vec<(String, String)> = sigs.iter().map(|(m, _, d)| (m.clone(), descriptor_hash(d))).collect();
    for (m, full) in formulas {
        hashes.push((m.to_string(), fx.hash_of(full)));
    }
    hashes.sort();
    let extent = sigs.iter().find(|(_, k, _)| *k == Kind::Extent).map(|(m, _, _)| m.clone());
    let extra = match &extent {
        Some(m) => format!(r#""extent":{{"keys":"{}"}},"#, hashes.iter().find(|(n, _)| n == m).unwrap().1),
        None => String::new(),
    };
    let refs: Vec<(&str, &str)> = hashes.iter().map(|(m, h)| (m.as_str(), h.as_str())).collect();
    let desc = crate::common::world::class_json(NONE_KEYS_PAIR, &["base", "quote"], &refs, HOST, &extra);
    fx.publish(&[class_line(class, &desc, None)], T1);
    Cls { name: class.to_string(), hash: fx.class_hash(class), methods: hashes }
}

/// Stored cells of the method `m` of `cls` for the objects `<prefix><i>` (`i` in `range`), quote `Q`.
pub fn cells_of(cls: &Cls, m: &str, k: Kind, prefix: &str, range: std::ops::Range<usize>) -> Vec<String> {
    range.map(|i| method_cell(&format!("{}/{m}", cls.name), k, &format!("{prefix}{i:05}"), "Q")).collect()
}

/// A signed stored version of the extent cell (`base`, `Q`) written straight into the store (no publish, so
/// nothing but the write is read), at `tx`; returns its cell id.
pub fn put_extent_version(fx: &Fx, cls: &Cls, m: &str, base: &str, tx: &'static str) -> String {
    let (name, hash) = (cls.name.clone(), cls.method_hash(m).to_string());
    let args = crate::common::args(&[("base", crate::common::s(base)), ("quote", crate::common::s("Q"))]);
    let cell = Cell { ns: name, fn_name: m.to_string(), fn_hash: hash, args };
    let id = cell.id();
    let r = ReceiptBuilder::new()
        .cell(cell)
        .value(Scalar::from_lexical(Type::Boolean, "true").unwrap())
        .valid(crate::common::args(&[]))
        .asof(tx)
        .tx(tx)
        .issued(tx)
        .publisher(&cls.name)
        .signer_host(HOST)
        .licence("CC0-1.0")
        .summary("Synthetic test data.")
        .sign(&fx.key)
        .unwrap();
    let bytes = crate::common::receipt_json(&r);
    let (store, id2, hash) = (fx.store.clone(), id.clone(), r.hash().to_string());
    on_thread(move || {
        store.put_receipt(&bytes).unwrap();
        store.put_version(&id2, tx, &hash).unwrap();
    });
    id
}

/// Whether `text` gives the reason of a hex id the last build of the index does not hold.
pub fn says_not_in_index(text: &str) -> bool {
    text.to_lowercase().contains("not in the object index yet")
}

pub fn key_doc_of(host: &str) -> Vec<u8> {
    key_doc(host, &key_for(host))
}
