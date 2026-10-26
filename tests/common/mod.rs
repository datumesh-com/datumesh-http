//! Helpers shared by the integration tests (`mod common;`).
//!
//! Every [`Node`] is a mesh on a PostgreSQL database of its own (created from `DATABASE_URL`,
//! a role allowed to create databases, and dropped afterwards), served on 127.0.0.1 on a
//! port of its own, with the real router: tests talk HTTP to it.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use datumesh_core::json::{self, Value};
use datumesh_core::{
    check_envelope_any, Arg, Args, Cell, FunctionDescriptor, Keys, Receipt, ReceiptBuilder, Request, Scalar,
    SigningKey, Type,
};
use datumesh_engine::{Budget, Clock, EvalError, Inputs, Runtime, SystemClock};
use datumesh_http::{Config, Mesh, MeshBuilder};
use datumesh_store::pg_unstable::postgres::{Config as PgConfig, NoTls};
use datumesh_store::{migrate, PgStore, PoolOptions, Store};

pub use datumesh_core::Resolver;
pub use datumesh_store::Store as _;

pub const BIND_TX: &str = "2020-01-01T00:00:00.000000Z";
pub const KEY_FROM: &str = "2000-01-01T00:00:00.000000Z";

/// Runs blocking store work from an async test (the store's driver may not run on a
/// runtime thread).
pub fn bl<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(_) => tokio::task::block_in_place(f),
        Err(_) => f(),
    }
}

pub fn admin_url() -> String {
    std::env::var("DATABASE_URL").expect("DATABASE_URL names a PostgreSQL 16 database (see README)")
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A database of its own, migrated, dropped when the value is dropped.
pub struct TestDb {
    pub name: String,
    admin: String,
}

impl TestDb {
    pub fn migrated() -> TestDb {
        bl(TestDb::migrated_now)
    }

    fn migrated_now() -> TestDb {
        let admin = admin_url();
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
        let name = format!("dm_http_{}_{}_{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst), nanos);
        let cfg: PgConfig = admin.parse().unwrap();
        let mut c = cfg.connect(NoTls).unwrap();
        c.batch_execute(&format!("CREATE DATABASE {name}")).unwrap();
        let db = TestDb { name, admin };
        migrate::apply(&db.store(4)).unwrap();
        db
    }

    pub fn url(&self) -> String {
        let (path, query) = match self.admin.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (self.admin.as_str(), None),
        };
        let (base, _) = path.rsplit_once('/').unwrap();
        match query {
            Some(q) => format!("{base}/{}?{q}", self.name),
            None => format!("{base}/{}", self.name),
        }
    }

    pub fn store(&self, pool: usize) -> PgStore {
        PgStore::connect_with(&self.url(), PoolOptions::new().max_size(pool)).unwrap()
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        bl(|| {
            let cfg: PgConfig = self.admin.parse().unwrap();
            if let Ok(mut c) = cfg.connect(NoTls) {
                let _ = c.batch_execute(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name));
            }
        })
    }
}

/// The test key of a host.
pub fn key_for(host: &str) -> SigningKey {
    SigningKey::from_seed(&datumesh_core::sha256(format!("datumesh-http test key: {host}").as_bytes()))
}

/// The key document of `host` publishing `key` from 2000 on.
pub fn key_doc(host: &str, key: &SigningKey) -> Vec<u8> {
    datumesh_http::keydoc::with_key(host, None, key, KEY_FROM).unwrap()
}

pub fn now() -> String {
    SystemClock.now()
}

/// `t` plus `us` microseconds.
pub fn plus(t: &str, us: i64) -> String {
    let m = datumesh_formula::calendar::parse_instant(t).unwrap();
    datumesh_formula::calendar::instant_text(m + us).unwrap()
}

pub fn s(v: &str) -> Scalar {
    Scalar::from_lexical(Type::String, v).unwrap()
}
pub fn n(v: &str) -> Scalar {
    Scalar::from_lexical(Type::Number, v).unwrap()
}
pub fn d(v: &str) -> Scalar {
    Scalar::from_lexical(Type::Date, v).unwrap()
}

pub fn args(a: &[(&str, Scalar)]) -> Args {
    a.iter().map(|(k, v)| (k.to_string(), Arg::Value(v.clone()))).collect()
}

/// A test runtime: `lang = "sleep"`, the body a number of milliseconds to sleep; the value is
/// the cell's `tag` argument (a string).
pub struct Sleep;

