//! http 0.4.9, CONTRACT-0.4.9 H1: `GET /fn/<name>` and `GET /describe/<function>` never run tests; they read
//! what the store holds. These tests are written first and FAIL until then.
//!
//! The way to see that nothing runs: the store's recorded results are changed, through the store, to ones a
//! run of the tests would not give (a passing test recorded as `fail`, a test that never ran, a result at a
//! date no run has), and the page must show the stored ones. A page that runs the tests shows the live ones.
//!
//! The tests use new store 0.5.2 methods (`name_function_tests`, `unnamed_function_tests`), so they do not
//! compile against 0.5.1: that is expected until the store lands.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/h049.rs"]
mod h049;
#[path = "common/publish4.rs"]
mod publish4;

use std::time::{Duration, Instant};

use calculus::*;
use common::lite::*;
use common::pub031::{function_line, run, write};
use common::readpath::*;
use common::world::call_path;
use datumesh_store::Store;
use h049::*;
use serde_json::{json, Value};

const F: &str = "mesh-test.datumesh.com/h049/tests/level";
const A: &str = "mesh-test.datumesh.com/h049/tests/a_level";
const B: &str = "mesh-test.datumesh.com/h049/tests/b_level";

fn describe_path(name: &str) -> String {
    format!("/describe/{}", call_path(name))
}

fn fn_path(name: &str) -> String {
    format!("/fn/{name}?format=json")
}

fn plain(h: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in h.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// F with three cells and three tests that pass when run (N = 10, S = 20, E = 30).
fn base_lines() -> Vec<String> {
    vec![
        function_line(F, &sig(), None),
        cell(F, "N", "10"),
        cell(F, "S", "20"),
        cell(F, "E", "30"),
        tline(F, "N", "10"),
        tline(F, "S", "20"),
        tline(F, "E", "30"),
    ]
}

fn name_all(lite: &Lite, hash: &str, name: &str, cases: &[String]) {
    let names: Vec<(String, String)> = cases.iter().map(|c| (c.clone(), name.to_string())).collect();
    lite.store.name_function_tests(hash, &names).unwrap();
}

/// The page shows the stored results: counts, the worst result, the latest run, the first cases; and no
/// `derived`. A fresh run of these tests would give 3 passes and an error for the test of the absent site.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_page_shows_the_stored_results_not_a_fresh_run() {
    let lite = Lite::new("h1-stored");
    let f = write(&lite, "f.jsonl", &base_lines());
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    let hash = hash_in(&lite, F);
    let (t0, t1, t2) = (case_id(&tline(F, "N", "10")), case_id(&tline(F, "S", "20")), case_id(&tline(F, "E", "30")));
    // as recorded: one fail (at 2030), one error (at 2029), one pass (at the publish), one that never ran
    lite.store.record_function_test(&hash, &t0, "2030-01-01T00:00:00.000000Z", "fail", "tampered").unwrap();
    lite.store.record_function_test(&hash, &t1, "2029-06-01T00:00:00.000000Z", "error", "tampered").unwrap();
    let never = tline(F, "W", "99");
    let t3 = put_raw(&lite, &hash, &never, None);
    name_all(&lite, &hash, F, std::slice::from_ref(&t3));
    let m = build_served(lite, None, |_| {}).await;

    let j = page_json(&m, &fn_path(F)).await;
    let t = &j["tests"];
    assert!(t.get("derived").is_none(), "`derived` is removed: {t}");
    assert_eq!(t["count"], 4, "{t}");
    assert_eq!(t["counterexamples"], 2, "the stored fail and error: {t}");
    assert_eq!((t["branches"].as_u64(), t["covered"].as_u64()), (Some(0), Some(0)), "{t}");
    let lr = t["last_run"].as_object().unwrap_or_else(|| panic!("last_run: {t}"));
    let mut keys: Vec<&String> = lr.keys().collect();
    keys.sort();
    assert_eq!(keys, ["at", "error", "fail", "not_run", "pass", "result"], "{t}");
    assert_eq!(
        t["last_run"],
        json!({"result": "error", "at": "2030-01-01T00:00:00.000000Z", "pass": 1, "fail": 1, "error": 1, "not_run": 1}),
        "the stored results, the worst first (error), the latest stored run time: {t}"
    );
    assert_eq!(t["cases_total"], 4, "{t}");
    let cases = t["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 4, "{t}");
    assert!(
        cases.iter().all(|c| c["call"].is_string() && c.get("expect").is_some() && c.get("result").is_some()),
        "{t}"
    );
    let result_of = |call_text: &str| {
        cases.iter().find(|c| c["call"] == call_text).unwrap_or_else(|| panic!("{call_text}: {t}"))["result"].clone()
    };
    assert_eq!(result_of(&call(F, "N")), "fail", "{t}");
    assert_eq!(result_of(&call(F, "S")), "error", "{t}");
    assert_eq!(result_of(&call(F, "E")), "pass", "{t}");
    let _ = t2;

    // the page of people says the same
    let h = plain(&publish4::get(&m, &format!("/fn/{F}"), &[]).await.text());
    assert!(h.contains("last run") && h.contains("2030-01-01"), "{h}");
    assert!(h.to_lowercase().contains("error"), "the worst result is shown: {h}");

    // and describe's numbers are the stored ones, in the shape it always had
    let d = m.get(&describe_path(F)).await;
    assert_eq!(d["tests"], json!({"count": 4, "branches": 0, "covered": 0, "counterexamples": 2}), "{d}");
}

/// The other way round: tests that fail when run, recorded as passing, show as passing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_test_recorded_as_a_pass_is_a_pass() {
    let lite = Lite::new("h1-stored-pass");
    let mut lines = vec![function_line(F, &sig(), None), cell(F, "N", "10")];
    lines.push(tline(F, "N", "11")); // fails when run
    lines.push(tline(F, "N", "12")); // fails when run
    let f = write(&lite, "f.jsonl", &lines);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    let hash = hash_in(&lite, F);
    for want in ["11", "12"] {
        lite.store
            .record_function_test(&hash, &case_id(&tline(F, "N", want)), "2031-02-03T04:05:06.000000Z", "pass", "")
            .unwrap();
    }
    let m = build_served(lite, None, |_| {}).await;
    let j = page_json(&m, &fn_path(F)).await;
    let t = &j["tests"];
    assert_eq!(t["count"], 2, "{t}");
    assert_eq!(t["counterexamples"], 0, "{t}");
    assert_eq!(
        t["last_run"],
        json!({"result": "pass", "at": "2031-02-03T04:05:06.000000Z", "pass": 2, "fail": 0, "error": 0, "not_run": 0}),
        "{t}"
    );
    let d = m.get(&describe_path(F)).await;
    assert_eq!(d["tests"]["counterexamples"], 0, "{d}");
}

