//! CONTRACT-1.7a H3: `GET /find` returns classes and methods as well as functions, each with a
//! `kind`. The catalog lists a class `N`, its methods `N/m` and a plain function; a class is also
//! matched against its methods' names and the catalog descriptions of its methods. SQLite store,
//! the mesh clock fixed at 2026-11-16T00:00:00.000000Z, the real router on loopback.

mod common;

use std::sync::Arc;

use common::lite::*;
use common::pub031::T1;
use common::pub031::*;
use common::*;
use datumesh_core::json::Value;
use datumesh_http::{Config, Mesh};
use datumesh_store::Store;

const MESH_NOW: &str = "2026-11-16T00:00:00.000000Z";
const CLASS: &str = "mesh-test.datumesh.com/scratch_1_7a/k/c";
const RATE: &str = "mesh-test.datumesh.com/scratch_1_7a/k/c/rate";
const CONVERT: &str = "mesh-test.datumesh.com/scratch_1_7a/k/c/convert";
const PLAIN: &str = "mesh-test.datumesh.com/scratch_1_7a/plain_fn";
const GHOST: &str = "mesh-test.datumesh.com/scratch_1_7a/k/ghost";

const CLASS_DESC: &str = "Currency pairs of the test mesh";
const RATE_DESC: &str = "Daily reference rate, zebrafish edition";
const CONVERT_DESC: &str = "Converts an amount between two currencies";
const PLAIN_DESC: &str = "A plain function about giraffes";

struct Served {
    base: String,
    http: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
    class_hash: String,
    rate_hash: String,
    convert_hash: String,
    plain_hash: String,
    class_tx: String,
    plain_tx: String,
    _lite: Lite,
}

impl Drop for Served {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Served {
    async fn get(&self, path: &str) -> Value {
        let r = self.http.get(format!("{}{path}", self.base)).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 200);
        datumesh_core::json::parse(&r.bytes().await.unwrap()).unwrap()
    }

    async fn get_text(&self, path: &str) -> String {
        self.http.get(format!("{}{path}", self.base)).send().await.unwrap().text().await.unwrap()
    }
}

/// The class `K/c` with methods `rate` and `convert`, a plain function and a catalog.
async fn served() -> Served {
    let lite = Lite::new("h3");
    let rate = stored_descriptor(&rate_sig());
    let convert = stored_descriptor(&convert_sig());
    let (rate_hash, convert_hash) = (descriptor_hash(&rate), descriptor_hash(&convert));
    let f1 = write(&lite, "d.jsonl", &[descriptor_line(&rate), descriptor_line(&convert)]);
    let plain = write(&lite, "p.jsonl", &[function_line(PLAIN, &rate_sig(), None)]);
    let class = write(
        &lite,
        "c.jsonl",
        &[class_line(CLASS, &pair_class(&[("convert", &convert_hash), ("rate", &rate_hash)]), None)],
    );
    // Rule A, one clock: descriptors and the plain function at NOW, the class after.
    let _ = run(&lite, &f1, NOW);
    run(&lite, &plain, NOW).unwrap();
    // The class's methods are the functions the store has by hash; put them directly if the
    // descriptor lines are not accepted yet, so the failure is the find's and not the setup's.
    let store = lite.store.clone();
    for d in [&rate, &convert] {
        store.put_descriptor(&descriptor_canonical(d)).unwrap();
    }
    run(&lite, &class, T1).unwrap();

    let mut config = Config::new(HOST);
    for (n, d) in [
        (CLASS, CLASS_DESC),
        (RATE, RATE_DESC),
        (CONVERT, CONVERT_DESC),
        (PLAIN, PLAIN_DESC),
        (GHOST, "A class that is never bound, kodiak"),
    ] {
        config.catalog.insert(n.to_string(), d.to_string());
    }
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
    let class_hash = lite.store.resolve_class(CLASS, END).unwrap().unwrap();
    let class_tx = lite.store.resolve_class_binding(CLASS, END).unwrap().unwrap().tx;
    let plain_tx = lite.bound_tx(PLAIN).unwrap();
    let plain_hash = lite.store.resolve_name(PLAIN, END).unwrap().unwrap();
    Served {
        base: format!("http://{addr}"),
        http: client(),
        task,
        class_hash,
        rate_hash,
        convert_hash,
        plain_hash,
        class_tx,
        plain_tx,
        _lite: lite,
    }
}

fn results(v: &Value) -> Vec<&Value> {
    match v.get("results") {
        Some(Value::Array(a)) => a.iter().collect(),
        other => panic!("results: {other:?}"),
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no string {k} in {}", String::from_utf8_lossy(&v.to_jcs())))
}

fn keys_of(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().iter().map(|(k, _)| k.clone()).collect();
    k.sort();
    k
}

fn kind_of<'a>(rs: &[&'a Value], kind: &str, name: &str) -> &'a Value {
    rs.iter()
        .find(|r| s(r, "kind") == kind && s(r, "name") == name)
        .unwrap_or_else(|| panic!("no {kind} {name} in {} results", rs.len()))
}

/// A word found only in a method's description (`zebrafish`, the description of `.../rate`) finds
/// the class (and the method entry itself); the class result is complete.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_word_of_a_methods_description_finds_the_class() {
    let m = served().await;
    let v = m.get("/find?q=zebrafish").await;
    let rs = results(&v);
    let c = kind_of(&rs, "class", CLASS);
    assert_eq!(s(c, "description"), CLASS_DESC);
    assert_eq!(s(c, "class_hash"), m.class_hash);
    assert_eq!(s(c, "since"), m.class_tx);
    assert!(matches!(c.get("descriptor"), Some(Value::Object(_))), "the flattened class descriptor");
    // 0.4.0 (R3): every result also has its `call`, `label`, `match` and `score`.
    assert_eq!(
        keys_of(c),
        [
            "call",
            "class_hash",
            "description",
            "descriptor",
            "kind",
            "label",
            "match",
            "methods",
            "name",
            "score",
            "since"
        ]
        .map(String::from)
    );
    // The method entry matches by its own description.
    kind_of(&rs, "method", RATE);
}