impl Runtime for Sleep {
    fn lang(&self) -> &str {
        "sleep"
    }
    fn evaluate(
        &self,
        d: &FunctionDescriptor,
        cell: &Cell,
        _inputs: &mut dyn Inputs,
        budget: &mut Budget,
    ) -> Result<Scalar, EvalError> {
        budget.charge(1)?;
        let ms: u64 = d.body.trim().parse().map_err(|_| EvalError::Runtime("body".into()))?;
        std::thread::sleep(Duration::from_millis(ms));
        match cell.args.get("tag") {
            Some(Arg::Value(v)) => Ok(v.clone()),
            _ => Err(EvalError::Runtime("tag".into())),
        }
    }
}

/// A response.
pub struct Resp {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn json(&self) -> Value {
        json::parse(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&self.body)))
    }
    pub fn outcome(&self) -> String {
        self.json().get("outcome").and_then(Value::as_str).unwrap_or("").to_string()
    }
    pub fn header(&self, k: &str) -> Option<String> {
        self.headers.get(k).and_then(|v| v.to_str().ok()).map(str::to_string)
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().pool_max_idle_per_host(2048).build().unwrap()
}

/// A mesh served on 127.0.0.1.
pub struct Node {
    pub host: String,
    pub key: SigningKey,
    pub db: Arc<TestDb>,
    pub store: Arc<PgStore>,
    pub mesh: Mesh,
    pub base: String,
    pub addr: SocketAddr,
    pub http: reqwest::Client,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

pub struct Setup {
    pub host: String,
    pub key: SigningKey,
    pub listen: String,
    pub db: Option<Arc<TestDb>>,
}

impl Setup {
    pub fn new(host: &str) -> Setup {
        Setup { host: host.to_string(), key: key_for(host), listen: "127.0.0.1:0".into(), db: None }
    }
}

impl Node {
    /// A mesh for `host` with the default configuration changed by `cfg`.
    pub async fn start(host: &str, cfg: impl FnOnce(&mut Config)) -> Node {
        Node::start_with(Setup::new(host), cfg, |b| b).await
    }

    pub async fn start_with(
        setup: Setup,
        cfg: impl FnOnce(&mut Config),
        build: impl FnOnce(MeshBuilder) -> MeshBuilder,
    ) -> Node {
        let host = setup.host.clone();
        let db = match setup.db {
            Some(db) => db,
            None => Arc::new(TestDb::migrated()),
        };
        let store = Arc::new(bl(|| db.store(40)));
        bl(|| {
            if store.namespace(&host).unwrap().is_none() {
                store.put_namespace(&host, &key_doc(&host, &setup.key), BIND_TX).unwrap();
            }
        });
        let mut config = Config::new(&host);
        cfg(&mut config);
        let shared: Arc<dyn Store + Send + Sync> = store.clone();
        let b = Mesh::builder(config, shared).signing_key(setup.key.clone()).runtime(Arc::new(Sleep));
        let mesh = bl(|| build(b).build().unwrap());
        let listener = tokio::net::TcpListener::bind(&setup.listen).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let m = mesh.clone();
        let task = tokio::spawn(async move {
            let _ = m
                .serve(listener, async move {
                    let _ = rx.await;
                })
                .await;
        });
        Node {
            host,
            key: setup.key,
            db,
            store,
            mesh,
            base: format!("http://{addr}"),
            addr,
            http: client(),
            stop: Some(tx),
            task: Some(task),
        }
    }

    /// Stops serving (the database stays).
    pub async fn stop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.task.take() {
            t.abort();
            let _ = t.await;
        }
    }

    pub async fn get(&self, path: &str) -> Resp {
        self.get_with(path, &[]).await
    }

    pub async fn get_with(&self, path: &str, headers: &[(&str, &str)]) -> Resp {
        let mut rb = self.http.get(format!("{}{path}", self.base));
        for (k, v) in headers {
            rb = rb.header(*k, *v);
        }
        let r = rb.send().await.unwrap();
        let status = r.status().as_u16();
        let headers = r.headers().clone();
        let body = r.bytes().await.unwrap().to_vec();
        Resp { status, headers, body }
    }

    pub async fn post(&self, path: &str, body: Vec<u8>) -> Resp {
        let r = self
            .http
            .post(format!("{}{path}", self.base))
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
        let status = r.status().as_u16();
        let headers = r.headers().clone();
        let body = r.bytes().await.unwrap().to_vec();
        Resp { status, headers, body }
    }