/// A function none of whose tests has run: `not run`, every count in `not_run`, `at` absent or null.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tests_that_never_ran_are_not_run() {
    let lite = Lite::new("h1-notrun");
    let f = write(&lite, "f.jsonl", &[function_line(F, &sig(), None), cell(F, "N", "10")]);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    let hash = hash_in(&lite, F);
    let cases: Vec<String> =
        ["10", "11", "12"].iter().map(|w| put_raw(&lite, &hash, &tline(F, "N", w), None)).collect();
    name_all(&lite, &hash, F, &cases);
    let m = build_served(lite, None, |_| {}).await;
    let t = page_json(&m, &fn_path(F)).await["tests"].clone();
    assert_eq!(t["count"], 3, "{t}");
    assert_eq!(t["last_run"]["result"], "not run", "{t}");
    assert_eq!(
        (t["last_run"]["pass"].as_u64(), t["last_run"]["fail"].as_u64(), t["last_run"]["error"].as_u64()),
        (Some(0), Some(0), Some(0)),
        "{t}"
    );
    assert_eq!(t["last_run"]["not_run"], 3, "{t}");
    assert!(t["last_run"]["at"].is_null(), "no run, no time: {t}");
}

/// A computed function: the branch counts are the stored row's (set through the store here), not a coverage
/// computed on the request; `derived` is gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_branch_counts_are_the_stored_ones() {
    let c = Calc::new("mesh-test.datumesh.com/h049/calc");
    let mut lines = data_lines(&c);
    lines.push(per_unit_line(&c, None));
    lines.extend(per_unit_tests(&c));
    let lite = Lite::new("h1-branches");
    let f = write(&lite, "f.jsonl", &lines);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    let hash = hash_in(&lite, &c.per_unit);
    let m = build_served(lite, None, |_| {}).await;
    let t = page_json(&m, &fn_path(&c.per_unit)).await["tests"].clone();
    assert!(t.get("derived").is_none(), "{t}");
    assert_eq!(
        (t["count"].as_u64(), t["branches"].as_u64(), t["covered"].as_u64()),
        (Some(6), Some(2), Some(2)),
        "{t}"
    );
    assert_eq!(t["last_run"]["result"], "pass", "{t}");
    // a stored count that no run would give shows as stored
    m.lite.store.set_function_branches(&hash, 5, 1).unwrap();
    let t = page_json(&m, &fn_path(&c.per_unit)).await["tests"].clone();
    assert_eq!((t["branches"].as_u64(), t["covered"].as_u64()), (Some(5), Some(1)), "{t}");
    let d = m.get(&describe_path(&c.per_unit)).await;
    assert_eq!((d["tests"]["branches"].as_u64(), d["tests"]["covered"].as_u64()), (Some(5), Some(1)), "{d}");
}

