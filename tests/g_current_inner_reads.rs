//! The guard patch G10, decided 26 Nov (from the http implementation): "past" means the request
//! named an asof earlier than the mesh's clock at the request's arrival. The reads a request makes
//! inside it, a class chain's links (`Bindings::chain`, mesh.rs) and a label's lookup
//! (`Bindings::lookup`), follow the request: inside a current request they are current reads and
//! store their computed versions; inside a request with a named past asof they store nothing.
//!
//! The mesh's clock here ticks (one microsecond a reading), so the request's arrival is always before
//! the engine's later clock readings, as on a real mesh. The links and the lookup are formulas (a
//! stored link would store nothing either way): `successor(sku)` and `from_label(label)` of the class
//! `shop/product` read stored `raw_next` and `raw_label` cells.

mod common;
#[path = "common/crworld.rs"]
mod crworld;

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use crworld::*;
use datumesh_core::{Arg, Args, Cell, Scalar};
use datumesh_engine::Clock;
use datumesh_http::{Config, Mesh};
use datumesh_store::Store;

use common::lite::*;
use common::pub031::{class_line, function_line, BEFORE};
use common::world::class_json;

/// A clock that moves on by a microsecond at every reading, from MESH_NOW's midnight.
struct Ticking(AtomicI64);

impl Clock for Ticking {
    fn now(&self) -> String {
        let us = self.0.fetch_add(1, Ordering::SeqCst);
        let base = datumesh_formula::calendar::parse_instant("2026-11-16T00:00:00.000000Z").unwrap();
        datumesh_formula::calendar::instant_text(base + us).unwrap()
    }
}

/// A past time: after every cell (BETWEEN), before the mesh's clock.
const PAST: &str = "2026-11-15T12:00:05.000000Z";
const END_OF_TIME: &str = "9999-12-31T23:59:59.999999Z";

