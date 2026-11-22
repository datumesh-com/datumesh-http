//! The classes round (CONTRACT-classes-round.md), the test run: K1 and K3. Failing tests: the behaviour is not written yet.
//!
//! * **K1, test inputs that override a slice.** An `inputs` entry `"<slice call, with *>": {"add": {"<cell call>": <typed
//!   scalar>, ...}, "remove": ["<cell call>", ...]}` (the add-and-remove form of a whatif slice override, SPEC 5.9): a slice
//!   read during the run gives the store's members, minus `remove`, plus `add`; the `empty:` event follows from that. Both
//!   members are optional, at least one is present, every cell named is a member of that slice by its arguments, and the case
//!   hash (SPEC 7.3) stays the hash of the test line, `inputs` included. There is no form that clears a slice.
//! * **K3, boolean expectations.** `expect` (and an `inputs` value) of type `boolean` takes JSON `true` or `false`, as a cell
//!   line does (SPEC 3); the string `"true"` is refused there. The other types keep their lexical strings. The fixture is the
//!   three `loaded` tests `sec.py` writes (`company_tests`) over a `loaded` function and its cells.
//!
//! The mesh is the 1.8b one: SQLite store, fixed clock, `publish_stored_checked` (the public rule on).

#[path = "common/calculus.rs"]
mod calculus;
mod common;

use calculus::*;
use common::lite::*;
use common::pub031::*;
use datumesh_http::publish::PublishError;
use datumesh_store::Store;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------------------------------------------------
// K1

/// The calculus data, and a site `S` with one log day only.
fn data(c: &Calc) -> Vec<String> {
    let mut v = data_lines(c);
    v.push(log_cell(&c.log, "S", "2026-03-01", "7"));
    v
}

/// The slice `log(site: <site>, day: *)`.
fn slice(c: &Calc, site: &str) -> String {
    format!("{}(site: \"{site}\", day: *)", c.log)
}

/// The cell `log(site: <site>, day: <day>)`.
fn cell(c: &Calc, site: &str, day: &str) -> String {
    format!("{}(site: \"{site}\", day: {day})", c.log)
}

fn n(v: &str) -> Value {
    num(v)
}

/// The three tests of `avg_log` that give the public rule its derived cases, the empty slice among them reached **by the
/// slice override** (S's only member removed), not by a site that has no log: positive, negative scale, empty.
fn avg_tests(c: &Calc) -> Vec<String> {
    let f = &c.avg_log;
    vec![
        test_line(f, &avg_call(c, "N", "2"), None, None, n("6")),
        test_line(f, &avg_call(c, "N", "-1"), None, None, n("-3")),
        test_line(
            f,
            &avg_call(c, "S", "1"),
            Some(json!({ slice(c, "S"): {"remove": [cell(c, "S", "2026-03-01")]} })),
            None,
            fault("E_FORMULA_EMPTY"),
        ),
    ]
}

fn file(c: &Calc, tests: Vec<String>) -> Vec<String> {
    let mut v = data(c);
    v.push(avg_log_line(c));
    v.extend(tests);
    v
}

/// A refusal with the public rule on: at `line`, and the store is as it was.
fn refused_at(label: &str, lines: &[String], line: usize) -> PublishError {
    let lite = Lite::new(label);
    let f = write(&lite, "f.jsonl", lines);
    let before = snapshot(&lite);
    let e = run_checked(&lite, &f, NOW).expect_err("the file must be refused");
    assert_eq!(e.line, line, "refused at the wrong line: {e}");
    assert_eq!(snapshot(&lite), before, "a refused file wrote something: {e}");
    e
}

/// The line of the n-th test (0-based) in `file(c, ..)`: data (7 lines), the formula, then the tests.
fn line_of_test(i: usize) -> usize {
    7 + 1 + 1 + i
}

