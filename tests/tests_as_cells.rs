//! Build 2 Part 3, E5: tests as cells, `function_tests` in the store, and `describe` `tests`.
//!
//! * The tests of a function are held in the store, whoever published them (`publish-stored` lines,
//!   a bundle's `tests`), table `function_tests` (`fn_hash`, `case`, `test_json`, `signed_by`,
//!   `last_run_at`, `last_result`, `last_detail`, `branches_total`, `branches_covered`). The case
//!   id is the first 16 hex digits of the SHA-256 of the test's JCS.
//! * `GET /describe/<function name>` answers the function answer of spec 16.8 (`kind` `function`,
//!   `fn`, `fn_hash`, `args`, `returns`), with `tests`: `count` (tests held for that hash),
//!   `branches` (arms in its body), `covered` (arms the held tests take), `counterexamples` (failing
//!   reruns, below). Core 0.3.9's `check_describe_with` accepts it. A stored function has no `tests`
//!   member (or all zeros).
//! * On a mesh with `DATUMESH_TEST_CELLS=1` (mesh-test), each test is also a signed cell (trust `primary`)
//!   `<host>/test_case(<hash arg>: string, case: string) -> boolean`, signed by the mesh: `true`
//!   when the function under test, run as the test says, gives what the test expects, `false`
//!   when it does not. Production (no variable) has no such cells (`not_covered`, the name is
//!   unbound), and still counts the tests in `describe`.

#[path = "common/calculus.rs"]
mod calculus;
mod common;

use std::collections::BTreeMap;

use calculus::*;
use common::lite::*;
use common::readpath::*;
use common::world::call_path;
use serde_json::json;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describe_counts_the_tests_of_a_function() {
    let c = calc();
    let m = serve_lite("tac-describe", &lines(&c), None, |_| {}).await;
    let (r, hash) = (m.get_any(&describe_path(&c.per_unit)).await, fn_hash(&m, &c.per_unit));
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let d = &r.json;
    assert_eq!(
        (d["kind"].as_str(), d["fn"].as_str(), d["fn_hash"].as_str()),
        (Some("function"), Some(c.per_unit.as_str()), Some(hash.as_str())),
        "{d}"
    );
    assert_eq!(d["tests"], serde_json::json!({"count": 6, "branches": 2, "covered": 2, "counterexamples": 0}), "{d}");
    // core 0.3.9 checks the shape, and the args and returns against the descriptor it holds
    let held =
        datumesh_core::FunctionDescriptor::from_json(&m.get_any(&format!("/descriptor/{hash}")).await.body).unwrap();
    let got = datumesh_core::check_describe_with(&r.body, &BTreeMap::new(), &[held]).unwrap_or_else(|e| panic!("{e}"));
    let t = got.tests.expect("tests");
    assert_eq!((t.count, t.branches, t.covered, t.counterexamples), (6, 2, 2, 0));
    assert_eq!(got.function.as_deref(), Some(c.per_unit.as_str()));

    // three tests over two ifs: 4 arms, all taken
    let d = m.get(&describe_path(&c.band)).await;
    assert_eq!(d["tests"], serde_json::json!({"count": 3, "branches": 4, "covered": 4, "counterexamples": 0}), "{d}");

    // a stored function is exempt: no tests member, or an all-zero one
    let d = m.get(&describe_path(&c.reading)).await;
    assert_eq!(d["kind"], "function");
    match d.get("tests") {
        None => {}
        Some(t) => assert_eq!(*t, serde_json::json!({"count": 0, "branches": 0, "covered": 0, "counterexamples": 0})),
    }
}

/// Tests that cover fewer arms count fewer: `covered` is below `branches`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn covered_counts_the_arms_the_tests_take() {
    let c = calc();
    let mut v = data_lines(&c);
    v.push(band_line(&c));
    v.extend(band_tests(&c)[..2].to_vec()); // x = -1 and 5: three of the four arms
    let m = serve_lite("tac-covered", &v, None, |_| {}).await;
    let d = m.get(&describe_path(&c.band)).await;
    assert_eq!(d["tests"], serde_json::json!({"count": 2, "branches": 4, "covered": 3, "counterexamples": 0}), "{d}");
}

fn test_cell_path(m: &Served, c: &Calc, line: &str) -> String {
    format!("/cell/{}", call_path(&test_case_call(&fn_hash(m, &c.per_unit), &case_id(line))))
}

/// On mesh-test: each test is a signed boolean cell, trust `primary` (spec 5.5: host-signed, no inputs; QUESTIONS D16), `true` for a test that passes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn on_mesh_test_each_test_is_a_cell() {
    let c = calc();
    let m = serve_lite("tac-cells", &lines(&c), Some(test_cells_config()), |_| {}).await;
    for t in per_unit_tests(&c) {
        let v = m.get(&test_cell_path(&m, &c, &t)).await;
        assert_eq!(v["outcome"], "ok", "{t}: {v}");
        assert_eq!((v["receipt"]["type"].as_str(), &v["receipt"]["value"]), (Some("boolean"), &json!(true)), "{v}");
        assert_eq!(v["trust"], "primary", "{v}");
        assert!(v["receipt"]["computed_by"].as_str().unwrap().starts_with(&format!("{HOST}#")), "{v}");
        assert_eq!(v["receipt"]["cell"]["fn"], "test_case");
        assert_eq!(v["receipt"]["cell"]["ns"], HOST);
    }
    // the cell of a case that no test of the function has is not a value
    let v = m
        .get_any(&format!("/cell/{}", call_path(&test_case_call(&fn_hash(&m, &c.per_unit), "0000000000000000"))))
        .await;
    assert_ne!(v.json["outcome"], "ok", "{}", v.json);
    assert!((400..600).contains(&v.status), "{}", v.status);
}

/// A failing test (the local publish records it) is a `false` cell.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_test_is_a_false_cell() {
    let c = calc();
    let wrong = test_line(&c.per_unit, &pu_call(&c, "N", "2"), None, None, num("6"));
    let mut v = data_lines(&c);
    v.push(per_unit_line(&c, None));
    v.push(wrong.clone());
    let m = serve_lite("tac-false", &v, Some(test_cells_config()), |_| {}).await;
    let cell = m.get(&test_cell_path(&m, &c, &wrong)).await;
    assert_eq!((cell["outcome"].as_str(), &cell["receipt"]["value"]), (Some("ok"), &json!(false)), "{cell}");
}

/// Production names no test cells: not covered; the tests are still counted by describe.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_has_no_test_cells_but_counts_the_tests() {
    let c = calc();
    let m = serve_lite("tac-prod", &lines(&c), None, |_| {}).await;
    let t = &per_unit_tests(&c)[0];
    let r = m.get_any(&test_cell_path(&m, &c, t)).await;
    assert_eq!((r.status, r.json["outcome"].as_str()), (404, Some("not_covered")), "{}", r.json);
    let d = m.get(&describe_path(&c.per_unit)).await;
    assert_eq!(d["tests"]["count"], 6);
}
