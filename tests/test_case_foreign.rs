//! http 0.4.2 (spec-0.6.1, 7.3, CONTRACT-build2-spec-0.6.1 S2; QUESTIONS D16): the `test_case`
//! convention.
//!
//! * One `test_case` function per host, under the host itself: on a mesh with `DATUMESH_TEST_CELLS=1`
//!   the test cells are `<host>/test_case(function_hash, case)`, never `<host>/<namespace>/test_case`,
//!   also for a function under `<host>/a/b` or under a DNS segment. Their trust is `primary` (D16:
//!   signed by the host, no inputs, no `source`).
//! * Anyone may publish `<their host>/test_case` for a function hash they do not own. A `false` cell
//!   of another host that this mesh holds, for a function hash of this mesh, is an open
//!   counterexample: it counts in `describe`'s `tests.counterexamples` (each cell once, beside the
//!   function's own failing tests) and the function page lists it with its signer. A `true` cell
//!   does not count, nor does a `false` one for another function hash.
//! * Without `DATUMESH_TEST_CELLS` no test cell is served; the counting needs no such switch.
//!
//! A foreign cell is put in the store as an import would: the host's key document, the descriptor of
//! `<host>/test_case`, the signed receipt and its version, the binding. (A bundle cannot carry a name
//! directly under a host: the format wants a namespace of at least host and one segment. See the
//! report.)

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use calculus::*;
use common::lite::*;
use common::pub031::*;
use common::readpath::*;
use common::world::call_path;
use datumesh_core::{Cell, ReceiptBuilder, Scalar, Type};
use datumesh_store::Store;
use publish4::get;
use serde_json::json;

const OTHER: &str = "pub.example";
const OTHER2: &str = "pub2.example";
const FOREIGN_TX: &str = "2026-11-15T11:00:00.000000Z";
const FOREIGN_TX2: &str = "2026-11-15T11:30:00.000000Z";

fn lines(c: &Calc) -> Vec<String> {
    let mut v = data_lines(c);
    v.push(per_unit_line(c, None));
    v.extend(per_unit_tests(c));
    v.push(band_line(c));
    v.extend(band_tests(c));
    v
}

fn describe_path(name: &str) -> String {
    format!("/describe/{}", call_path(name))
}

/// The signature of `<host>/test_case` (7.3).
fn test_case_signature() -> String {
    r#"{"args":{"case":{"type":"string","valid":false},"function_hash":{"type":"string","valid":false}},"order":["function_hash","case"],"returns":"boolean"}"#
        .to_string()
}

/// Puts a signed `<host>/test_case(function_hash, case)` cell of `host` in the store: what an
/// import of another host's signed cells leaves behind.
fn hold_foreign(lite: &Lite, host: &str, fn_hash: &str, case: &str, value: bool, tx: &str) {
    let key = common::key_for(host);
    if lite.store.namespace(host).unwrap().is_none() {
        lite.store.put_namespace(host, &common::key_doc(host, &key), K_FROM).unwrap();
    }
    let descriptor = stored_descriptor(&test_case_signature());
    let d = datumesh_core::FunctionDescriptor::from_json(descriptor.as_bytes()).unwrap();
    lite.store.put_descriptor(&d.canonical()).unwrap();
    let args = common::args(&[("case", common::s(case)), ("function_hash", common::s(fn_hash))]);
    let cell = Cell { ns: host.to_string(), fn_name: "test_case".into(), fn_hash: d.hash(), args };
    let id = cell.id();
    let r = ReceiptBuilder::new()
        .cell(cell)
        .value(Scalar::from_lexical(Type::Boolean, if value { "true" } else { "false" }).unwrap())
        .valid(common::args(&[]))
        .asof(tx)
        .tx(tx)
        .issued(tx)
        .publisher(host)
        .signer_host(host)
        .licence("CC0-1.0")
        .summary("A test of someone else's function.")
        .sign(&key)
        .unwrap();
    let bytes = common::receipt_json(&r);
    lite.store.put_receipt(&bytes).unwrap();
    lite.store.put_version(&id, tx, r.hash()).unwrap();
    let name = format!("{host}/test_case");
    if lite.store.resolve_name(&name, END).unwrap().is_none() {
        lite.store.bind_name(&name, &d.hash(), "2000-01-01T00:00:00.000000Z").unwrap();
    }
}