/// An added member is seen: `mean(2, 4, 9) = 5` (the store's mean is 3). The same test expecting the store's mean fails, and
/// the failure says what came out.
#[test]
fn k1_an_added_member_is_seen() {
    let c = calc();
    let add = test_line(
        &c.avg_log,
        &avg_call(&c, "N", "1"),
        Some(json!({ slice(&c, "N"): {"add": {cell(&c, "N", "2026-03-03"): n("9")}} })),
        None,
        n("5"),
    );
    let mut t = avg_tests(&c);
    t.push(add);
    let (lite, r) = checked("cr-k1-add", &file(&c, t));
    let p = r.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(p.functions, 3, "{p:?}");
    // the override is the test's, not the data's: the store holds no third cell for N
    let h = lite.store.resolve_binding(&c.log, END).unwrap().unwrap().fn_hash;
    let id = datumesh_core::Cell {
        ns: c.log.rsplit_once('/').unwrap().0.to_string(),
        fn_name: "log".to_string(),
        fn_hash: h,
        args: [("site", "N", datumesh_core::Type::String), ("day", "2026-03-03", datumesh_core::Type::Date)]
            .iter()
            .map(|(k, v, t)| {
                (k.to_string(), datumesh_core::Arg::Value(datumesh_core::Scalar::from_lexical(*t, v).unwrap()))
            })
            .collect(),
    }
    .id();
    assert!(lite.store.version_asof(&id, END).unwrap().is_none(), "the added member was written to the store");
}

/// A removed member is not seen: removing the day worth 2 leaves `mean(4) = 4`; removing the day worth 4 leaves 2.
#[test]
fn k1_a_removed_member_is_not_seen() {
    let c = calc();
    let rm = |day: &str, want: &str| {
        test_line(
            &c.avg_log,
            &avg_call(&c, "N", "1"),
            Some(json!({ slice(&c, "N"): {"remove": [cell(&c, "N", day)]} })),
            None,
            n(want),
        )
    };
    let mut t = avg_tests(&c);
    t.push(rm("2026-03-01", "4"));
    t.push(rm("2026-03-02", "2"));
    let (_, r) = checked("cr-k1-remove", &file(&c, t));
    r.unwrap_or_else(|e| panic!("{e}"));
    // a wrong expectation is a failing test, with the number that came out
    let mut t = avg_tests(&c);
    t.push(rm("2026-03-01", "3"));
    let e = refused_at("cr-k1-remove-wrong", &file(&c, t), line_of_test(3));
    assert!(e.detail.contains("E_TESTS_FAILING") && e.detail.contains('3') && e.detail.contains('4'), "{e}");
}

/// Add and remove together, in the same entry, in key order: `{2, 4}` minus the day worth 2, plus a day worth 9: `mean(4, 9)`.
#[test]
fn k1_add_and_remove_together() {
    let c = calc();
    let both = test_line(
        &c.avg_log,
        &avg_call(&c, "N", "2"),
        Some(json!({ slice(&c, "N"): {
            "add": {cell(&c, "N", "2026-03-03"): n("9")},
            "remove": [cell(&c, "N", "2026-03-01")]} })),
        None,
        n("13"),
    );
    let mut t = avg_tests(&c);
    t.push(both);
    let (_, r) = checked("cr-k1-both", &file(&c, t));
    r.unwrap_or_else(|e| panic!("{e}"));
}

/// Removing a stored slice's only member gives the empty slice: the `empty:` event, so the derived case `empty(log)` is
/// covered by that test alone; without it the file is `E_TESTS_MISSING` for that case.
#[test]
fn k1_removing_the_only_member_gives_empty() {
    let c = calc();
    let (_, r) = checked("cr-k1-empty", &file(&c, avg_tests(&c)));
    r.unwrap_or_else(|e| panic!("{e}"));
    // the control: the same tests without the removal one miss the empty case (no site of the tests has no log)
    let two: Vec<String> = avg_tests(&c).into_iter().take(2).collect();
    refused("cr-k1-empty-control", &file(&c, two), 8, "E_TESTS_MISSING", &[&format!("empty({})", c.log)]);
    // and a value expected where the slice is empty is a failing test, which says E_FORMULA_EMPTY came out
    let mut t = avg_tests(&c);
    t[2] = test_line(
        &c.avg_log,
        &avg_call(&c, "S", "1"),
        Some(json!({ slice(&c, "S"): {"remove": [cell(&c, "S", "2026-03-01")]} })),
        None,
        n("7"),
    );
    let e = refused_at("cr-k1-empty-value", &file(&c, t), line_of_test(2));
    assert!(e.detail.contains("E_TESTS_FAILING") && e.detail.contains("E_FORMULA_EMPTY"), "{e}");
}

