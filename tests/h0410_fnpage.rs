//! http 0.4.10, CONTRACT-round-0.7-P HP5: the function page and `describe` of a function take their numbers
//! from the test counter, list at most 100 of each kind, and read a test cell by its key. Written first; FAIL
//! until then (and, for the counter, until store 0.5.3: with 0.5.2 the numbers come from reading the rows).
//!
//! The reads are counted by the probe of `common/h0410.rs` (the store is sealed, so there is no wrapper that
//! can make `function_tests` fail): the number of rows of `function_tests` the mesh visits.
//!
//! # JSON fields chosen here (the contract names the cap and the wording, not the members)
//!
//! `GET /fn/<name>` (JSON), the `tests` object keeps `count`, `branches`, `covered`, `cases`, `cases_total`,
//! `last_run` and `counterexamples` (a number), and:
//!
//! * `counterexamples`: the stored failing and erroring tests (exact, from the counter) plus the foreign `false`
//!   test cells (at most 100 read), as a number;
//! * `counterexamples_more`: `true` when the foreign ones were cut at 100 (so the number is "100 or more" on
//!   that side), otherwise `false`;
//! * `open_counterexamples`: at most 100 stored ones followed by at most 100 foreign ones, each
//!   `{"signer", "case"}`.
//!
//! `lineage.dependants`: at most 100 links; `lineage.dependants_more`: `true` when there are more. The HTML
//! says `100 or more` beside the counterexamples when `counterexamples_more`, and `and more` after the
//! dependants when `dependants_more`. `GET /describe/<function>`'s `tests` stays the four numbers of core's
//! `DescribeTests`; its `counterexamples` is the same number as the page's.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/h0410.rs"]
mod h0410;

use calculus::*;
use common::lite::*;
use common::pub031::{function_line, stored_descriptor};
use common::world::call_path;
use datumesh_core::{Cell, ReceiptBuilder, Scalar, Type};
use datumesh_store::FunctionTest;
use h0410::*;
use serde_json::{json, Value};

const F: &str = "mesh-test.datumesh.com/h0410/fnp/level";
const G: &str = "mesh-test.datumesh.com/h0410/fnp/other";
const BULK: usize = 997;
const FAILING: usize = 130;

fn call(f: &str, site: &str) -> String {
    format!("{f}(site: \"{site}\")")
}

fn tline(f: &str, site: &str, want: &str) -> String {
    test_line(f, &call(f, site), None, None, num(want))
}

fn jcs(line: &str) -> Vec<u8> {
    datumesh_core::json::parse(line.as_bytes()).unwrap().to_jcs()
}

/// F with three cells and three tests that pass when published, plus `BULK` more test rows written straight
/// into the store: the first `FAILING` recorded as failing, the others as passing; all named F.
fn level_with_tests(fx: &Fx) -> String {
    let mut lines = vec![function_line(F, &reading_sig(), None)];
    for s in ["N", "S", "E"] {
        lines.push(site_cell(F, s, "10"));
    }
    for s in ["N", "S", "E"] {
        lines.push(tline(F, s, "10"));
    }
    fx.publish(&lines, NOW);
    let hash = fx.hash_of(F);
    let (store, h) = (fx.store.clone(), hash.clone());
    common::on_thread(move || {
        let mut names = Vec::new();
        for i in 0..BULK {
            let line = tline(F, &format!("o{i:04}"), "1");
            let case = case_id(&line);
            let mut t = FunctionTest::new(&h, &case, &jcs(&line), HOST);
            t.last_run_at = Some("2026-01-01T00:00:00.000000Z".to_string());
            t.last_result = Some(if i < FAILING { "fail" } else { "pass" }.to_string());
            t.last_detail = Some(String::new());
            store.put_function_test(&t).unwrap();
            names.push((case, F.to_string()));
        }
        store.name_function_tests(&h, &names).unwrap();
    });
    hash
}

async fn serve_held(fx: Fx, cfg_extra: &[(&str, &str)]) -> W {
    fx.probe().arm();
    let mut vars = held(&[]);
    vars.extend_from_slice(cfg_extra);
    fx.serve(config(&vars, &[])).await
}