/// Two stored functions with one signature share a hash: each page counts only its own tests, from the
/// moment the tests are named (http names them when it stores them).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_functions_of_one_hash_count_their_own_tests() {
    let lite = Lite::new("h1-shared");
    let mut lines = vec![function_line(A, &sig(), None), function_line(B, &sig(), None)];
    for (f, sites) in [(A, vec!["N", "S", "E"]), (B, vec!["N", "S"])] {
        for s in &sites {
            lines.push(cell(f, s, "10"));
        }
        for s in &sites {
            lines.push(tline(f, s, "10"));
        }
    }
    let file = write(&lite, "f.jsonl", &lines);
    run(&lite, &file, NOW).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hash_in(&lite, A), hash_in(&lite, B), "one signature, one hash");
    let hash = hash_in(&lite, A);
    // B's tests recorded as failing: A's page must not count them
    for s in ["N", "S"] {
        lite.store
            .record_function_test(&hash, &case_id(&tline(B, s, "10")), "2030-01-01T00:00:00.000000Z", "fail", "x")
            .unwrap();
    }
    let m = build_served(lite, None, |_| {}).await;
    let a = page_json(&m, &fn_path(A)).await["tests"].clone();
    let b = page_json(&m, &fn_path(B)).await["tests"].clone();
    assert_eq!((a["count"].as_u64(), a["counterexamples"].as_u64()), (Some(3), Some(0)), "{a}");
    assert_eq!((b["count"].as_u64(), b["counterexamples"].as_u64()), (Some(2), Some(2)), "{b}");
    assert_eq!(a["last_run"]["pass"], 3, "{a}");
    assert_eq!(a["last_run"]["fail"], 0, "{a}");
    assert_eq!((b["last_run"]["pass"].as_u64(), b["last_run"]["fail"].as_u64()), (Some(0), Some(2)), "{b}");
    assert_eq!(a["cases_total"], 3, "{a}");
    assert_eq!(b["cases"].as_array().unwrap().len(), 2, "{b}");
    for c in b["cases"].as_array().unwrap() {
        assert!(c["call"].as_str().unwrap().starts_with(B), "{c}");
    }
    for (n, want) in [(A, 3), (B, 2)] {
        let d = m.get(&describe_path(n)).await;
        assert_eq!(d["tests"]["count"], want, "{n}: {d}");
    }
    assert!(!plain(&publish4::get(&m, &format!("/fn/{A}"), &[]).await.text())
        .contains("counts include tests of other functions"));
}