/// A cell that is a member of the slice by its arguments but is not stored is accepted (the override is by arguments):
/// removing it changes nothing.
#[test]
fn k1_a_member_by_its_arguments_need_not_be_stored() {
    let c = calc();
    let noop = test_line(
        &c.avg_log,
        &avg_call(&c, "N", "2"),
        Some(json!({ slice(&c, "N"): {"remove": [cell(&c, "N", "2026-03-09")]} })),
        None,
        n("6"),
    );
    let mut t = avg_tests(&c);
    t.push(noop);
    let (_, r) = checked("cr-k1-noop", &file(&c, t));
    r.unwrap_or_else(|e| panic!("{e}"));
}

/// A cell that is not in the slice is refused, in `add` and in `remove`: the slice is N's, the cell is S's (another `site`),
/// or the cell is of another function. The file is refused at the test's line and nothing is written.
#[test]
fn k1_a_cell_not_in_the_slice_is_refused() {
    let c = calc();
    let with = |inputs: Value| {
        let mut t = avg_tests(&c);
        t.push(test_line(&c.avg_log, &avg_call(&c, "N", "1"), Some(inputs), None, n("3")));
        file(&c, t)
    };
    for (what, inputs) in [
        ("remove of another site's cell", json!({ slice(&c, "N"): {"remove": [cell(&c, "S", "2026-03-01")]} })),
        ("add of another site's cell", json!({ slice(&c, "N"): {"add": {cell(&c, "S", "2026-03-05"): n("1")}} })),
        (
            "remove of another function's cell",
            json!({ slice(&c, "N"): {"remove": [format!("{}(site: \"N\")", c.reading)]} }),
        ),
    ] {
        let e = refused_at(&format!("cr-k1-notin-{}", what.len()), &with(inputs), line_of_test(3));
        eprintln!("{what}: {e}");
    }
    // the control: the same test with a member of the slice is accepted
    let ok = with(json!({ slice(&c, "N"): {"remove": [cell(&c, "N", "2026-03-09")]} }));
    let (_, r) = checked("cr-k1-notin-control", &ok);
    r.unwrap_or_else(|e| panic!("control: {e}"));
}

/// An entry with neither member is refused (`{}`), and so is a member that is not `add` or `remove`.
#[test]
fn k1_an_entry_with_neither_member_is_refused() {
    let c = calc();
    for (i, entry) in [json!({}), json!({"keep": [cell(&c, "N", "2026-03-01")]})].into_iter().enumerate() {
        let mut t = avg_tests(&c);
        t.push(test_line(&c.avg_log, &avg_call(&c, "N", "1"), Some(json!({ slice(&c, "N"): entry })), None, n("3")));
        refused_at(&format!("cr-k1-neither-{i}"), &file(&c, t), line_of_test(3));
    }
}

/// The case hash is the hash of the test line, so the overrides, which are in `inputs`, are part of it: the same call and
/// expectation with and without them are two cases, and the store holds a row under each case id.
#[test]
fn k1_the_case_hash_changes_with_the_overrides() {
    let c = calc();
    let plain = test_line(&c.avg_log, &avg_call(&c, "N", "1"), None, None, n("3"));
    let over = test_line(
        &c.avg_log,
        &avg_call(&c, "N", "1"),
        Some(json!({ slice(&c, "N"): {"add": {cell(&c, "N", "2026-03-03"): n("3")}} })),
        None,
        n("3"),
    );
    let other = test_line(
        &c.avg_log,
        &avg_call(&c, "N", "1"),
        Some(json!({ slice(&c, "N"): {"add": {cell(&c, "N", "2026-03-03"): n("4")}} })),
        None,
        n("3"),
    );
    assert_eq!(case_id(&plain).len(), 16);
    assert_ne!(case_id(&plain), case_id(&over), "the case hash holds inputs");
    assert_ne!(case_id(&over), case_id(&other), "and the override's value");
    let mut t = avg_tests(&c);
    t.extend([plain.clone(), over.clone()]);
    let (lite, r) = checked("cr-k1-hash", &file(&c, t));
    r.unwrap_or_else(|e| panic!("{e}"));
    let hash = lite.store.resolve_binding(&c.avg_log, END).unwrap().unwrap().fn_hash;
    let cases: Vec<String> = lite.store.function_tests(&hash).unwrap().into_iter().map(|r| r.case).collect();
    for l in [&plain, &over] {
        assert!(cases.contains(&case_id(l)), "a row for case {}: {cases:?}", case_id(l));
    }
    assert_eq!(cases.len(), 5, "{cases:?}");
}