fn p() -> String {
    product()
}
fn class_str() -> String {
    format!(r#"{{"classes":["{}"],"type":"string"}}"#, p())
}
fn sku_sig(ret: &str) -> String {
    format!(r#"{{"args":{{"sku":{SKU_ARG}}},"order":["sku"],"returns":{ret}}}"#)
}
fn label_sig(ret: &str) -> String {
    format!(r#"{{"args":{{"label":{LABEL_ARG}}},"order":["label"],"returns":{ret}}}"#)
}
fn price_sig() -> String {
    format!(r#"{{"args":{{"date":{DATE_V},"sku":{SKU_ARG}}},"order":["sku","date"],"returns":{NUM1}}}"#)
}

struct Shop {
    lite: Lite,
    base: String,
    http: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Shop {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The shop with computed links, served by a mesh on a ticking clock.
async fn shop(label: &str) -> Shop {
    let lite = Lite::new(label);
    let pr = p();
    publish(
        &lite,
        "functions.jsonl",
        &[
            function_line(&format!("{pr}/price"), &price_sig(), None),
            function_line(&format!("{pr}/raw_next"), &sku_sig(&class_str()), None),
            function_line(&format!("{pr}/raw_label"), &label_sig(&class_str()), None),
        ],
        BEFORE,
    )
    .unwrap_or_else(|e| panic!("functions: {e}"));
    publish(
        &lite,
        "formulas.jsonl",
        &[
            formula_line(&format!("{pr}/successor"), &format!("{pr}/raw_next(sku: sku)"), &sku_sig(&class_str())),
            formula_line(
                &format!("{pr}/from_label"),
                &format!("{pr}/raw_label(label: label)"),
                &label_sig(&class_str()),
            ),
        ],
        NOW,
    )
    .unwrap_or_else(|e| panic!("formulas: {e}"));
    let h = |n: &str| fn_hash(&lite, &format!("{pr}/{n}"));
    let class = class_json(
        r#"{"sku":{"key_normalisation":"upper_ascii","type":"string"}}"#,
        &["sku"],
        &[("from_label", &h("from_label")), ("price", &h("price")), ("successor", &h("successor"))],
        HOST,
        r#""lookup":"from_label","#,
    );
    let lines = vec![
        class_line(&pr, &class, None),
        cell_line(&format!("{pr}/raw_next"), &[("sku", "string", "A1")], ("string", &pk("P1"))),
        cell_line(&format!("{pr}/raw_label"), &[("label", "string", "apple")], ("string", &pk("A1"))),
        price_cell_of("A1", SHOP_DAY, "1.5"),
        price_cell_of("P1", SHOP_DAY, "3"),
    ];
    publish(&lite, "classes.jsonl", &lines, BETWEEN).unwrap_or_else(|e| panic!("classes and cells: {e}"));
    let shared: Arc<dyn Store + Send + Sync> = lite.store.clone();
    let mesh = Mesh::builder(Config::new(HOST), shared)
        .signing_key(lite.key.clone())
        .clock(Arc::new(Ticking(AtomicI64::new(0))))
        .build()
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let _ = mesh.serve(listener, std::future::pending::<()>()).await;
    });
    Shop { lite, base: format!("http://{addr}"), http: common::client(), task }
}

impl Shop {
    async fn get(&self, path: &str) -> (u16, serde_json::Value, String) {
        let r = self.http.get(format!("{}{path}", self.base)).send().await.unwrap();
        let status = r.status().as_u16();
        let text = r.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(serde_json::Value::Null), text)
    }

    /// The computed version of `<product>/<f>(<arg>: <value>)` answering at the end of time, if any.
    fn computed(&self, f: &str, arg: &str, value: &str) -> Option<datumesh_store::Version> {
        let mut args = Args::new();
        args.insert(arg.into(), Arg::Value(Scalar::String(value.into())));
        let cell = Cell { ns: p(), fn_name: f.into(), fn_hash: fn_hash(&self.lite, &format!("{}/{f}", p())), args };
        self.lite.store.computed_asof(&cell.id(), END_OF_TIME).unwrap()
    }
}

fn value(v: &serde_json::Value) -> Option<&str> {
    v["receipt"]["value"].as_str()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_chains_link_inside_a_current_request_stores_its_version() {
    let s = shop("g10-chain-current").await;
    let (status, v, text) = s.get(&path(&format!("{}(A1).successor().price({SHOP_DAY})", p()), None)).await;
    assert_eq!((status, value(&v)), (200, Some("3")), "{text}");
    let link =
        s.computed("successor", "sku", "A1").expect("the link, read inside a current request, stores its version");
    // A second identical current request reuses it.
    let (_, v2, text2) = s.get(&path(&format!("{}(A1).successor().price({SHOP_DAY})", p()), None)).await;
    assert_eq!(value(&v2), Some("3"), "{text2}");
    assert_eq!(s.computed("successor", "sku", "A1"), Some(link), "reused, not evaluated again");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_chains_link_inside_a_past_request_stores_nothing() {
    let s = shop("g10-chain-past").await;
    let (status, v, text) = s.get(&path(&format!("{}(A1).successor().price({SHOP_DAY})", p()), Some(PAST))).await;
    assert_eq!((status, value(&v)), (200, Some("3")), "{text}");
    assert_eq!(s.computed("successor", "sku", "A1"), None, "a named past asof: the link stores nothing");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lookup_inside_a_current_request_stores_its_version() {
    let s = shop("g10-lookup-current").await;
    let (status, v, text) = s.get(&path(&format!("{}[apple].price({SHOP_DAY})", p()), None)).await;
    assert_eq!((status, value(&v)), (200, Some("1.5")), "{text}");
    assert!(
        s.computed("from_label", "label", "apple").is_some(),
        "the lookup, read inside a current request, stores its version"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lookup_inside_a_past_request_stores_nothing() {
    let s = shop("g10-lookup-past").await;
    let (status, v, text) = s.get(&path(&format!("{}[apple].price({SHOP_DAY})", p()), Some(PAST))).await;
    assert_eq!((status, value(&v)), (200, Some("1.5")), "{text}");
    assert_eq!(s.computed("from_label", "label", "apple"), None, "a named past asof: the lookup stores nothing");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_naming_an_asof_after_its_arrival_stores_nothing() {
    // Decided 27 Nov (second review, B1), replacing review item 7's "an asof at or after the arrival is
    // current" (51e485d): any request that names an asof, past or future, answers and signs but stores
    // nothing except the slice definition. Only a request with no asof, or a pending job, is current.
    let s = shop("g10-named-future").await;
    let named = "2026-11-16T00:00:01.000000Z";
    let (status, v, text) = s.get(&path(&format!("{}(A1).successor().price({SHOP_DAY})", p()), Some(named))).await;
    assert_eq!((status, value(&v)), (200, Some("3")), "{text}");
    assert_eq!(
        s.computed("successor", "sku", "A1"),
        None,
        "a named asof, even after the arrival: the link stores nothing"
    );
}