// ---- numbers from the counter, a bounded read -------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describe_of_a_function_takes_its_numbers_from_the_counter() {
    let fx = Fx::pg("hp5-describe");
    level_with_tests(&fx);
    let w = serve_held(fx, &[]).await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let before = w.fx.probe().reads(Group::Tests);
    let r = w.get(&format!("/describe/{}", call_path(F))).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        r.json()["tests"],
        json!({"count": 3 + BULK, "branches": 0, "covered": 0, "counterexamples": FAILING}),
        "{}",
        r.text()
    );
    let read = w.fx.probe().reads(Group::Tests) - before;
    assert!(read <= 2, "a describe of a function with {} test rows visited {read} of them", 3 + BULK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_function_page_reads_at_most_what_it_shows() {
    let fx = Fx::pg("hp5-page");
    level_with_tests(&fx);
    let w = serve_held(fx, &[]).await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let before = w.fx.probe().reads(Group::Tests);
    let r = w.get_h(&format!("/fn/{F}?format=json"), &[("accept", "application/json")]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let t = r.json()["tests"].clone();
    assert_eq!(t["count"], 3 + BULK, "{}", t["count"]);
    assert_eq!(t["cases_total"], 3 + BULK);
    assert_eq!(t["cases"].as_array().unwrap().len(), 100, "the first 100 cases");
    assert_eq!(t["counterexamples"], FAILING, "the stored failing ones, exact");
    assert_eq!(t["counterexamples_more"], false, "{t}");
    let open = t["open_counterexamples"].as_array().unwrap();
    assert_eq!(open.len(), 100, "at most 100 are listed: {}", open.len());
    assert!(open.iter().all(|o| o["signer"] == HOST && o["case"].is_string()), "{t}");
    // 100 cases, 100 failing ones, and room for the rest of what the page needs
    let read = w.fx.probe().reads(Group::Tests) - before;
    assert!(read <= 250, "the page visited {read} test rows; it shows at most 200");
    // the HTML says how many there are
    let html = w.get_h(&format!("/fn/{F}"), &[("accept", "text/html")]).await.text();
    assert!(html.contains(&format!("{FAILING} counterexamples")), "{html}");
}

// ---- the test cell by its key -----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_test_cell_is_read_by_its_key_not_by_reading_every_test() {
    let fx = Fx::pg("hp5-cell");
    let hash = level_with_tests(&fx);
    fx.probe().arm();
    let mut vars = held(&[]);
    vars.push(("DATUMESH_TEST_CELLS", "1"));
    let w = fx.serve(config(&vars, &[])).await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let case = case_id(&tline(F, "N", "10"));
    let before = w.fx.probe().reads(Group::Tests);
    let r = w.get(&format!("/cell/{}", call_path(&test_case_call(F, &hash, &case)))).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = r.json();
    assert_eq!(v["outcome"], "ok", "{v}");
    assert_eq!(v["receipt"]["value"], json!(true), "{v}");
    let read = w.fx.probe().reads(Group::Tests) - before;
    assert!(read <= 3, "a test cell of a function with {} test rows read {read} of them", 3 + BULK);
    // a case no test of the function has is still not a value
    let r = w.get(&format!("/cell/{}", call_path(&test_case_call(F, &hash, "0000000000000000")))).await;
    assert_ne!(r.json()["outcome"], "ok", "{}", r.text());
}

// ---- foreign counterexamples ----------------------------------------------------------------------------------

const OTHER: &str = "pub.example";

fn test_case_signature() -> String {
    r#"{"args":{"case":{"type":"string","valid":false},"function_hash":{"type":"string","valid":false}},"order":["function_hash","case"],"returns":"boolean"}"#
        .to_string()
}

/// `n` signed `false` `<OTHER>/test_case` cells for `fn_hash`, put in the store as an import would.
fn hold_foreign(fx: &Fx, fn_hash: &str, n: usize) {
    let key = common::key_for(OTHER);
    let (store, hash) = (fx.store.clone(), fn_hash.to_string());
    common::on_thread(move || {
        if store.namespace(OTHER).unwrap().is_none() {
            store.put_namespace(OTHER, &common::key_doc(OTHER, &key), K_FROM).unwrap();
        }
        let d =
            datumesh_core::FunctionDescriptor::from_json(stored_descriptor(&test_case_signature()).as_bytes()).unwrap();
        store.put_descriptor(&d.canonical()).unwrap();
        for i in 0..n {
            let tx = format!("2026-11-15T11:00:00.{i:06}Z");
            let args = common::args(&[("case", common::s(&format!("{i:016x}"))), ("function_hash", common::s(&hash))]);
            let cell = Cell { ns: OTHER.to_string(), fn_name: "test_case".into(), fn_hash: d.hash(), args };
            let id = cell.id();
            let r = ReceiptBuilder::new()
                .cell(cell)
                .value(Scalar::from_lexical(Type::Boolean, "false").unwrap())
                .valid(common::args(&[]))
                .asof(&tx)
                .tx(&tx)
                .issued(&tx)
                .publisher(OTHER)
                .signer_host(OTHER)
                .licence("CC0-1.0")
                .summary("A test of someone else's function.")
                .sign(&key)
                .unwrap();
            store.put_receipt(&common::receipt_json(&r)).unwrap();
            store.put_version(&id, &tx, r.hash()).unwrap();
        }
        let name = format!("{OTHER}/test_case");
        if store.resolve_name(&name, END).unwrap().is_none() {
            store.bind_name(&name, &d.hash(), "2000-01-01T00:00:00.000000Z").unwrap();
        }
    });
}

async fn page_and_html(w: &W, name: &str) -> (Value, String, Value) {
    let page = w.get_h(&format!("/fn/{name}?format=json"), &[("accept", "application/json")]).await;
    assert_eq!(page.status, 200, "{}", page.text());
    let html = w.get_h(&format!("/fn/{name}"), &[("accept", "text/html")]).await.text();
    let d = w.get(&format!("/describe/{}", call_path(name))).await;
    assert_eq!(d.status, 200, "{}", d.text());
    (page.json(), html, d.json())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn foreign_counterexamples_are_read_to_a_hundred_and_say_100_or_more() {
    let fx = Fx::lite("hp5-foreign");
    fx.publish(&[function_line(G, &reading_sig(), None), site_cell(G, "N", "10")], NOW);
    let hash = fx.hash_of(G);
    hold_foreign(&fx, &hash, 150);
    let w = fx.serve(config(&held(&[]), &[])).await;
    let (j, html, d) = page_and_html(&w, G).await;
    let t = &j["tests"];
    assert_eq!(t["counterexamples"], 100, "{t}");
    assert_eq!(t["counterexamples_more"], true, "{t}");
    let open = t["open_counterexamples"].as_array().unwrap();
    assert_eq!(open.len(), 100, "{t}");
    assert!(open.iter().all(|o| o["signer"] == OTHER), "{t}");
    assert!(html.contains("100 or more"), "the page says it: {html}");
    assert_eq!(d["tests"]["counterexamples"], 100, "{d}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_few_foreign_counterexamples_are_counted_exactly() {
    let fx = Fx::lite("hp5-foreign-few");
    fx.publish(&[function_line(G, &reading_sig(), None), site_cell(G, "N", "10")], NOW);
    let hash = fx.hash_of(G);
    hold_foreign(&fx, &hash, 3);
    let w = fx.serve(config(&held(&[]), &[])).await;
    let (j, html, d) = page_and_html(&w, G).await;
    let t = &j["tests"];
    assert_eq!(t["counterexamples"], 3, "{t}");
    assert_eq!(t["counterexamples_more"], false, "{t}");
    assert_eq!(t["open_counterexamples"].as_array().unwrap().len(), 3, "{t}");
    assert!(!html.contains("100 or more"), "{html}");
    assert_eq!(d["tests"]["counterexamples"], 3, "{d}");
}

// ---- dependants ----------------------------------------------------------------------------------------------

async fn with_dependants(label: &str, n: usize) -> (W, Value, String) {
    let fx = Fx::lite(label);
    let mut lines = vec![function_line(F, &reading_sig(), None), site_cell(F, "N", "10")];
    for i in 0..n {
        lines.push(formula_line(
            &format!("mesh-test.datumesh.com/h0410/fnp/dep{i:03}"),
            &format!("{F}(site: site)"),
            &reading_sig(),
            None,
        ));
    }
    fx.publish(&lines, NOW);
    let w = fx.serve(config(&held(&[]), &[])).await;
    let j = w.get_h(&format!("/fn/{F}?format=json"), &[("accept", "application/json")]).await;
    assert_eq!(j.status, 200, "{}", j.text());
    let html = w.get_h(&format!("/fn/{F}"), &[("accept", "text/html")]).await.text();
    let json = j.json();
    (w, json, html)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dependants_are_listed_to_a_hundred_and_then_and_more() {
    let (_w, j, html) = with_dependants("hp5-deps", 130).await;
    let l = &j["lineage"];
    assert_eq!(l["dependants"].as_array().unwrap().len(), 100, "{l}");
    assert_eq!(l["dependants_more"], true, "{l}");
    assert!(html.contains("and more"), "{html}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_few_dependants_are_all_listed_with_no_more() {
    let (_w, j, html) = with_dependants("hp5-deps-few", 3).await;
    let l = &j["lineage"];
    assert_eq!(l["dependants"].as_array().unwrap().len(), 3, "{l}");
    assert_eq!(l["dependants_more"], false, "{l}");
    assert!(!html.contains("and more"), "{html}");
}