// ---------------------------------------------------------------------------------------------------------------------
// K3

const LOADED: &str = "mesh-test.datumesh.com/sec/company/loaded";
const FLAG: &str = "mesh-test.datumesh.com/sec/company/flag";
const CIKS: [&str; 3] = ["0000018230", "0000019617", "0000000777"];

fn loaded_sig() -> String {
    r#"{"args":{"cik":{"type":"string","valid":false}},"order":["cik"],"returns":"boolean"}"#.to_string()
}

fn loaded_cell(cik: &str) -> String {
    json!({"cell": {"fn": LOADED, "args": {"cik": {"type": "string", "value": cik}}},
        "licence": "CC0-1.0", "summary": "Synthetic test data.", "value": {"type": "boolean", "value": true}})
    .to_string()
}

/// A boolean typed scalar as a cell line writes it: JSON `true` or `false`.
fn boolean(v: bool) -> Value {
    json!({"type": "boolean", "value": v})
}

/// The shape `sec.py`'s `company_tests` writes for `loaded`: `{"test", "call", "expect": {"type": "boolean", "value": true}}`.
fn loaded_test(cik: &str, expect: Value) -> String {
    test_line(LOADED, &format!("{LOADED}(cik: \"{cik}\")"), None, None, expect)
}

fn loaded_file(tests: Vec<String>) -> Vec<String> {
    let mut v = vec![function_line(LOADED, &loaded_sig(), None)];
    v.extend(CIKS.iter().map(|c| loaded_cell(c)));
    v.extend(tests);
    v
}

/// The three `loaded` tests of the sec/company file publish, and every stored row is a pass.
#[test]
fn k3_the_three_loaded_tests_publish() {
    let tests: Vec<String> = CIKS.iter().map(|c| loaded_test(c, boolean(true))).collect();
    let (lite, r) = checked("cr-k3-loaded", &loaded_file(tests));
    let p = r.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((p.functions, p.cells), (1, 3), "{p:?}");
    let hash = lite.store.resolve_binding(LOADED, END).unwrap().unwrap().fn_hash;
    let rows = lite.store.function_tests(&hash).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r.last_result.as_deref() == Some("pass")), "{rows:?}");
}

/// A boolean expectation that does not match is a failing test. `loaded` is a stored function, and the failing test of a
/// stored function is an open counterexample, never a refusal (tests/stored_conformance.rs; coordinator decision on K3):
/// the file is published, the mismatching case is `fail`, the others `pass`, and `describe` counts one counterexample.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k3_a_boolean_that_does_not_match_is_a_failing_test() {
    let wrong = loaded_test(CIKS[0], boolean(false));
    let mut tests = vec![wrong.clone()];
    tests.extend(CIKS[1..].iter().map(|c| loaded_test(c, boolean(true))));
    let (lite, r) = checked("cr-k3-wrong", &loaded_file(tests));
    let p = r.unwrap_or_else(|e| panic!("a stored function's failing test never refuses the publish: {e}"));
    assert_eq!((p.functions, p.cells), (1, 3), "{p:?}");
    let hash = lite.store.resolve_binding(LOADED, END).unwrap().unwrap().fn_hash;
    let rows = lite.store.function_tests(&hash).unwrap();
    assert_eq!(rows.len(), 3, "{rows:?}");
    for r in &rows {
        let want = if r.case == case_id(&wrong) { "fail" } else { "pass" };
        assert_eq!(r.last_result.as_deref(), Some(want), "{rows:?}");
    }
    // the counterexample is open in `describe`
    let m = common::readpath::build_served(lite, None, |_| {}).await;
    let d = m.get(&format!("/describe/{}", common::world::call_path(LOADED))).await;
    assert_eq!(d["tests"]["count"], 3, "{d}");
    assert_eq!(d["tests"]["counterexamples"], 1, "{d}");
}