    /// A stored function under this mesh's host, bound since BIND_TX.
    pub fn stored(&self, name: &str, signature: &str) -> String {
        bl(|| {
            let d = format!(r#"{{"code":{{"body":"","lang":"stored"}},"inputs":[],"signature":{signature}}}"#);
            let h = self.store.put_descriptor(d.as_bytes()).unwrap();
            self.store.bind_name(name, &h, BIND_TX).unwrap();
            h
        })
    }

    /// A function of another language bound since BIND_TX.
    pub fn function(&self, name: &str, lang: &str, body: &str, inputs: &[&str], signature: &str) -> String {
        bl(|| {
            let ins: Vec<String> = inputs.iter().map(|i| format!("\"{i}\"")).collect();
            let d = format!(
                r#"{{"code":{{"body":{},"lang":"{lang}"}},"inputs":[{}],"signature":{signature}}}"#,
                Value::String(body.into()).to_jcs().iter().map(|b| *b as char).collect::<String>(),
                ins.join(",")
            );
            let h = self.store.put_descriptor(d.as_bytes()).unwrap();
            self.store.bind_name(name, &h, BIND_TX).unwrap();
            h
        })
    }

    /// A formula function defined against the store and published (bound) at BIND_TX.
    pub fn formula(&self, name: &str, source: &str, signature: &str) -> String {
        bl(|| {
            let sig = json::parse(signature.as_bytes()).unwrap();
            let s: &PgStore = &self.store;
            let def =
                datumesh_formula::definition::define_function(name, source, &sig, &Default::default(), s, BIND_TX)
                    .unwrap_or_else(|e| panic!("define {name}: {e:?}"));
            datumesh_formula::definition::publish(s, &def, BIND_TX).unwrap()
        })
    }

    /// A stored version of `name`'s cell with `args`, signed by `key` (default this mesh's)
    /// as publisher `host_of(name)`, at `tx` (asof = issued = tx). Returns the receipt hash.
    pub fn put_value_signed(&self, key: &SigningKey, name: &str, a: &Args, value: Scalar, tx: &str) -> String {
        bl(|| {
            let (ns, f) = name.rsplit_once('/').unwrap();
            let fn_hash = self.store.resolve_name(name, tx).unwrap().expect("bound");
            let cell = Cell { ns: ns.into(), fn_name: f.into(), fn_hash, args: a.clone() };
            let host = ns.split('/').next().unwrap();
            let d = FunctionDescriptor::from_json(&self.store.descriptor(&cell.fn_hash).unwrap()).unwrap();
            let valid: Args = a
                .iter()
                .filter(|(k, _)| d.signature.args.get(*k).is_some_and(|x| x.valid))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let r = ReceiptBuilder::new()
                .cell(cell.clone())
                .value(value)
                .valid(valid)
                .asof(tx)
                .tx(tx)
                .issued(tx)
                .publisher(ns)
                .signer_host(host)
                .licence("CC-BY-4.0")
                .summary("A test value.")
                .sign(key)
                .unwrap();
            let bytes = receipt_json(&r);
            let h = self.store.put_receipt(&bytes).unwrap();
            self.store.put_version(&cell.id(), tx, &h).unwrap();
            self.mesh.stored_version_added(&cell.id());
            h
        })
    }

    pub fn put_value(&self, name: &str, a: &Args, value: Scalar, tx: &str) -> String {
        let k = self.key.clone();
        self.put_value_signed(&k, name, a, value, tx)
    }

    /// The cell id of `name`'s cell with `args` under its binding now.
    pub fn cell_id(&self, name: &str, a: &Args) -> String {
        bl(|| {
            let (ns, f) = name.rsplit_once('/').unwrap();
            let fn_hash = self.store.resolve_name(name, &now()).unwrap().expect("bound");
            Cell { ns: ns.into(), fn_name: f.into(), fn_hash, args: a.clone() }.id()
        })
    }

    /// Keys of this mesh's host.
    pub fn keys(&self) -> Keys {
        let mut k = Keys::new();
        k.insert_response(&self.host, 200, Some(&key_doc(&self.host, &self.key)));
        k
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        // The store's synchronous driver cannot be dropped on a runtime thread: it is kept
        // (its connections end when the test's database is dropped, WITH (FORCE)).
        std::mem::forget(self.store.clone());
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

/// A receipt's JSON (payload members and `sig`).
pub fn receipt_json(r: &Receipt) -> Vec<u8> {
    let mut members = vec![("sig".to_string(), Value::String(r.sig().to_string()))];
    members.extend(json::parse(r.payload()).unwrap().as_object().unwrap().iter().cloned());
    Value::Object(members).to_jcs()
}

/// Checks a response as a client does (SPEC 5.5): the outcome, or the code.
pub fn check(resp: &Resp, keys: &Keys, cell_id: &str, asof: Option<&str>, job: Option<&str>) -> Result<String, String> {
    let req = match job {
        Some(j) => Request::with_job(cell_id, asof, j).unwrap(),
        None => Request::new(cell_id, asof).unwrap(),
    };
    check_envelope_any(resp.status, &resp.body, keys, &req)
        .map(|e| e.outcome_str().to_string())
        .map_err(|e| e.code_str().to_string())
}

/// The spec's JSON Schemas, for envelopes and mesh documents.
pub struct Schemas {
    pub envelope: jsonschema::Validator,
    pub mesh: jsonschema::Validator,
}

pub fn spec_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("spec")
}

pub fn schemas() -> Schemas {
    let dir = spec_dir().join("schemas");
    let mut opts = jsonschema::options();
    let mut docs = std::collections::BTreeMap::new();
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        let id = v.get("$id").and_then(|x| x.as_str()).unwrap().to_string();
        docs.insert(p.file_name().unwrap().to_string_lossy().into_owned(), v.clone());
        opts = opts.with_resource(id, jsonschema::Resource::from_contents(v).unwrap());
    }
    let envelope = opts.build(&docs["envelope.schema.json"]).unwrap();
    let mut opts = jsonschema::options();
    for v in docs.values() {
        let id = v.get("$id").and_then(|x| x.as_str()).unwrap().to_string();
        opts = opts.with_resource(id, jsonschema::Resource::from_contents(v.clone()).unwrap());
    }
    let mesh = opts.build(&docs["mesh.schema.json"]).unwrap();
    Schemas { envelope, mesh }
}

impl Schemas {
    /// Asserts that `body` is an envelope by the spec's schema.
    #[track_caller]
    pub fn assert_envelope(&self, body: &[u8]) {
        let v: serde_json::Value = serde_json::from_slice(body).unwrap();
        let errors: Vec<String> =
            self.envelope.iter_errors(&v).map(|e| format!("{e} at {}", e.instance_path)).collect();
        assert!(errors.is_empty(), "envelope fails the schema: {errors:?}\n{}", String::from_utf8_lossy(body));
    }
}

/// Where tests write what the report quotes (`DATUMESH_HTTP_OUT`, default target-out/).
pub fn out_dir() -> std::path::PathBuf {
    let d = std::env::var("DATUMESH_HTTP_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target-out"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Percentile of sorted durations.
pub fn pct(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

/// A number from `/metrics`.
pub fn metric(text: &str, name: &str) -> f64 {
    text.lines()
        .find(|l| l.starts_with(name) && l[name.len()..].starts_with(' '))
        .and_then(|l| l.rsplit(' ').next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(f64::NAN)
}

pub const RATE_SIG: &str = r#"{"args":{"ccy":{"type":"string","valid":false},"date":{"type":"date","valid":true}},"order":["ccy","date"],"returns":{"type":"number","unit":"EUR"}}"#;
pub const DATE_SIG: &str =
    r#"{"args":{"date":{"type":"date","valid":true}},"order":["date"],"returns":{"type":"number","unit":"EUR"}}"#;
pub const TYPES_SIG: &str = r#"{"args":{"b":{"type":"boolean","valid":false},"d":{"type":"date","valid":false},"i":{"type":"instant","valid":false},"n":{"type":"number","unit":"EUR","valid":false},"s":{"type":"string","valid":false},"x":{"type":"blob","valid":false}},"order":["b","d","i","n","s","x"],"returns":{"type":"number","unit":"EUR"}}"#;
pub const TAG_SIG: &str = r#"{"args":{"tag":{"type":"string","valid":false}},"order":["tag"],"returns":"string"}"#;
pub const T1: &str = "2026-03-02T09:00:00.000000Z";
pub const T2: &str = "2026-03-03T09:00:00.000000Z";

/// The functions and values most tests use, under the node's host `<h>`:
/// `<h>/rate(ccy, date)` stored (USD and GBP on 2026-03-01 at T1, USD on 2026-03-02 at T2);
/// `<h>/types(b, d, i, n, s, x)` stored, without values; formulas `<h>/double` (rate * 2),
/// `<h>/broken` (rate / 0) and `<h>/total(date)` (the sum of rate(*, date)); `<h>/slow(tag)`
/// sleeping `slow_ms`; `<h>/secret(ccy)` and `<h>/paid(ccy)` stored, with a value for USD.
pub fn seed(node: &Node, slow_ms: u64) {
    let h = node.host.clone();
    node.stored(&format!("{h}/rate"), RATE_SIG);
    node.stored(&format!("{h}/types"), TYPES_SIG);
    let rate = format!("{h}/rate");
    node.put_value(&rate, &args(&[("ccy", s("USD")), ("date", d("2026-03-01"))]), n("1.08"), T1);
    node.put_value(&rate, &args(&[("ccy", s("GBP")), ("date", d("2026-03-01"))]), n("0.85"), T1);
    node.put_value(&rate, &args(&[("ccy", s("USD")), ("date", d("2026-03-02"))]), n("1.09"), T2);
    node.formula(&format!("{h}/double"), &format!("{h}/rate(ccy: ccy, date: date) * 2"), RATE_SIG);
    node.formula(&format!("{h}/broken"), &format!("{h}/rate(ccy: ccy, date: date) / 0"), RATE_SIG);
    node.formula(&format!("{h}/total"), &format!("sum({h}/rate(ccy: *, date: date))"), DATE_SIG);
    node.function(&format!("{h}/slow"), "sleep", &slow_ms.to_string(), &[], TAG_SIG);
    let one =
        r#"{"args":{"ccy":{"type":"string","valid":false}},"order":["ccy"],"returns":{"type":"number","unit":"EUR"}}"#;
    node.stored(&format!("{h}/secret"), one);
    node.stored(&format!("{h}/paid"), one);
    node.put_value(&format!("{h}/secret"), &args(&[("ccy", s("USD"))]), n("42"), T1);
    node.put_value(&format!("{h}/paid"), &args(&[("ccy", s("USD"))]), n("43"), T1);
}

/// The configuration `seed`'s functions need: alice may read `secret`, `paid` costs 0.01 EUR.
pub fn policy(c: &mut Config, alice_key: &str) {
    let h = c.host.clone();
    let p = format!(
        r#"{{"{h}/secret":{{"principals":["alice"]}},"{h}/paid":{{"price":{{"amount":"0.01","currency":"EUR"}}}}}}"#
    );
    c.policy = datumesh_http::config::Policy::from_json(p.as_bytes()).unwrap();
    c.api_keys.insert_hash(
        &datumesh_core::sha256_hex(alice_key.as_bytes()),
        datumesh_http::apikeys::Principal { id: "alice".into(), per_minute: None },
    );
}

pub const ALICE: &str = "dm_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Stores the value `<i>.5` of `<host>/<f>(i: i)` at `tx`, signed by `key`.
pub fn put_obs(store: &PgStore, key: &SigningKey, host: &str, f: &str, fh: &str, i: usize, tx: &str) {
    let cell = Cell { ns: host.into(), fn_name: f.into(), fn_hash: fh.into(), args: args(&[("i", n(&i.to_string()))]) };
    let r = ReceiptBuilder::new()
        .cell(cell.clone())
        .value(n(&format!("{i}.5")))
        .valid(Default::default())
        .asof(tx)
        .tx(tx)
        .issued(tx)
        .publisher(host)
        .signer_host(host)
        .licence("CC0-1.0")
        .summary("An observation.")
        .sign(key)
        .unwrap();
    let h = store.put_receipt(&receipt_json(&r)).unwrap();
    store.put_version(&cell.id(), tx, &h).unwrap();
}

/// Loads `<host>/<f>(i)` for i in 0..count from `threads` threads.
pub fn load_obs(node: &Node, f: &str, count: usize, threads: usize, tx: &str) {
    bl(|| {
        let fh = node.store.resolve_name(&format!("{}/{f}", node.host), tx).unwrap().unwrap();
        let hs: Vec<_> = (0..threads)
            .map(|t| {
                let (store, key, fh, host, f, tx) = (
                    node.store.clone(),
                    node.key.clone(),
                    fh.clone(),
                    node.host.clone(),
                    f.to_string(),
                    tx.to_string(),
                );
                std::thread::spawn(move || {
                    for i in (t..count).step_by(threads) {
                        put_obs(&store, &key, &host, &f, &fh, i, &tx);
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
    })
}

/// Makes the node's database commit without waiting for the disk (bulk loads).
pub fn fast_commits(node: &Node) {
    bl(|| {
        let cfg: PgConfig = admin_url().parse().unwrap();
        let mut c = cfg.connect(NoTls).unwrap();
        c.batch_execute(&format!("ALTER DATABASE {} SET synchronous_commit = off", node.db.name)).unwrap();
    })
}
