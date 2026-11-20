//! http 0.4.3 (CONTRACT-build2-part5-feeds F2; QUESTIONS-build2 D16, D20, D21): source-conformance
//! tests of STORED functions.
//!
//! A relay is a stored function, so the public rule's formula part does not apply to it; its tests
//! are source-conformance tests: a call, with an expected value taken from a second, independent
//! copy of the source data. What these tests specify:
//!
//! * a `test` line for a stored function is stored with the function (table `function_tests`, case
//!   id = first 16 hex digits of the SHA-256 of the test's JCS, spec 7.3), whatever file carries it
//!   (with the cells, in a tests-only file, in a bundle's `tests`), once per case;
//! * it is run against the stored value of the cell the call names; `describe` counts it
//!   (`tests.count`; `branches` and `covered` 0, a stored function has no body), the function page
//!   lists it with its result (`tests.cases`, `tests.last_run.result`: `pass`, `fail`, `error`);
//! * a test whose expectation differs from the stored value is `fail`; a call that names a cell that
//!   is not stored is `error` (a conformance test that cannot be checked is not a pass); both are
//!   open counterexamples in `describe` and never a refusal (stored functions stay exempt from the
//!   public rule: the file is published);
//! * a new version of a tested cell reruns the tests of its function; the result follows the latest
//!   value;
//! * with `DATUMESH_TEST_CELLS=1` each test is also a `<host>/test_case(function_hash, case)` cell,
//!   trust `primary`, value `true` (pass) or `false` (fail or error);
//! * a computed function keeps exactly its behaviour.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use calculus::*;
use common::lite::*;
use common::pub031::{function_line, T1, T2};
use common::readpath::*;
use common::world::call_path;
use datumesh_store::Store;
use publish4::*;
use serde_json::{json, Value};

const FX: &str = "mesh-test.datumesh.com/scratch_b5/relay/fx";

fn fx_sig() -> String {
    r#"{"args":{"currency":{"type":"string","valid":false},"date":{"type":"date","valid":false}},"order":["currency","date"],"returns":{"type":"number","unit":"1"}}"#
        .to_string()
}

fn src() -> Value {
    json!({"name": "ecb.europa.eu", "url": "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-hist.zip",
        "retrieved": "2024-03-16T10:00:00.000000Z"})
}

fn fx_cell(currency: &str, date: &str, value: &str) -> String {
    json!({"cell": {"fn": FX, "args": {"currency": {"type": "string", "value": currency},
            "date": {"type": "date", "value": date}}},
        "licence": "CC0-1.0", "summary": "A reference rate.", "source": src(),
        "value": {"type": "number", "value": value}})
    .to_string()
}

fn call(currency: &str, date: &str) -> String {
    format!("{FX}(currency: \"{currency}\", date: {date})")
}

fn t(currency: &str, date: &str, want: &str) -> String {
    test_line(FX, &call(currency, date), None, None, num(want))
}

fn data() -> Vec<String> {
    vec![
        function_line(FX, &fx_sig(), None),
        fx_cell("USD", "2024-03-15", "1.0892"),
        fx_cell("GBP", "2024-03-15", "0.8567"),
    ]
}

fn good_tests() -> Vec<String> {
    vec![t("USD", "2024-03-15", "1.0892"), t("GBP", "2024-03-15", "0.8567")]
}

fn describe_path(name: &str) -> String {
    format!("/describe/{}", call_path(name))
}

async fn tests_of(m: &Served) -> Value {
    m.get(&describe_path(FX)).await["tests"].clone()
}

fn counts(count: u64, counterexamples: u64) -> Value {
    json!({"count": count, "branches": 0, "covered": 0, "counterexamples": counterexamples})
}