/// The string `"true"` (or `"false"`) is refused as the value of a boolean typed scalar, in `expect` and in `inputs`, as a
/// cell refuses it. The other types keep their lexical strings: a number's `"1.5"` is accepted.
#[test]
fn k3_the_string_true_is_refused() {
    for (i, s) in ["true", "false", "True"].into_iter().enumerate() {
        let bad = loaded_test(CIKS[0], json!({"type": "boolean", "value": s}));
        let mut tests = vec![bad];
        tests.extend(CIKS[1..].iter().map(|c| loaded_test(c, boolean(true))));
        refused_at(&format!("cr-k3-string-expect-{i}"), &loaded_file(tests), 5);
    }
    // the same cell line with the string value is refused as well (the rule is the cell's, SPEC 3)
    let mut ls = loaded_file(vec![]);
    ls[1] = ls[1].replace("\"value\":true", "\"value\":\"true\"");
    assert!(ls[1].contains("\"true\""), "{}", ls[1]);
    refused_at("cr-k3-string-cell", &ls, 2);
}

/// The other types keep their lexical strings (a number's value is a JSON string): unchanged.
#[test]
fn k3_the_other_types_keep_their_lexical_strings() {
    let c = calc();
    let (_, r) = checked("cr-k3-number", &with_per_unit(&c));
    r.unwrap_or_else(|e| panic!("{e}"));
    // a JSON number where the lexical string is expected is still refused
    let mut t = per_unit_tests(&c);
    t[0] = t[0].replace("\"value\":\"5\"", "\"value\":5");
    assert!(t[0].contains("\"value\":5"), "{}", t[0]);
    let mut v = data_lines(&c);
    v.push(per_unit_line(&c, None));
    v.extend(t);
    refused_at("cr-k3-number-json", &v, 8);
}

fn with_per_unit(c: &Calc) -> Vec<String> {
    let mut v = data_lines(c);
    v.push(per_unit_line(c, None));
    v.extend(per_unit_tests(c));
    v
}

/// A boolean `inputs` value is JSON `true` or `false` too: `flag(cik)` is 1 when `loaded(cik)` is true and 0 when not; the
/// test supplies `loaded` as `false` for a company that has it stored as `true`.
#[test]
fn k3_boolean_inputs_are_json_booleans() {
    let sig =
        r#"{"args":{"cik":{"type":"string","valid":false}},"order":["cik"],"returns":{"type":"number","unit":"1"}}"#;
    let flag = formula_line(FLAG, &format!("if {LOADED}(cik: cik) then 1 else 0"), sig, None);
    let call = |c: &str| format!("{FLAG}(cik: \"{c}\")");
    let input = |cik: &str, v: Value| json!({ format!("{LOADED}(cik: \"{cik}\")"): v });
    let tests = vec![
        test_line(FLAG, &call(CIKS[0]), None, None, num("1")),
        test_line(FLAG, &call(CIKS[0]), Some(input(CIKS[0], boolean(false))), None, num("0")),
        test_line(FLAG, &call("0000000000"), Some(input("0000000000", boolean(true))), None, num("1")),
        test_line(FLAG, &call("0000000000"), None, None, fault("E_FORMULA_ABSENT")),
    ];
    let mut v = loaded_file(vec![]);
    v.push(flag.clone());
    v.extend(tests.clone());
    let (_, r) = checked("cr-k3-inputs", &v);
    let p = r.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((p.functions, p.cells), (2, 3), "{p:?}");
    // the string "false" as an input is refused
    let mut bad = tests.clone();
    bad[1] = test_line(
        FLAG,
        &call(CIKS[0]),
        Some(input(CIKS[0], json!({"type": "boolean", "value": "false"}))),
        None,
        num("0"),
    );
    let mut v = loaded_file(vec![]);
    v.push(flag);
    v.extend(bad);
    refused_at("cr-k3-inputs-string", &v, 7);
}
