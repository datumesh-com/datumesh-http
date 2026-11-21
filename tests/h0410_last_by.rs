//! http 0.4.10 (spec-0.6.2 in the mesh; CONTRACT-spec-0.6.2 P2, D45; SPEC 15.7, 15.13): `last_by` and
//! `first_by` over a stored function and the method of a class, the SEC use ("the figure from the
//! latest filing").
//!
//! `company_fact(cik, accession) -> number` is stored; the class `filing` (key `accession`) has the
//! stored method `filed() -> date`. The formula
//! `last_by(company_fact(cik: cik, accession: *), filing/filed(accession: *))` defines with tests (the
//! public rule), publishes, and evaluates through `/cell` and `/x/calc`, the member being the one of the
//! latest filing. A fact whose filing has no `filed` cell is not a candidate (inner join).

mod common;
#[path = "common/calculus.rs"]
mod calculus;

use calculus::*;
use common::lite::*;
use common::pub031::*;
use common::readpath::*;
use common::world::call_path;
use datumesh_store::Store;
use serde_json::{json, Value};

const P: &str = "mesh-test.datumesh.com/scratch_h0410l";
const FILING: &str = "mesh-test.datumesh.com/scratch_h0410l/filing";

fn fact() -> String {
    format!("{P}/company_fact")
}
fn filed() -> String {
    format!("{FILING}/filed")
}
fn latest() -> String {
    format!("{P}/latest_fact")
}
fn earliest() -> String {
    format!("{P}/earliest_fact")
}

/// Three runs: the data, the class and the formulas at `T0`; the filing dates at `T0B`; the formulas again
/// with their tests, under the public rule, at `T1_`.
const T0: &str = "2026-11-10T00:00:00.000000Z";
const T0B: &str = "2026-11-11T00:00:00.000000Z";
const BETWEEN: &str = "2026-11-10T12:00:00.000000Z";
const T1_: &str = "2026-11-12T00:00:00.000000Z";

const STR: &str = r#"{"type":"string","valid":false}"#;

fn fact_sig() -> String {
    format!(
        r#"{{"args":{{"accession":{STR},"cik":{STR}}},"order":["cik","accession"],"returns":{{"type":"number","unit":"1"}}}}"#
    )
}

/// The method's arguments after the class key: none.
fn filed_sig() -> String {
    r#"{"args":{"accession":{"type":"string","valid":false}},"order":["accession"],"returns":"date"}"#
        .to_string()
}