fn hash_in(lite: &Lite, name: &str) -> String {
    lite.store.resolve_binding(name, END).unwrap().unwrap_or_else(|| panic!("{name} is unbound")).fn_hash
}

/// The case id of a test the function does not hold: an expectation that is wrong.
fn foreign_case(c: &Calc) -> String {
    case_id(&test_line(&c.per_unit, &pu_call(c, "N", "5"), None, None, num("99")))
}

fn configs() -> Vec<(&'static str, Option<datumesh_http::Config>)> {
    vec![("production", None), ("test cells", Some(test_cells_config()))]
}

// ---- the test cells: host only, trust primary -----------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_cells_are_named_under_the_host_whatever_the_namespace_of_the_function() {
    for (i, prefix) in [
        "mesh-test.datumesh.com/a/b",
        "mesh-test.datumesh.com/relay/ecb.europa.eu",
        "mesh-test.datumesh.com/scratch_b3/calc",
    ]
    .into_iter()
    .enumerate()
    {
        let c = Calc::new(prefix);
        let ls = lines(&c);
        let m = serve_lite(&format!("tcf-host{i}"), &ls, Some(test_cells_config()), |_| {}).await;
        let hash = fn_hash(&m, &c.per_unit);
        for t in per_unit_tests(&c) {
            let call = test_case_call(&hash, &case_id(&t));
            assert!(call.starts_with(&format!("{HOST}/test_case(")), "{call}");
            let v = m.get(&format!("/cell/{}", call_path(&call))).await;
            assert_eq!(v["outcome"], "ok", "{prefix}: {t}: {v}");
            assert_eq!(v["trust"], "primary", "{prefix}: {v}");
            assert_eq!(v["receipt"]["cell"]["ns"], HOST, "{prefix}: {v}");
            assert_eq!(v["receipt"]["cell"]["fn"], "test_case", "{prefix}: {v}");
            assert_eq!(v["receipt"]["value"], json!(true), "{prefix}: {v}");
            assert!(v["receipt"].get("source").is_none(), "{v}");
            assert!(v["receipt"]["inputs"].as_array().is_none_or(|a| a.is_empty()), "{v}");
            // the receipt verifies as any receipt does
            let mut keys = datumesh_core::Keys::new();
            keys.insert_response(
                HOST,
                200,
                Some(&datumesh_http::keydoc::with_key(HOST, None, &common::key_for(HOST), K_FROM).unwrap()),
            );
            let r = datumesh_core::verify(&serde_json::to_vec(&v["receipt"]).unwrap(), &keys, None, None);
            assert!(r.ok, "{:?}", r.error());
        }
        // never a test_case in the function's own namespace
        let sub = format!("{prefix}/test_case");
        assert!(m.lite.store.resolve_name(&sub, END).unwrap().is_none(), "{sub} is bound");
        let sub_call = format!("{sub}(case: \"{}\", function_hash: \"{hash}\")", case_id(&per_unit_tests(&c)[0]));
        let r = m.get_any(&format!("/cell/{}", call_path(&sub_call))).await;
        assert_eq!((r.status, r.json["outcome"].as_str()), (404, Some("not_covered")), "{}", r.json);
        // and the host's name is bound, to the descriptor of 7.3
        let tc = m.lite.store.resolve_name(&format!("{HOST}/test_case"), END).unwrap();
        assert!(tc.is_some(), "{HOST}/test_case is not bound");
    }
}

