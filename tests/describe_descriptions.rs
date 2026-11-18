//! CONTRACT-1.7a-text H1 (H-T 1 to 4): `GET /describe/{class}` fills each method's `description`
//! from the catalog entry `"{class}/{method}"` when it is 1 to 1,024 UTF-8 bytes; otherwise the
//! member is absent. Nothing else of the class answer changes. SQLite store, the mesh clock fixed at
//! 2026-11-16T00:00:00.000000Z, the real router on loopback.
//!
//! H-T 5 (describe by short id of a class) has no test: a short id names an object, and describe by a
//! class hash is not a form (see `classes_describe.rs`), so there is no such existing path.

mod common;

use std::sync::Arc;

use common::lite::*;
use common::pub031::T1;
use common::pub031::*;
use common::world::call_path;
use common::*;
use datumesh_core::json::Value;
use datumesh_http::{Config, Mesh};
use datumesh_store::Store;

const MESH_NOW: &str = "2026-11-16T00:00:00.000000Z";
const CLASS: &str = "mesh-test.datumesh.com/scratch_1_7a/k/c";
const RATE: &str = "mesh-test.datumesh.com/scratch_1_7a/k/c/rate";
const CONVERT: &str = "mesh-test.datumesh.com/scratch_1_7a/k/c/convert";

/// Non-ASCII on purpose: Arabic, with marks, and an accented Latin word.
const RATE_DESC: &str = "سعر الصرف اليومي \u{64e}\u{650} — café";

struct Served {
    base: String,
    http: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
    _lite: Lite,
}

impl Drop for Served {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Served {
    async fn describe_class(&self) -> (Vec<u8>, Value) {
        let r = self.http.get(format!("{}/describe/{}", self.base, call_path(CLASS))).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 200);
        let body = r.bytes().await.unwrap().to_vec();
        let v = datumesh_core::json::parse(&body).unwrap();
        (body, v)
    }
}

/// The class `K/c` with methods `rate` and `convert`, and a catalog of the given entries.
async fn served(catalog: &[(&str, String)]) -> Served {
    let lite = Lite::new("h1");
    let rate = stored_descriptor(&rate_sig());
    let convert = stored_descriptor(&convert_sig());
    let (rate_hash, convert_hash) = (descriptor_hash(&rate), descriptor_hash(&convert));
    let f1 = write(&lite, "d.jsonl", &[descriptor_line(&rate), descriptor_line(&convert)]);
    let class = write(
        &lite,
        "c.jsonl",
        &[class_line(CLASS, &pair_class(&[("convert", &convert_hash), ("rate", &rate_hash)]), None)],
    );
    let _ = run(&lite, &f1, NOW);
    let store = lite.store.clone();
    for d in [&rate, &convert] {
        store.put_descriptor(&descriptor_canonical(d)).unwrap();
    }
    run(&lite, &class, T1).unwrap();

    let mut config = Config::new(HOST);
    for (n, d) in catalog {
        config.catalog.insert(n.to_string(), d.clone());
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
    Served { base: format!("http://{addr}"), http: client(), task, _lite: lite }
}

/// The `description` of method `m` in a class answer, if present.
fn description(doc: &Value, m: &str) -> Option<String> {
    let table = doc.get("methods").and_then(Value::as_object).expect("methods");
    let entry = table.iter().find(|(k, _)| k == m).map(|(_, v)| v).unwrap_or_else(|| panic!("no method {m}"));
    entry.get("description").map(|d| d.as_str().expect("description is a string").to_string())
}

/// H-T1: a catalog entry for a method is its `description`, byte for byte; the other method, with no
/// entry, has none; the class has no description member.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_catalog_entry_is_the_methods_description() {
    let m = served(&[(RATE, RATE_DESC.to_string())]).await;
    let (_, doc) = m.describe_class().await;
    assert_eq!(description(&doc, "rate").as_deref(), Some(RATE_DESC));
    assert_eq!(description(&doc, "convert"), None);
    assert!(doc.get("description").is_none(), "the class answer has no description member (16.8)");
}

/// H-T2: no catalog entry, no `description` member on any method.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_catalog_entry_means_no_description() {
    let m = served(&[]).await;
    let (_, doc) = m.describe_class().await;
    assert_eq!(description(&doc, "rate"), None);
    assert_eq!(description(&doc, "convert"), None);
}

/// H-T3: 1,024 bytes is present, 1,025 is absent, empty is absent (bytes, not chars: `é` is two).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_description_limit_is_1024_bytes() {
    let exact = format!("{}é", "a".repeat(1022));
    assert_eq!(exact.len(), 1024);
    assert!(exact.chars().count() < 1024);
    let m = served(&[(RATE, exact.clone()), (CONVERT, String::new())]).await;
    let (_, doc) = m.describe_class().await;
    assert_eq!(description(&doc, "rate"), Some(exact), "1,024 bytes is present, whole");
    assert_eq!(description(&doc, "convert"), None, "empty is absent");

    let over = "a".repeat(1025);
    let multibyte_over = format!("{}éé", "a".repeat(1022)); // 1,026 bytes, 1,024 chars
    assert_eq!(multibyte_over.chars().count(), 1024);
    for text in [over, multibyte_over] {
        let m = served(&[(RATE, text.clone()), (CONVERT, text)]).await;
        let (_, doc) = m.describe_class().await;
        assert_eq!(description(&doc, "rate"), None, "over 1,024 bytes is absent, never truncated");
        assert_eq!(description(&doc, "convert"), None);
    }
}

/// H-T4: apart from the methods' `description`, the answer is byte-identical to the one with no
/// catalog: same `class_hash`, `descriptor`, and the other members of each method.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_answer_is_otherwise_unchanged_by_the_catalog() {
    let plain = served(&[]).await;
    let (_, without) = plain.describe_class().await;
    let cat = served(&[(CLASS, "A class entry".into()), (RATE, RATE_DESC.into()), (CONVERT, "Converts".into())]).await;
    let (_, with) = cat.describe_class().await;
    assert_eq!(description(&with, "rate").as_deref(), Some(RATE_DESC));
    assert_eq!(description(&with, "convert").as_deref(), Some("Converts"));

    for k in ["class", "class_hash", "descriptor", "kind"] {
        assert_eq!(with.get(k).map(Value::to_jcs), without.get(k).map(Value::to_jcs), "{k}");
    }
    let (tw, to) = (
        with.get("methods").and_then(Value::as_object).unwrap(),
        without.get("methods").and_then(Value::as_object).unwrap(),
    );
    assert_eq!(tw.len(), to.len());
    for ((kw, vw), (ko, vo)) in tw.iter().zip(to.iter()) {
        assert_eq!(kw, ko);
        let strip = |v: &Value| -> Vec<(String, Vec<u8>)> {
            v.as_object()
                .unwrap()
                .iter()
                .filter(|(k, _)| k != "description")
                .map(|(k, v)| (k.clone(), v.to_jcs()))
                .collect()
        };
        assert_eq!(strip(vw), strip(vo), "{kw}: every member but description is as without a catalog");
    }
}