/// A class name finds the class with its methods: for each, the function's name and hash, its
/// arguments after the keys, and what it returns.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_class_name_finds_the_class_with_its_methods() {
    let m = served().await;
    let v = m.get("/find?q=scratch_1_7a/k/c").await;
    let rs = results(&v);
    let c = kind_of(&rs, "class", CLASS);
    let methods = c.get("methods").and_then(Value::as_object).expect("methods");
    let mut names: Vec<&str> = methods.iter().map(|(k, _)| k.as_str()).collect();
    names.sort();
    assert_eq!(names, ["convert", "rate"]);
    let get = |name: &str| &methods.iter().find(|(k, _)| k == name).unwrap().1;
    let (rate, convert) = (get("rate"), get("convert"));
    assert_eq!((s(rate, "fn"), s(rate, "fn_hash")), (RATE, m.rate_hash.as_str()));
    assert_eq!((s(convert, "fn"), s(convert, "fn_hash")), (CONVERT, m.convert_hash.as_str()));
    // `args` are the arguments after the keys (`base`, `quote`): `date` for rate, `amount` for convert.
    let args = String::from_utf8(rate.get("args").expect("args").to_jcs()).unwrap();
    assert!(args.contains("date") && !args.contains("base") && !args.contains("quote"), "{args}");
    let args = String::from_utf8(convert.get("args").expect("args").to_jcs()).unwrap();
    assert!(args.contains("amount") && !args.contains("base") && !args.contains("quote"), "{args}");
    assert!(rate.get("returns").is_some() && convert.get("returns").is_some());
    // The class is first: the catalog's order, ascending by name (class before its methods).
    assert_eq!(s(rs[0], "kind"), "class");
}

/// The class is matched against its methods' names too: `convert` is in no description of the
/// class's own entry.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_class_is_matched_against_its_method_names() {
    let m = served().await;
    let v = m.get("/find?q=pairs%20convert").await;
    let rs = results(&v);
    kind_of(&rs, "class", CLASS);
    assert!(rs.iter().all(|r| s(r, "kind") == "class"), "no entry but the class has both words");
}

/// A method entry of the catalog (`N/m`): its own members.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_method_entry_has_its_class_and_function() {
    let m = served().await;
    let v = m.get("/find?q=zebrafish").await;
    let rs = results(&v);
    let r = kind_of(&rs, "method", RATE);
    assert_eq!(s(r, "description"), RATE_DESC);
    assert_eq!(s(r, "class"), CLASS);
    assert_eq!(s(r, "fn_hash"), m.rate_hash);
    assert_eq!(s(r, "since"), m.class_tx, "the method is bound with the class");
    assert!(matches!(r.get("descriptor"), Some(Value::Object(_))));
    assert_eq!(
        keys_of(r),
        ["call", "class", "description", "descriptor", "fn_hash", "kind", "label", "match", "name", "score", "since"]
            .map(String::from)
    );
}

/// A function's result is what it was, apart from `kind`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_function_result_is_unchanged_apart_from_kind() {
    let m = served().await;
    let v = m.get("/find?q=giraffes").await;
    let rs = results(&v);
    assert_eq!(rs.len(), 1, "only the function: the class does not match");
    let r = rs[0];
    assert_eq!(s(r, "kind"), "function");
    assert_eq!(s(r, "name"), PLAIN);
    assert_eq!(s(r, "description"), PLAIN_DESC);
    assert_eq!(s(r, "fn_hash"), m.plain_hash);
    assert_eq!(s(r, "since"), m.plain_tx);
    assert!(matches!(r.get("descriptor"), Some(Value::Object(_))));
    assert_eq!(
        keys_of(r),
        ["call", "description", "descriptor", "fn_hash", "kind", "label", "match", "name", "score", "since"]
            .map(String::from)
    );
}

/// A catalog name unbound as of now is skipped, as before; the order is the catalog's (ascending
/// name) and `limit` cuts it. (0.4.0: a blank query lists the entries; `limit` defaults to 10.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unbound_names_are_skipped_and_the_order_and_limit_hold() {
    let m = served().await;
    let v = m.get("/find?q=").await;
    let names: Vec<&str> = results(&v).iter().map(|r| s(r, "name")).collect();
    assert_eq!(names, [CLASS, CONVERT, RATE, PLAIN], "{names:?}");
    assert!(!names.contains(&GHOST));
    let kinds: Vec<&str> = results(&v).iter().map(|r| s(r, "kind")).collect();
    assert_eq!(kinds, ["class", "method", "method", "function"]);
    let v = m.get("/find?q=&limit=2").await;
    assert_eq!(results(&v).len(), 2);
    let v = m.get("/find?q=kodiak").await;
    assert!(results(&v).is_empty(), "the class that is not bound is skipped");
}

/// `openapi.json` documents the new members of a result.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn openapi_documents_the_new_members() {
    let m = served().await;
    let doc = m.get_text("/openapi.json").await;
    for word in ["class_hash", "methods", "kind"] {
        assert!(doc.contains(word), "openapi.json does not mention {word}");
    }
}