/// Production behaviour is unchanged where nothing is configured: no test cell is served, the name
/// is unbound, and the tests are still counted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_the_switch_no_test_cell_is_served() {
    let c = Calc::new("mesh-test.datumesh.com/a/b");
    let m = serve_lite("tcf-prod", &lines(&c), None, |_| {}).await;
    let hash = fn_hash(&m, &c.per_unit);
    for t in per_unit_tests(&c) {
        let r = m.get_any(&format!("/cell/{}", call_path(&test_case_call(&hash, &case_id(&t))))).await;
        assert_eq!((r.status, r.json["outcome"].as_str()), (404, Some("not_covered")), "{}", r.json);
        assert_eq!(r.json["error"], "E_NAME_UNBOUND", "{}", r.json);
    }
    assert!(m.lite.store.resolve_name(&format!("{HOST}/test_case"), END).unwrap().is_none());
    assert_eq!(m.get(&describe_path(&c.per_unit)).await["tests"]["count"], 6);
}

// ---- counterexamples ---------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_foreign_false_test_cell_is_an_open_counterexample() {
    for (label, base) in configs() {
        let c = calc();
        let lite = Lite::new("tcf-ce");
        let f = write(&lite, "fixture.jsonl", &lines(&c));
        run(&lite, &f, NOW).unwrap();
        let (pu, band) = (hash_in(&lite, &c.per_unit), hash_in(&lite, &c.band));
        let local = case_id(&per_unit_tests(&c)[0]);
        let x = foreign_case(&c);
        hold_foreign(&lite, OTHER, &pu, &x, false, FOREIGN_TX); // counts for per_unit
        hold_foreign(&lite, OTHER, &pu, &local, true, FOREIGN_TX); // true: does not count
        hold_foreign(&lite, OTHER, &band, "00000000000000aa", false, FOREIGN_TX); // counts for band only
        hold_foreign(&lite, OTHER, &"ab".repeat(32), "00000000000000bb", false, FOREIGN_TX); // no function of ours
        let m = build_served(lite, base, |_| {}).await;

        let r = m.get_any(&describe_path(&c.per_unit)).await;
        assert_eq!(r.status, 200, "{label}: {}", String::from_utf8_lossy(&r.body));
        assert_eq!(
            r.json["tests"],
            json!({"count": 6, "branches": 2, "covered": 2, "counterexamples": 1}),
            "{label}: only the false cell of this hash counts: {}",
            r.json
        );
        // core 0.3.10 accepts the answer and reads the count
        let held =
            datumesh_core::FunctionDescriptor::from_json(&m.get_any(&format!("/descriptor/{pu}")).await.body).unwrap();
        let got = datumesh_core::check_describe_with(&r.body, &Default::default(), &[held])
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        let t = got.tests.expect("tests");
        assert_eq!((t.count, t.counterexamples), (6, 1), "{label}");
        // the band function has its own count, from its own false cell
        let b = m.get(&describe_path(&c.band)).await;
        assert_eq!(b["tests"]["counterexamples"], 1, "{label}: {b}");
        assert_eq!(b["tests"]["count"], 3, "{label}: {b}");
        // a function nothing was said about has none
        let q = m.get(&describe_path(&c.reading)).await;
        let n = q.get("tests").map_or(0, |t| t["counterexamples"].as_u64().unwrap());
        assert_eq!(n, 0, "{label}: {q}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_true_cell_or_a_cell_of_another_function_counts_for_nothing() {
    let c = calc();
    let lite = Lite::new("tcf-no");
    let f = write(&lite, "fixture.jsonl", &lines(&c));
    run(&lite, &f, NOW).unwrap();
    let (pu, band) = (hash_in(&lite, &c.per_unit), hash_in(&lite, &c.band));
    hold_foreign(&lite, OTHER, &pu, &case_id(&per_unit_tests(&c)[1]), true, FOREIGN_TX);
    hold_foreign(&lite, OTHER2, &pu, &case_id(&per_unit_tests(&c)[2]), true, FOREIGN_TX);
    hold_foreign(&lite, OTHER, &band, "00000000000000aa", false, FOREIGN_TX);
    let m = build_served(lite, None, |_| {}).await;
    let d = m.get(&describe_path(&c.per_unit)).await;
    assert_eq!(d["tests"]["counterexamples"], 0, "{d}");
    let page = get(&m, &format!("/fn/{}?format=json", c.per_unit), &[]).await.json();
    assert_eq!(page["tests"]["counterexamples"], 0, "{page}");
    assert!(!page["tests"].to_string().contains(OTHER), "a true cell is not listed: {page}");
    assert!(!page["tests"].to_string().contains(OTHER2), "a true cell is not listed: {page}");
}

/// Each cell counts once, from any host, beside the function's own failing tests.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn own_failures_and_foreign_cells_add_up_each_cell_once() {
    let c = calc();
    let wrong = test_line(&c.per_unit, &pu_call(&c, "N", "2"), None, None, num("6"));
    let mut ls = lines(&c);
    ls.push(wrong);
    let lite = Lite::new("tcf-sum");
    let f = write(&lite, "fixture.jsonl", &ls);
    run(&lite, &f, NOW).unwrap();
    let pu = hash_in(&lite, &c.per_unit);
    let x = foreign_case(&c);
    hold_foreign(&lite, OTHER, &pu, &x, false, FOREIGN_TX);
    hold_foreign(&lite, OTHER2, &pu, &x, false, FOREIGN_TX2); // the same case, another host: another cell
    hold_foreign(&lite, OTHER, &pu, "00000000000000cc", false, FOREIGN_TX2);
    let m = build_served(lite, None, |_| {}).await;
    let d = m.get(&describe_path(&c.per_unit)).await;
    assert_eq!(d["tests"]["count"], 7, "{d}");
    assert_eq!(d["tests"]["counterexamples"], 4, "one own failing test and three foreign cells: {d}");
    // asking again changes nothing (nothing is counted twice)
    let d = m.get(&describe_path(&c.per_unit)).await;
    assert_eq!(d["tests"]["counterexamples"], 4, "{d}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_function_page_lists_an_open_counterexample_with_its_signer() {
    let c = calc();
    let lite = Lite::new("tcf-page");
    let f = write(&lite, "fixture.jsonl", &lines(&c));
    run(&lite, &f, NOW).unwrap();
    let (pu, band) = (hash_in(&lite, &c.per_unit), hash_in(&lite, &c.band));
    let x = foreign_case(&c);
    hold_foreign(&lite, OTHER, &pu, &x, false, FOREIGN_TX);
    hold_foreign(&lite, OTHER2, &band, "00000000000000aa", false, FOREIGN_TX);
    let m = build_served(lite, None, |_| {}).await;

    // JSON: the count, and the cell with its signer, inside `tests`
    let j = get(&m, &format!("/fn/{}?format=json", c.per_unit), &[]).await.json();
    assert_eq!(j["tests"]["counterexamples"], 1, "{j}");
    let t = j["tests"].to_string();
    assert!(t.contains(OTHER), "the signer {OTHER} is named in the tests: {j}");
    assert!(t.contains(&x), "the case {x} is named in the tests: {j}");
    assert!(!t.contains(OTHER2), "another function's counterexample is not listed: {j}");

    // HTML: in the tests section, as an open counterexample with its signer
    let html = get(&m, &format!("/fn/{}", c.per_unit), &[]).await;
    assert_eq!(html.status, 200, "{}", html.text());
    let text = html.text();
    let at = text.find("id=\"tests\"").expect("a tests section");
    let rest = &text[at..];
    let section = &rest[..rest[1..].find("<section").map(|i| i + 1).unwrap_or(rest.len())];
    assert!(section.contains("1 counterexamples"), "{section}");
    assert!(section.contains(OTHER), "the signer is named: {section}");
    assert!(section.contains(&x), "the case is named: {section}");
    assert!(section.to_lowercase().contains("open counterexample"), "{section}");
    assert!(!section.contains(OTHER2), "{section}");
    assert!(!text.to_lowercase().contains("<script"));

    // the other function's page lists its own
    let b = get(&m, &format!("/fn/{}?format=json", c.band), &[]).await.json();
    assert_eq!(b["tests"]["counterexamples"], 1, "{b}");
    assert!(b["tests"].to_string().contains(OTHER2), "{b}");
}
