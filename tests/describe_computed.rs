//! The describe index over every recorded method cell, computed ones included
//! (`CONTRACT-1.8b.md` H4): the objects of `N/m` are those of its stored cells (`slice_values`) and
//! of its computed cells (store 0.4.2 `computed_cells`); nothing is evaluated and no product of
//! argument extents is formed to build the index.

mod common;

use std::sync::Arc;

use common::world::*;
use common::*;
use datumesh_core::{Arg, Cell, KeyValue, ObjectKey, Scalar, Slice, Type};
use datumesh_http::{Config, Mesh};
use datumesh_store::Store;

const A: &str = "mesh-a.example";
const STR: &str = r#"{"type":"string","valid":false}"#;

fn describe_path(id: &str) -> String {
    format!("/describe/{}", call_path(id))
}

fn short_id(class: &str, class_hash: &str, base: &str, quote: &str) -> String {
    let key = |v: &str| KeyValue::Scalar(Scalar::from_lexical(Type::String, v).unwrap());
    let object = ObjectKey::new(class, [("base".to_string(), key(base)), ("quote".to_string(), key(quote))]).unwrap();
    object.object_id(class_hash).unwrap()[..12].to_string()
}

/// A class `ns/pair` with keys `base`, `quote` and the methods `rate` (stored, cells for (EUR, USD) and
/// (EUR, GBP)) and `twice` (a formula over `rate`). Returns the class name and hash.
async fn pair_world(a: &Node) -> (String, String) {
    let class = format!("{A}/h4/pair");
    let rate = a.stored(&format!("{class}/rate"), &fx_rate_sig());
    let twice = a.formula(
        &format!("{class}/twice"),
        &format!("{class}/rate(base: base, quote: quote, date: date) * 2"),
        &fx_rate_sig(),
    );
    let source = class_json(NONE_KEYS_PAIR, &["base", "quote"], &[("rate", &rate), ("twice", &twice)], A, "");
    let hash = class_at(a, &class, &source, TC);
    for quote in ["USD", "GBP"] {
        a.put_value(
            &format!("{class}/rate"),
            &args(&[("base", s("EUR")), ("date", d(DAY)), ("quote", s(quote))]),
            n("1.2"),
            TV,
        );
    }
    (class, hash)
}