fn cik_sig() -> String {
    format!(r#"{{"args":{{"cik":{STR}}},"order":["cik"],"returns":{{"type":"number","unit":"1"}}}}"#)
}

fn fact_cell(cik: &str, accession: &str, value: &str) -> String {
    cell_line(&fact(), json!({"cik": {"type": "string", "value": cik}, "accession": {"type": "string", "value": accession}}), value)
}

fn filed_cell(accession: &str, date: &str) -> String {
    json!({"cell": {"fn": filed(), "args": {"accession": {"type": "string", "value": accession}}},
        "licence": "CC0-1.0", "summary": "Synthetic test data.", "value": {"type": "date", "value": date}})
    .to_string()
}

fn latest_body() -> String {
    format!("last_by({}(cik: cik, accession: *), {}(accession: *))", fact(), filed())
}

fn earliest_body() -> String {
    format!("first_by({}(cik: cik, accession: *), {}(accession: *))", fact(), filed())
}

fn call(f: &str, cik: &str) -> String {
    format!("{f}(cik: \"{cik}\")")
}

fn data_lines() -> Vec<String> {
    let d = stored_descriptor(&filed_sig());
    let h = descriptor_hash(&d);
    let class = common::world::class_json(
        r#"{"accession":{"key_normalisation":"none","type":"string"}}"#,
        &["accession"],
        &[("filed", &h)],
        HOST,
        "",
    );
    vec![
        function_line(&fact(), &fact_sig(), None),
        descriptor_line(&d),
        class_line(FILING, &class, None),
        // C1: A1 filed Jan, A2 filed Mar, A3 filed Feb, A4 never filed (no candidate)
        fact_cell("C1", "A1", "10"),
        fact_cell("C1", "A2", "20"),
        fact_cell("C1", "A3", "30"),
        fact_cell("C1", "A4", "99"),
        fact_cell("C2", "A1", "5"),
    ]
}

fn bare_formulas() -> Vec<String> {
    vec![
        formula_line(&latest(), &latest_body(), &cik_sig(), None),
        formula_line(&earliest(), &earliest_body(), &cik_sig(), None),
    ]
}

/// The filing dates, a day after the facts (so a test as of the day between sees facts and no dates).
fn filed_lines() -> Vec<String> {
    vec![
        filed_cell("A1", "2026-01-01"),
        filed_cell("A2", "2026-03-01"),
        filed_cell("A3", "2026-02-01"),
    ]
}

fn formula_lines() -> Vec<String> {
    let mut v = vec![
        formula_line(&latest(), &latest_body(), &cik_sig(), None),
        formula_line(&earliest(), &earliest_body(), &cik_sig(), None),
    ];
    // the tests of the public rule: the happy cases, and an absent input
    for (f, c, want) in [(latest(), "C1", "20"), (latest(), "C2", "5"), (earliest(), "C1", "10")] {
        v.push(test_line(&f, &call(&f, c), None, None, num(want)));
    }
    for f in [latest(), earliest()] {
        v.push(test_line(&f, &call(&f, "ZZ"), None, None, fault("E_FORMULA_ABSENT")));
        // facts but no filing date yet: no candidate
        v.push(test_line(&f, &call(&f, "C1"), None, Some(BETWEEN), fault("E_FORMULA_ABSENT")));
    }
    v
}

async fn mesh(label: &str) -> Served {
    let lite = Lite::new(label);
    // The formulas are bound from `T0`, before the filing dates exist; the last run states them again
    // with their tests under the public rule (same hash: nothing is rebound).
    let mut first = data_lines();
    first.extend(bare_formulas());
    let f1 = write(&lite, "data.jsonl", &first);
    run(&lite, &f1, T0).unwrap_or_else(|e| panic!("the data and the class publish: {e}"));
    let fb = write(&lite, "filed.jsonl", &filed_lines());
    run(&lite, &fb, T0B).unwrap_or_else(|e| panic!("the filing dates publish: {e}"));
    let f2 = write(&lite, "formulas.jsonl", &formula_lines());
    run_checked(&lite, &f2, T1_).unwrap_or_else(|e| panic!("the formulas define, with their tests, under the public rule: {e}"));
    build_served(lite, None, |_| {}).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_formulas_define_with_tests_and_are_bound() {
    let m = mesh("h0410-lb-def").await;
    for f in [latest(), earliest()] {
        assert!(m.lite.store.resolve_binding(&f, END).unwrap().is_some(), "{f} is unbound");
    }
    assert!(m.lite.store.resolve_binding(&filed(), END).unwrap().is_some(), "the class method is bound");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cell_evaluates_the_member_of_the_latest_filing() {
    let m = mesh("h0410-lb-cell").await;
    let v = m.get(&format!("/cell/{}", call_path(&call(&latest(), "C1")))).await;
    assert_eq!((v["outcome"].as_str(), v["receipt"]["value"].as_str()), (Some("ok"), Some("20")), "{v}");
    // two slices read: each is an input
    assert_eq!(v["receipt"]["inputs"].as_array().map(|a| a.len()), Some(2), "{v}");
    let v = m.get(&format!("/cell/{}", call_path(&call(&latest(), "C2")))).await;
    assert_eq!(v["receipt"]["value"].as_str(), Some("5"), "{v}");
    let v = m.get(&format!("/cell/{}", call_path(&call(&earliest(), "C1")))).await;
    assert_eq!(v["receipt"]["value"].as_str(), Some("10"), "{v}");
    // no candidate: absent
    let r = m.get_any(&format!("/cell/{}", call_path(&call(&latest(), "ZZ")))).await;
    assert_ne!(r.json["outcome"], "ok", "{}", r.json);
    assert!(String::from_utf8_lossy(&r.body).contains("E_FORMULA_ABSENT"), "{}", String::from_utf8_lossy(&r.body));
}

async fn calc(m: &Served, formula: &str) -> (u16, Value) {
    let r = m.post_raw("/x/calc", serde_json::to_vec(&json!({ "formula": formula })).unwrap()).await;
    (r.status, r.json)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn x_calc_evaluates_last_by_and_first_by() {
    let m = mesh("h0410-lb-calc").await;
    let f = format!("last_by({}(cik: \"C1\", accession: *), {}(accession: *))", fact(), filed());
    let (s, j) = calc(&m, &f).await;
    let a = j.get("calculation").unwrap_or(&j);
    assert_eq!((s, a["value"].as_str()), (200, Some("20")), "{j}");
    assert!(a["lineage"].as_array().is_some_and(|l| l.len() >= 2), "both slices are read: {j}");
    let f = format!("first_by({}(cik: \"C1\", accession: *), {}(accession: *))", fact(), filed());
    let (s, j) = calc(&m, &f).await;
    let a = j.get("calculation").unwrap_or(&j);
    assert_eq!((s, a["value"].as_str()), (200, Some("10")), "{j}");
    // through the stored formula
    let (s, j) = calc(&m, &call(&latest(), "C1")).await;
    let a = j.get("calculation").unwrap_or(&j);
    assert_eq!((s, a["value"].as_str()), (200, Some("20")), "{j}");
    // and arithmetic around it
    let (s, j) = calc(&m, &format!("{} + 1", call(&latest(), "C2"))).await;
    let a = j.get("calculation").unwrap_or(&j);
    assert_eq!((s, a["value"].as_str()), (200, Some("6")), "{j}");
}