async fn page(m: &Served) -> Value {
    let r = publish4::get(m, &format!("/fn/{FX}"), &[("accept", "application/json")]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()
}

fn with(mut a: Vec<String>, b: Vec<String>) -> Vec<String> {
    a.extend(b);
    a
}

// ---- 1: stored, counted, listed ------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_tests_of_a_stored_function_are_stored_counted_and_listed() {
    let m = serve_lite("sc-stored", &with(data(), good_tests()), None, |_| {}).await;
    assert_eq!(tests_of(&m).await, counts(2, 0));
    // the store holds them, by case id
    let hash = fn_hash(&m, FX);
    let rows = m.lite.store.function_tests(&hash).unwrap();
    let mut got: Vec<String> = rows.iter().map(|r| r.case.clone()).collect();
    got.sort();
    let mut want: Vec<String> = good_tests().iter().map(|l| case_id(l)).collect();
    want.sort();
    assert_eq!(got, want);
    assert!(rows.iter().all(|r| r.last_result.as_deref() == Some("pass")), "{rows:?}");
    // core accepts the describe answer
    let r = m.get_any(&describe_path(FX)).await;
    let held =
        datumesh_core::FunctionDescriptor::from_json(&m.get_any(&format!("/descriptor/{hash}")).await.body).unwrap();
    let got = datumesh_core::check_describe_with(&r.body, &std::collections::BTreeMap::new(), &[held])
        .unwrap_or_else(|e| panic!("{e}"));
    let x = got.tests.expect("tests");
    assert_eq!((x.count, x.branches, x.covered, x.counterexamples), (2, 0, 0, 0));
    // the function page lists them with their result
    let j = page(&m).await;
    assert_eq!(j["tests"]["count"], 2, "{j}");
    assert_eq!(j["tests"]["last_run"]["result"], "pass", "{j}");
    let cases = j["tests"]["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 2, "{j}");
    for l in good_tests() {
        let v: Value = serde_json::from_str(&l).unwrap();
        assert!(cases.iter().any(|c| c["call"] == v["call"]), "{v}: {j}");
    }
}

// ---- 2: failing and erroring tests ---------------------------------------------------------------------

/// Run with the public rule (publish_stored_checked): stored functions are exempt, the file is published.
fn published_checked(label: &str, lines: &[String]) -> Lite {
    let lite = Lite::new(label);
    let f = common::pub031::write(&lite, "f.jsonl", lines);
    run_checked(&lite, &f, NOW).unwrap_or_else(|e| panic!("a stored function's tests must never refuse: {e}"));
    assert!(lite.bound_tx(FX).is_some());
    lite
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_test_that_differs_from_the_stored_value_fails_and_is_a_counterexample() {
    let lite =
        published_checked("sc-fail", &with(data(), vec![t("USD", "2024-03-15", "1.0893"), good_tests()[1].clone()]));
    let m = build_served(lite, None, |_| {}).await;
    assert_eq!(tests_of(&m).await, counts(2, 1));
    let hash = fn_hash(&m, FX);
    let rows = m.lite.store.function_tests(&hash).unwrap();
    let bad = rows.iter().find(|r| r.case == case_id(&t("USD", "2024-03-15", "1.0893"))).expect("held");
    assert_eq!(bad.last_result.as_deref(), Some("fail"), "{rows:?}");
    assert_eq!(page(&m).await["tests"]["last_run"]["result"], "fail");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_test_of_a_cell_that_is_not_stored_errors_and_is_a_counterexample() {
    let missing = t("JPY", "2024-03-15", "162.1");
    let lite = published_checked("sc-error", &with(data(), vec![missing.clone(), good_tests()[0].clone()]));
    let m = build_served(lite, None, |_| {}).await;
    assert_eq!(tests_of(&m).await, counts(2, 1));
    let hash = fn_hash(&m, FX);
    let rows = m.lite.store.function_tests(&hash).unwrap();
    let r = rows.iter().find(|r| r.case == case_id(&missing)).expect("held");
    assert_eq!(r.last_result.as_deref(), Some("error"), "{rows:?}");
    assert_eq!(page(&m).await["tests"]["last_run"]["result"], "error");
}

// ---- 3: a later, tests-only file; the same test twice ---------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tests_may_come_in_a_later_file_and_the_same_test_is_stored_once() {
    let first = vec![good_tests()[0].clone(), good_tests()[0].clone()]; // twice in one file
    let m = serve_runs(
        "sc-later",
        &[(NOW, with(data(), first)), (T1, vec![good_tests()[0].clone(), good_tests()[1].clone()]), (T2, good_tests())],
        None,
    )
    .await;
    assert_eq!(tests_of(&m).await, counts(2, 0));
    assert_eq!(m.lite.store.function_tests(&fn_hash(&m, FX)).unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tests_only_file_for_an_existing_stored_function_is_stored_and_counted() {
    let m = serve_runs("sc-testsonly", &[(NOW, data()), (T1, good_tests())], None).await;
    assert_eq!(tests_of(&m).await, counts(2, 0));
}

// ---- 4: rerun on a new version ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_version_of_a_tested_cell_reruns_the_test() {
    let m = serve_runs(
        "sc-rerun",
        &[
            (NOW, with(data(), good_tests())),
            (T1, vec![fx_cell("USD", "2024-03-15", "1.09")]),
            (T2, vec![fx_cell("USD", "2024-03-15", "1.0892")]),
        ],
        None,
    )
    .await;
    // the last version is the first value again: back to passing
    assert_eq!(tests_of(&m).await, counts(2, 0));
    // and in between it failed: publish the changed value alone and look
    let m = serve_runs(
        "sc-rerun2",
        &[(NOW, with(data(), good_tests())), (T1, vec![fx_cell("USD", "2024-03-15", "1.09")])],
        None,
    )
    .await;
    assert_eq!(tests_of(&m).await, counts(2, 1));
    assert_eq!(page(&m).await["tests"]["last_run"]["result"], "fail");
}

// ---- 5: test cells -----------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn on_mesh_test_each_stored_function_test_is_a_cell() {
    let wrong = t("USD", "2024-03-15", "1.0893");
    let m = serve_lite(
        "sc-cells",
        &with(data(), with(good_tests(), vec![wrong.clone()])),
        Some(test_cells_config()),
        |_| {},
    )
    .await;
    let hash = fn_hash(&m, FX);
    for (line, want) in [(&good_tests()[0], true), (&good_tests()[1], true), (&wrong, false)] {
        let v = m.get(&format!("/cell/{}", call_path(&test_case_call(&hash, &case_id(line))))).await;
        assert_eq!(v["outcome"], "ok", "{line}: {v}");
        assert_eq!(v["receipt"]["type"], "boolean", "{v}");
        assert_eq!(v["receipt"]["value"], json!(want), "{line}: {v}");
        assert_eq!(v["trust"], "primary", "{v}");
        assert!(v["receipt"].get("source").is_none(), "{v}");
    }
}

// ---- 6: a computed function is as it was ---------------------------------------------------------------------

#[test]
fn a_computed_function_keeps_the_public_rule() {
    let c = calc();
    let mut v = data_lines(&c);
    v.push(per_unit_line(&c, None));
    // without tests: refused, as before
    refused("sc-computed-none", &v, 7, "E_TESTS_MISSING", &[c.per_unit.as_str(), "zero(units)"]);
    // a failing one: refused, as before
    let mut t = per_unit_tests(&c);
    t[0] = test_line(&c.per_unit, &pu_call(&c, "N", "2"), None, None, num("6"));
    let all = with(v.clone(), t);
    refused("sc-computed-fail", &all, 8, "E_TESTS_FAILING", &[]);
    // full tests: accepted
    let (_, r) = checked("sc-computed-ok", &with(v, per_unit_tests(&c)));
    r.unwrap_or_else(|e| panic!("{e}"));
}

// ---- 7: bundle ----------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bundle_carries_the_tests_of_a_stored_function() {
    let stub = Stub::start(PUB).await;
    let p = public("sc-bundle", &[stub.peer()], Some(PUB)).await;
    let name = level_name(PUB);
    let good = test_line(&name, &format!("{name}(site: \"N\")"), None, None, num("42"));
    let bad = test_line(&name, &format!("{name}(site: \"N\")"), None, None, num("43"));
    let b = datumesh_http::bundle::build_with_tests(
        &ns_of(PUB),
        vec![stored_fn().canonical()],
        vec![(name.clone(), stored_fn().hash())],
        vec![level_receipt(PUB, "N", "42", TX)],
        vec![],
        vec![good.clone().into_bytes(), bad.into_bytes()],
        &common::seed_for(PUB),
    )
    .unwrap();
    let r = p.post(b).await;
    assert_eq!(r.status, 201, "stored functions stay exempt from the public rule: {}", r.text());
    let d = p.get(&describe_path(&name), &[]).await.json();
    assert_eq!(d["tests"], counts(2, 1), "{d}");
    let page = p.get(&format!("/fn/{name}"), &[("accept", "application/json")]).await.json();
    assert_eq!(page["tests"]["count"], 2, "{page}");
    assert_eq!(page["tests"]["last_run"]["result"], "fail", "{page}");
}

// ---- 8: two functions with one descriptor (QUESTIONS-build2 D23, http 0.4.4) ----------------------------------

const FY: &str = "mesh-test.datumesh.com/scratch_b5/relay/fy";

fn y_cell(currency: &str, date: &str, value: &str) -> String {
    fx_cell(currency, date, value).replace(FX, FY)
}

fn y_call(currency: &str, date: &str) -> String {
    format!("{FY}(currency: \"{currency}\", date: {date})")
}

fn yt(currency: &str, date: &str, want: &str) -> String {
    test_line(FY, &y_call(currency, date), None, None, num(want))
}

/// FX and FY: the same signature (so the same function hash), each with its own cells; FX has two
/// good tests, FY one good and one failing.
fn twins() -> Vec<String> {
    let mut v = data();
    v.push(function_line(FY, &fx_sig(), None));
    v.push(y_cell("USD", "2024-03-15", "2"));
    v.push(y_cell("GBP", "2024-03-15", "3"));
    v.extend(good_tests());
    v.push(yt("USD", "2024-03-15", "2"));
    v.push(yt("GBP", "2024-03-15", "3.5"));
    v
}

async fn tests_named(m: &Served, name: &str) -> Value {
    m.get(&describe_path(name)).await["tests"].clone()
}

async fn page_of(m: &Served, name: &str) -> Value {
    let r = publish4::get(m, &format!("/fn/{name}"), &[("accept", "application/json")]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()
}

fn calls_of(page: &Value) -> Vec<String> {
    let mut v: Vec<String> = page["tests"]["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .map(|c| c["call"].as_str().unwrap().to_string())
        .collect();
    v.sort();
    v
}

async fn check_twins(m: &Served, fx: (u64, u64, &str), fy: (u64, u64, &str)) {
    assert_eq!(fn_hash(m, FX), fn_hash(m, FY), "the two share one descriptor");
    assert_eq!(tests_named(m, FX).await, counts(fx.0, fx.1));
    assert_eq!(tests_named(m, FY).await, counts(fy.0, fy.1));
    let (px, py) = (page_of(m, FX).await, page_of(m, FY).await);
    assert_eq!((px["tests"]["count"].as_u64(), py["tests"]["count"].as_u64()), (Some(fx.0), Some(fy.0)));
    assert_eq!(px["tests"]["counterexamples"].as_u64(), Some(fx.1));
    assert_eq!(py["tests"]["counterexamples"].as_u64(), Some(fy.1));
    assert_eq!(px["tests"]["open_counterexamples"].as_array().unwrap().len() as u64, fx.1, "{px}");
    assert_eq!(py["tests"]["open_counterexamples"].as_array().unwrap().len() as u64, fy.1, "{py}");
    assert!(calls_of(&px).iter().all(|c| c.starts_with(FX)), "{px}");
    assert!(calls_of(&py).iter().all(|c| c.starts_with(FY)), "{py}");
    assert_eq!(px["tests"]["last_run"]["result"], fx.2, "{px}");
    assert_eq!(py["tests"]["last_run"]["result"], fy.2, "{py}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_stored_functions_with_one_descriptor_keep_their_own_tests() {
    let lite = published_checked("sc-twins", &twins());
    let m = build_served(lite, None, |_| {}).await;
    check_twins(&m, (2, 0, "pass"), (2, 1, "fail")).await;
    let py = page_of(&m, FY).await;
    let results: Vec<(String, String)> = py["tests"]["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["call"].as_str().unwrap().to_string(), c["result"].as_str().unwrap().to_string()))
        .collect();
    assert!(results.contains(&(y_call("GBP", "2024-03-15"), "fail".to_string())), "{results:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_version_reruns_only_the_tests_naming_that_function() {
    // FY's failing cell is corrected: FY passes, FX is as it was.
    let m =
        serve_runs("sc-twins-rerun-y", &[(NOW, twins()), (T1, vec![y_cell("GBP", "2024-03-15", "3.5")])], None).await;
    check_twins(&m, (2, 0, "pass"), (2, 0, "pass")).await;
    // FX's cell changes: FX fails, FY keeps its own result.
    let m =
        serve_runs("sc-twins-rerun-x", &[(NOW, twins()), (T1, vec![fx_cell("USD", "2024-03-15", "1.5")])], None).await;
    check_twins(&m, (2, 1, "fail"), (2, 1, "fail")).await;
    // both change in one run, in opposite directions
    let m = serve_runs(
        "sc-twins-rerun-both",
        &[(NOW, twins()), (T1, vec![fx_cell("USD", "2024-03-15", "1.5"), y_cell("GBP", "2024-03-15", "3.5")])],
        None,
    )
    .await;
    check_twins(&m, (2, 1, "fail"), (2, 0, "pass")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_cells_of_twins_are_unaffected() {
    let m = serve_lite("sc-twins-cells", &twins(), Some(test_cells_config()), |_| {}).await;
    let hash = fn_hash(&m, FX);
    let wrong = yt("GBP", "2024-03-15", "3.5");
    let all = [
        (good_tests()[0].clone(), true),
        (good_tests()[1].clone(), true),
        (yt("USD", "2024-03-15", "2"), true),
        (wrong, false),
    ];
    for (line, want) in &all {
        let v = m.get(&format!("/cell/{}", call_path(&test_case_call(&hash, &case_id(line))))).await;
        assert_eq!(v["outcome"], "ok", "{line}: {v}");
        assert_eq!(v["receipt"]["value"], json!(want), "{line}: {v}");
    }
    check_twins(&m, (2, 0, "pass"), (2, 1, "fail")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_computed_functions_with_one_body_keep_their_own_tests() {
    let c = calc();
    let mut c2 = c.clone();
    c2.per_unit = format!("{PFX}/per_unit_twin");
    let mut lines = data_lines(&c);
    lines.push(per_unit_line(&c, None));
    lines.push(per_unit_line(&c2, None));
    lines.extend(per_unit_tests(&c));
    lines.extend(per_unit_tests(&c2));
    let (lite, r) = checked("sc-computed-twins", &lines);
    r.unwrap_or_else(|e| panic!("{e}"));
    let m = build_served(lite, None, |_| {}).await;
    assert_eq!(fn_hash(&m, &c.per_unit), fn_hash(&m, &c2.per_unit), "one body, one hash");
    for f in [&c.per_unit, &c2.per_unit] {
        let t = tests_named(&m, f).await;
        assert_eq!(t, json!({"count": 6, "branches": 2, "covered": 2, "counterexamples": 0}), "{f}: {t}");
        let p = page_of(&m, f).await;
        assert_eq!(p["tests"]["count"], 6, "{p}");
        assert!(calls_of(&p).iter().all(|x| x.starts_with(f.as_str())), "{p}");
        assert_eq!(p["tests"]["last_run"]["result"], "pass", "{p}");
    }
}