/// Old rows with no name (written before 0.4.9) are named by a thread of the mesh, once; after that each
/// function counts only its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_background_thread_names_the_old_tests() {
    let lite = Lite::new("h1-naming");
    let mut lines = vec![function_line(A, &sig(), None), function_line(B, &sig(), None)];
    for f in [A, B] {
        for s in ["N", "S", "E"] {
            lines.push(cell(f, s, "10"));
        }
    }
    let file = write(&lite, "f.jsonl", &lines);
    run(&lite, &file, NOW).unwrap_or_else(|e| panic!("{e}"));
    let hash = hash_in(&lite, A);
    for s in ["N", "S"] {
        put_raw(&lite, &hash, &tline(A, s, "10"), Some((OLD_RUN, "pass")));
    }
    for s in ["N", "S", "E"] {
        put_raw(&lite, &hash, &tline(B, s, "10"), Some((OLD_RUN, "pass")));
    }
    assert_eq!(lite.store.unnamed_function_tests(100).unwrap().len(), 5);
    let m = build_served(lite, None, |_| {}).await;
    let end = Instant::now() + Duration::from_secs(40);
    loop {
        let a = page_json(&m, &fn_path(A)).await["tests"]["count"].as_u64();
        let b = page_json(&m, &fn_path(B)).await["tests"]["count"].as_u64();
        if (a, b) == (Some(2), Some(3)) {
            break;
        }
        assert!(Instant::now() < end, "the old tests were not named in 40 s: A {a:?}, B {b:?}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(m.lite.store.unnamed_function_tests(100).unwrap().is_empty(), "every row is named");
    let t = page_json(&m, &fn_path(A)).await["tests"].clone();
    assert_eq!(t["last_run"]["result"], "pass", "{t}");
    assert_eq!(t["last_run"]["at"], OLD_RUN, "naming does not run or change a result: {t}");
}

/// A function with 3,000 stored tests answers its page and describe quickly: the page reads an aggregate and
/// a page of 100, and runs nothing (running 3,000 tests takes far longer than the bound).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thousands_of_tests_do_not_slow_the_page() {
    const N: usize = 3000;
    let lite = Lite::new("h1-many");
    let mut lines = vec![function_line(F, &sig(), None)];
    lines.push(cell(F, "N", "10"));
    let file = write(&lite, "f.jsonl", &lines);
    run(&lite, &file, NOW).unwrap_or_else(|e| panic!("{e}"));
    let hash = hash_in(&lite, F);
    let mut cases = Vec::new();
    for i in 0..N {
        cases.push(put_raw(&lite, &hash, &tline(F, &format!("s{i:04}"), "1"), Some((OLD_RUN, "pass"))));
    }
    name_all(&lite, &hash, F, &cases);
    let m = build_served(lite, None, |_| {}).await;
    let bound = Duration::from_secs(5);
    for _ in 0..3 {
        let t0 = Instant::now();
        let j = page_json(&m, &fn_path(F)).await;
        let took = t0.elapsed();
        assert!(took < bound, "/fn took {took:?} with {N} tests");
        let t = &j["tests"];
        assert_eq!(t["count"], N, "{}", t["count"]);
        assert_eq!(t["cases_total"], N);
        assert_eq!(t["cases"].as_array().unwrap().len(), 100);
        assert_eq!(t["last_run"]["pass"], N);
        let t0 = Instant::now();
        let d = m.get(&describe_path(F)).await;
        let took = t0.elapsed();
        assert!(took < bound, "/describe took {took:?} with {N} tests");
        assert_eq!(d["tests"]["count"], N);
        let t0 = Instant::now();
        let h = publish4::get(&m, &format!("/fn/{F}"), &[]).await;
        assert_eq!(h.status, 200);
        assert!(t0.elapsed() < bound, "the HTML page took {:?}", t0.elapsed());
    }
}

/// The page of people: the first 100 cases and the total.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_page_lists_the_first_hundred_and_says_how_many_there_are() {
    let lite = Lite::new("h1-hundred");
    let file = write(&lite, "f.jsonl", &[function_line(F, &sig(), None), cell(F, "N", "10")]);
    run(&lite, &file, NOW).unwrap_or_else(|e| panic!("{e}"));
    let hash = hash_in(&lite, F);
    let cases: Vec<String> =
        (0..250).map(|i| put_raw(&lite, &hash, &tline(F, &format!("s{i:03}"), "1"), Some((OLD_RUN, "pass")))).collect();
    name_all(&lite, &hash, F, &cases);
    let m = build_served(lite, None, |_| {}).await;
    let j = page_json(&m, &fn_path(F)).await;
    assert_eq!(
        (j["tests"]["cases_total"].as_u64(), j["tests"]["cases"].as_array().map(Vec::len)),
        (Some(250), Some(100))
    );
    let h = plain(&publish4::get(&m, &format!("/fn/{F}"), &[]).await.text());
    assert!(h.contains("Showing the first 100 of 250 tests"), "{h}");
    assert!(h.contains("last run") && h.contains("2026-01-01"), "{h}");
    let _: Value = j;
}