/// A class `ns/only` whose single method `val` is a formula; returns the class name, hash, and the
/// method's hash.
async fn only_world(a: &Node) -> (String, String, String) {
    let class = format!("{A}/h4/only");
    let sig = format!(r#"{{"args":{{"base":{STR},"quote":{STR}}},"order":["base","quote"],"returns":"string"}}"#);
    let val = a.formula(&format!("{class}/val"), "base", &sig);
    let source = class_json(NONE_KEYS_PAIR, &["base", "quote"], &[("val", &val)], A, "");
    let hash = class_at(a, &class, &source, TC);
    (class, hash, val)
}

/// The computed cells of `name` (hash `hash`, keys `base` and `quote`), every argument open.
fn computed_count(a: &Node, name: &str, hash: &str, arg_names: &[&str]) -> usize {
    let (ns, f) = name.rsplit_once('/').unwrap();
    let args = arg_names.iter().map(|k| (k.to_string(), Arg::Slice)).collect();
    let cell = Cell { ns: ns.into(), fn_name: f.into(), fn_hash: hash.into(), args };
    let slice = Slice::from_value(&cell.to_value()).unwrap();
    bl(|| a.store.computed_cells(&slice, "9999-12-31T23:59:59.999999Z", 1_000_000).unwrap().len())
}

/// 1. Before any `twice` cell is computed, (EUR, GBP) is known through `rate`; an object with no
///    recorded cell of any method (USD, JPY) is `404`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_object_is_known_through_its_stored_method() {
    let a = Node::start(A, |c| c.budget = std::time::Duration::from_secs(30)).await;
    let (class, hash) = pair_world(&a).await;
    assert_eq!(a.get(&describe_path(&class)).await.status, 200);
    // HP1 to HP3 (CONTRACT-round-0.7-P): the describe indexes nothing, so the mesh is told of the stored version
    for quote in ["USD", "GBP"] {
        let id =
            a.cell_id(&format!("{class}/rate"), &args(&[("base", s("EUR")), ("date", d(DAY)), ("quote", s(quote))]));
        bl(|| a.mesh.stored_version_added(&id));
    }
    let known = short_id(&class, &hash, "EUR", "GBP");
    let r = a.get(&describe_path(&known)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let unknown = short_id(&class, &hash, "USD", "JPY");
    let r = a.get(&describe_path(&unknown)).await;
    assert_eq!(r.status, 404, "{}", r.text());
}

/// 2. A class whose only method is a formula: 404 before a computation; after a `get` of one method
///    cell (200), describe by that object's short id is `200` with the object.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_computed_only_object_is_known_after_a_computation() {
    let a = Node::start(A, |c| c.budget = std::time::Duration::from_secs(30)).await;
    let (class, hash, _) = only_world(&a).await;
    assert_eq!(a.get(&describe_path(&class)).await.status, 200);
    let short = short_id(&class, &hash, "EUR", "GBP");
    let r = a.get(&describe_path(&short)).await;
    assert_eq!(r.status, 404, "{}", r.text());

    let got = a.get(&format!("/cell/{class}/val?base=EUR&quote=GBP&asof={ASOF}")).await;
    assert_eq!(got.status, 200, "{}", got.text());
    let r = a.get(&describe_path(&short)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let doc = r.json();
    assert_eq!(doc.get("short_id").and_then(datumesh_core::json::Value::as_str), Some(short.as_str()));
    assert!(doc.get("object").is_some(), "{}", r.text());
}

/// 3. After a restart (a new `Mesh` over the same store, the class heard of again through describe of the class
///    name) a computed-only object is no longer known by id: the index does not read `computed_cells`
///    (CONTRACT-round-0.7-P HP2), and a class without `extent.keys` is indexed from its first method's stored
///    cells. It still describes by its key encoding, and is known again once it is computed again (test 2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_computed_object_is_not_known_by_id_after_a_restart() {
    let a = Node::start(A, |c| c.budget = std::time::Duration::from_secs(30)).await;
    let (class, hash, _) = only_world(&a).await;
    let short = short_id(&class, &hash, "EUR", "GBP");
    let got = a.get(&format!("/cell/{class}/val?base=EUR&quote=GBP&asof={ASOF}")).await;
    assert_eq!(got.status, 200, "{}", got.text());

    let shared: Arc<dyn Store + Send + Sync> = a.store.clone();
    let restarted = bl(|| {
        Mesh::builder(Config::new(A), shared).signing_key(a.key.clone()).runtime(Arc::new(Sleep)).build().unwrap()
    });
    let r = bl(|| restarted.describe(&class));
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let r = bl(|| restarted.describe(&short));
    assert!(matches!(r.status, 404 | 503), "{} {}", r.status, String::from_utf8_lossy(&r.body));
    let enc = format!(r#"{class}(base:"EUR",quote:"GBP")"#);
    let r = bl(|| restarted.describe(&enc));
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
}

/// 4. Nothing is evaluated by the index: the store's computed-version count is the same before and
///    after a describe by short id.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_index_evaluates_nothing() {
    let a = Node::start(A, |c| c.budget = std::time::Duration::from_secs(30)).await;
    let (class, hash) = pair_world(&a).await;
    let twice = bl(|| a.store.resolve_name(&format!("{class}/twice"), "9999-12-31T23:59:59.999999Z").unwrap().unwrap());
    let got = a.get(&format!("/cell/{class}/twice?base=EUR&date={DAY}&quote=GBP&asof={ASOF}")).await;
    assert_eq!(got.status, 200, "{}", got.text());
    let names = ["base", "date", "quote"];
    let before = computed_count(&a, &format!("{class}/twice"), &twice, &names);
    assert!(before >= 1, "the get computed a cell");
    assert_eq!(a.get(&describe_path(&class)).await.status, 200);
    for (base, quote) in [("EUR", "GBP"), ("USD", "JPY")] {
        let _ = a.get(&describe_path(&short_id(&class, &hash, base, quote))).await;
    }
    assert_eq!(computed_count(&a, &format!("{class}/twice"), &twice, &names), before);
}
