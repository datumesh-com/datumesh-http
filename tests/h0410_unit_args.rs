//! http 0.4.10 (spec-0.6.2 in the mesh; CONTRACT-spec-0.6.2 P1, D44; SPEC 7.1, units.md): a stored
//! function returning `{"arg": "unit"}`, with cells whose `unit` argument is `USD`, `shares`,
//! `USD/shares` and `pure`, as the SEC relay will have it.
//!
//! The cells publish (the resolved forms are valid), each receipt's value carries the resolved unit,
//! the receipts verify, and `describe` and the function page show the declared `returns` as the mesh
//! already shows an argument unit (`{"arg": "unit"}`, as `classes_describe` pins for fx_pair).

mod common;
#[path = "common/calculus.rs"]
mod calculus;

use calculus::*;
use common::lite::*;
use common::pub031::*;
use common::readpath::*;
use common::world::call_path;
use serde_json::{json, Value};

const FN: &str = "mesh-test.datumesh.com/scratch_h0410u/fact";
const STR: &str = r#"{"type":"string","valid":false}"#;
const UNITS: [&str; 4] = ["USD", "shares", "USD/shares", "pure"];

fn sig() -> String {
    format!(
        r#"{{"args":{{"concept":{STR},"unit":{STR}}},"order":["concept","unit"],"returns":{{"type":"number","unit":{{"arg":"unit"}}}}}}"#
    )
}

fn cell(unit: &str, value: &str) -> String {
    cell_line(FN, json!({"concept": {"type": "string", "value": "x"}, "unit": {"type": "string", "value": unit}}), value)
}

fn lines() -> Vec<String> {
    let mut v = vec![function_line(FN, &sig(), None)];
    for (i, u) in UNITS.iter().enumerate() {
        v.push(cell(u, &format!("{}", i + 1)));
    }
    v
}

fn path(unit: &str) -> String {
    format!("/cell/{}", call_path(&format!("{FN}(concept: \"x\", unit: \"{unit}\")")))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cells_with_resolved_units_publish_and_carry_the_unit() {
    let m = serve_lite("h0410-units", &lines(), None, |_| {}).await;
    let mut keys = datumesh_core::Keys::new();
    keys.insert_response(
        HOST,
        200,
        Some(&datumesh_http::keydoc::with_key(HOST, None, &common::key_for(HOST), K_FROM).unwrap()),
    );
    for (i, u) in UNITS.iter().enumerate() {
        let v = m.get(&path(u)).await;
        assert_eq!(v["outcome"], "ok", "{u}: {v}");
        assert_eq!(v["receipt"]["value"], format!("{}", i + 1), "{v}");
        assert!(v["receipt"].get("unit").is_none(), "a receipt carries no unit member, as for any arg unit: {v}");
        let r = datumesh_core::verify(&serde_json::to_vec(&v["receipt"]).unwrap(), &keys, None, None);
        assert!(r.ok, "{u}: {:?}", r.error());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describe_and_the_function_page_show_the_declared_unit() {
    let m = serve_lite("h0410-units-d", &lines(), None, |_| {}).await;
    let want = json!({"type": "number", "unit": {"arg": "unit"}});
    let d = m.get(&format!("/describe/{}", call_path(FN))).await;
    assert_eq!(d["returns"], want, "{d}");
    let page = publish_get(&m).await;
    assert_eq!(page["signature"]["returns"], want, "{page}");
}

async fn publish_get(m: &Served) -> Value {
    let r = m.get_any(&format!("/fn/{FN}?format=json")).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    r.json
}

/// core resolves the returns unit of each cell (7.1): the served descriptor and the cell give `USD`,
/// `shares`, `USD/shares` and `pure`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn core_resolves_each_cells_unit() {
    let m = serve_lite("h0410-units-r", &lines(), None, |_| {}).await;
    for u in UNITS {
        let v = m.get(&path(u)).await;
        let hash = v["fn_hash"].as_str().unwrap();
        let d = m.get_any(&format!("/descriptor/{hash}")).await;
        let cell = serde_json::to_vec(&v["receipt"]["cell"]).unwrap();
        let got = datumesh_core::resolved_unit(&d.body, &cell).unwrap_or_else(|e| panic!("{u}: {e}"));
        assert_eq!(got.as_deref(), Some(u), "{v}");
    }
}
