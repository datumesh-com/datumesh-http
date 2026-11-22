//! The 0.7 round (CONTRACT-round-0.7.md), R3: **a test or a whatif may replace a slice's whole member set** (SPEC 5.9, 7.3).
//! Failing tests: the behaviour is not written yet.
//!
//! * A test's `inputs` entry for a slice may be `{"members": {"<cell call>": <typed scalar>, ...}}` besides `add` and `remove`
//!   (CONTRACT-classes-round K1): it replaces the slice's whole member set, which may be empty (the `empty:` event follows).
//!   `members` excludes `add` and `remove` (`E_SLICE_MEMBER`), and every cell must be a member of the slice by its arguments.
//!   The case hash takes it through `inputs` (7.3): two tests with different members are two cases.
//! * A whatif override `{"slice": <call>, "members": [{"cell", "value"}, ...]}` does the same; the scenario echoes
//!   `{"slice": <slice 4.4>, "members": [{"cell": <cell 4.4>, "value": <typed scalar>}, ...]}`, the members sorted by cell id and
//!   present even when empty, with no `add` or `remove`.
//!
//! The `latest` tests of the SEC views file are in `sec_views.rs` (`latest_publishes_under_the_public_rule`).

#[path = "common/calculus.rs"]
mod calculus;
mod common;

use calculus::*;
use common::lite::*;
use common::pub031::{snapshot, write};
use common::readpath::*;
use datumesh_http::publish::PublishError;
use datumesh_store::Store;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------------------------------------------------
// tests (inputs)

/// The calculus data, and a site `S` with one log day only.
fn data(c: &Calc) -> Vec<String> {
    let mut v = data_lines(c);
    v.push(log_cell(&c.log, "S", "2026-03-01", "7"));
    v
}

fn slice(c: &Calc, site: &str) -> String {
    format!("{}(site: \"{site}\", day: *)", c.log)
}

fn cell(c: &Calc, site: &str, day: &str) -> String {
    format!("{}(site: \"{site}\", day: {day})", c.log)
}

fn n(v: &str) -> Value {
    num(v)
}

/// The three tests of `avg_log` that give the public rule its derived cases, the empty slice among them reached by
/// `{"members": {}}` (S's only member replaced by nothing): positive, negative scale, empty.
fn base_tests(c: &Calc) -> Vec<String> {
    let f = &c.avg_log;
    vec![
        test_line(f, &avg_call(c, "N", "2"), None, None, n("6")),
        test_line(f, &avg_call(c, "N", "-1"), None, None, n("-3")),
        test_line(
            f,
            &avg_call(c, "S", "1"),
            Some(json!({ slice(c, "S"): {"members": {}} })),
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

/// The (1-based) line of the n-th test (0-based) in `file(c, ..)`: data (7 lines), the formula, then the tests.
fn line_of_test(i: usize) -> usize {
    7 + 1 + 1 + i
}

fn refused_at(label: &str, lines: &[String], line: usize) -> PublishError {
    let lite = Lite::new(label);
    let f = write(&lite, "f.jsonl", lines);
    let before = snapshot(&lite);
    let e = run_checked(&lite, &f, NOW).expect_err("the file must be refused");
    assert_eq!(e.line, line, "refused at the wrong line: {e}");
    assert_eq!(snapshot(&lite), before, "a refused file wrote something: {e}");
    e
}

/// `{"members": {}}` empties a stored slice: S has one stored member, the replacement has none, so `mean` is empty
/// (`E_FORMULA_EMPTY`) and the derived case `empty(log)` is covered by that test alone. Without it, the file is
/// `E_TESTS_MISSING` for that case (no site of the other tests has no log); and a value expected where the slice is empty fails
/// the test, saying `E_FORMULA_EMPTY` came out.
#[test]
fn r3_an_empty_members_replacement_gives_empty() {
    let c = calc();
    let (_, r) = checked("r07-m-empty", &file(&c, base_tests(&c)));
    r.unwrap_or_else(|e| panic!("{e}"));
    let two: Vec<String> = base_tests(&c).into_iter().take(2).collect();
    refused("r07-m-empty-control", &file(&c, two), 8, "E_TESTS_MISSING", &[&format!("empty({})", c.log)]);
    let mut t = base_tests(&c);
    t[2] =
        test_line(&c.avg_log, &avg_call(&c, "S", "1"), Some(json!({ slice(&c, "S"): {"members": {}} })), None, n("7"));
    let e = refused_at("r07-m-empty-value", &file(&c, t), line_of_test(2));
    assert!(e.detail.contains("E_TESTS_FAILING") && e.detail.contains("E_FORMULA_EMPTY"), "{e}");
}

/// The replacement is the whole member set: N's stored days (2 and 4) are not seen, only the given members: `mean(9, 5) = 7`;
/// one member, `mean(9) = 9`; a given member that is also stored takes the given value (`100`, not 2), and the other stored one
/// is gone. Members are cells of the slice by their arguments and need not be stored.
#[test]
fn r3_members_replace_the_whole_member_set() {
    let c = calc();
    let with = |members: Value, want: &str| {
        test_line(
            &c.avg_log,
            &avg_call(&c, "N", "1"),
            Some(json!({ slice(&c, "N"): {"members": members} })),
            None,
            n(want),
        )
    };
    let mut t = base_tests(&c);
    t.push(with(json!({cell(&c, "N", "2026-03-03"): n("9"), cell(&c, "N", "2026-03-04"): n("5")}), "7"));
    t.push(with(json!({cell(&c, "N", "2026-03-03"): n("9")}), "9"));
    t.push(with(json!({cell(&c, "N", "2026-03-01"): n("100")}), "100"));
    let (_, r) = checked("r07-m-two", &file(&c, t));
    r.unwrap_or_else(|e| panic!("{e}"));
    // the store's own mean (3) is not what a replacement sees
    let mut t = base_tests(&c);
    t.push(with(json!({cell(&c, "N", "2026-03-03"): n("9"), cell(&c, "N", "2026-03-04"): n("5")}), "3"));
    let e = refused_at("r07-m-two-wrong", &file(&c, t), line_of_test(3));
    assert!(e.detail.contains("E_TESTS_FAILING") && e.detail.contains('3') && e.detail.contains('7'), "{e}");
}

/// `members` with `add`, or with `remove`, in one entry is refused (`E_SLICE_MEMBER`), at the test's line, and nothing is
/// written. The control: the same entry with only `members` is accepted.
#[test]
fn r3_members_with_add_or_remove_is_refused() {
    let c = calc();
    let test = |entry: Value| {
        let mut t = base_tests(&c);
        t.push(test_line(&c.avg_log, &avg_call(&c, "N", "1"), Some(json!({ slice(&c, "N"): entry })), None, n("9")));
        file(&c, t)
    };
    let both = [
        json!({"members": {cell(&c, "N", "2026-03-03"): n("9")}, "add": {cell(&c, "N", "2026-03-05"): n("1")}}),
        json!({"members": {cell(&c, "N", "2026-03-03"): n("9")}, "remove": [cell(&c, "N", "2026-03-01")]}),
        json!({"members": {}, "add": {}}),
        json!({"members": {}, "remove": []}),
    ];
    for (i, entry) in both.into_iter().enumerate() {
        let e = refused_at(&format!("r07-m-excl-{i}"), &test(entry), line_of_test(3));
        assert!(e.detail.contains("E_SLICE_MEMBER"), "{i}: {e}");
    }
    let (_, r) = checked("r07-m-excl-ok", &test(json!({"members": {cell(&c, "N", "2026-03-03"): n("9")}})));
    r.unwrap_or_else(|e| panic!("control: {e}"));
}

/// Every cell of `members` must be a member of the slice by its arguments: another site's cell, another function's cell, and a
/// `members` that is not an object, are refused at the test's line.
#[test]
fn r3_a_cell_not_in_the_slice_is_refused() {
    let c = calc();
    let test = |entry: Value| {
        let mut t = base_tests(&c);
        t.push(test_line(&c.avg_log, &avg_call(&c, "N", "1"), Some(json!({ slice(&c, "N"): entry })), None, n("9")));
        file(&c, t)
    };
    for (i, (what, entry)) in [
        ("another site's cell", json!({"members": {cell(&c, "S", "2026-03-05"): n("1")}})),
        ("another function's cell", json!({"members": {format!("{}(site: \"N\")", c.reading): n("1")}})),
        ("an array in place of the object", json!({"members": [cell(&c, "N", "2026-03-05")]})),
    ]
    .into_iter()
    .enumerate()
    {
        let e = refused_at(&format!("r07-m-notin-{i}"), &test(entry), line_of_test(3));
        if i < 2 {
            assert!(e.detail.contains("E_SLICE_MEMBER"), "{what}: {e}");
        }
    }
}

/// The case hash is the hash of the test line: the same call and expectation with no override, with `add`, with `remove` and with
/// `members` are four cases, and different members are two cases; the store holds a row under each case id.
#[test]
fn r3_the_case_hash_differs_from_add_and_remove() {
    let c = calc();
    let mk = |inputs: Option<Value>, want: &str| test_line(&c.avg_log, &avg_call(&c, "N", "1"), inputs, None, n(want));
    // N's stored days are worth 2 and 4; every expectation below is what its override computes
    let plain = mk(None, "3");
    let add = mk(Some(json!({ slice(&c, "N"): {"add": {cell(&c, "N", "2026-03-03"): n("6")}} })), "4");
    let remove = mk(Some(json!({ slice(&c, "N"): {"remove": [cell(&c, "N", "2026-03-01")]} })), "4");
    let members = mk(Some(json!({ slice(&c, "N"): {"members": {cell(&c, "N", "2026-03-02"): n("4")}} })), "4");
    let members2 = mk(Some(json!({ slice(&c, "N"): {"members": {cell(&c, "N", "2026-03-02"): n("5")}} })), "5");
    let all = [plain, add, remove, members, members2];
    let ids: Vec<String> = all.iter().map(|l| case_id(l)).collect();
    for i in 0..ids.len() {
        for j in i + 1..ids.len() {
            assert_ne!(ids[i], ids[j], "cases {i} and {j} differ");
        }
    }
    let mut t = base_tests(&c);
    t.extend(all);
    let (lite, r) = checked("r07-m-hash", &file(&c, t));
    r.unwrap_or_else(|e| panic!("{e}"));
    let hash = lite.store.resolve_binding(&c.avg_log, END).unwrap().unwrap().fn_hash;
    let cases: Vec<String> = lite.store.function_tests(&hash).unwrap().into_iter().map(|r| r.case).collect();
    for id in &ids {
        assert!(cases.contains(id), "a row for case {id}: {cases:?}");
    }
    assert_eq!(cases.len(), 8, "{cases:?}");
}

// ---------------------------------------------------------------------------------------------------------------------
// whatif

async fn mesh(label: &str) -> Served {
    serve_lite(label, &whatif_lines(&calc()), None, |_| {}).await
}

fn reading(c: &Calc, m: &Served, site: &str) -> Value {
    cell_json(m, &c.reading, &[("site", "string", site)])
}

fn q(c: &Calc, site: &str) -> String {
    format!("{}(site: \"{site}\")", c.reading)
}

fn nv(v: &str) -> Value {
    json!({"type": "number", "value": v})
}

fn body(c: &Calc, overrides: Value) -> Value {
    json!({"cell": format!("{}(scale: 1)", c.all_sum), "overrides": overrides})
}

/// The canonical members override (5.9): `members` sorted by cell id, each `{"cell": <cell 4.4>, "value": <typed scalar>}`, no
/// `add` or `remove`.
fn canonical_members(slice: &Value, members: &[(Value, &str)]) -> Value {
    let mut items: Vec<(String, Value)> =
        members.iter().map(|(cell, v)| (core_id(cell), json!({"cell": cell, "value": nv(v)}))).collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    json!({"slice": slice, "members": items.into_iter().map(|x| x.1).collect::<Vec<_>>()})
}

/// `{"slice", "members"}` replaces the member set: `all_sum(scale: 1)` over `reading(site: *)` (stored: N = 10, S = 30) with
/// members A = 7 and W = 5 is 12; the scenario echoes the override canonically: the slice object, the members sorted by cell id
/// (given here in the opposite order), no `add` or `remove`. Nothing is written, and the added cells are not served at `/cell`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r3_whatif_members_replace_the_member_set_and_are_echoed_sorted() {
    let c = calc();
    let m = mesh("r07-m-wi").await;
    let slice = slice_json(&m, &c.reading, &[], &["site"]);
    // A = 7 and W = 5, given in the opposite order of their cell ids
    let (a, w) = (reading(&c, &m, "A"), reading(&c, &m, "W"));
    let mut by_id = vec![("A", "7", a.clone()), ("W", "5", w.clone())];
    by_id.sort_by_key(|x| core_id(&x.2));
    let given: Vec<Value> =
        by_id.iter().rev().map(|(site, v, _)| json!({"cell": q(&c, site), "value": nv(v)})).collect();
    let given = Value::Array(given);
    let before = snapshot(&m.lite);
    let r = m.whatif(&body(&c, json!([{"slice": format!("{}(site: *)", c.reading), "members": given}]))).await;
    assert_eq!(scenario_value(&r), "12", "{}", String::from_utf8_lossy(&r.body));
    let want = canonical_members(&slice, &[(a.clone(), "7"), (w.clone(), "5")]);
    assert_eq!(r.json["scenario"]["overrides"], json!([want]), "{}", String::from_utf8_lossy(&r.body));
    let o = &r.json["scenario"]["overrides"][0];
    assert!(o.get("add").is_none() && o.get("remove").is_none(), "{o}");
    assert_eq!(snapshot(&m.lite), before, "a scenario writes nothing");
    let served = m.get_any(&format!("/cell/{}?site=W", c.reading)).await;
    assert_eq!(served.status, 404, "an added cell is not served as a value of the cell");
}

/// An empty replacement: `all_sum` over no members is 0 (`sum` of an empty slice), and the echo has `members: []`, present when
/// empty, with no `add` or `remove`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r3_whatif_empty_members_are_echoed_present() {
    let c = calc();
    let m = mesh("r07-m-wi-empty").await;
    let slice = slice_json(&m, &c.reading, &[], &["site"]);
    let r = m.whatif(&body(&c, json!([{"slice": format!("{}(site: *)", c.reading), "members": []}]))).await;
    assert_eq!(scenario_value(&r), "0", "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json["scenario"]["overrides"], json!([{"slice": slice, "members": []}]), "{}", r.json);
}

/// `members` with `add` or `remove` is malformed: `bad_request` with `E_SLICE_MEMBER`; so is a member that is not a cell of the
/// slice (another function's cell), and two overrides of one slice (`E_CANON_ARG_DUPLICATE`), whichever forms they take.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r3_whatif_members_errors() {
    let c = calc();
    let m = mesh("r07-m-wi-bad").await;
    let s = format!("{}(site: *)", c.reading);
    let one = json!([{"cell": q(&c, "W"), "value": nv("5")}]);
    let cases: Vec<(&str, Value, &str)> = vec![
        ("members and add", json!([{"slice": s, "members": one, "add": one}]), "E_SLICE_MEMBER"),
        ("members and remove", json!([{"slice": s, "members": one, "remove": [q(&c, "N")]}]), "E_SLICE_MEMBER"),
        ("members, empty add", json!([{"slice": s, "members": [], "add": []}]), "E_SLICE_MEMBER"),
        (
            "a member of another function",
            json!([{"slice": s, "members": [{"cell": format!("{}(site: \"W\")", c.double), "value": nv("5")}]}]),
            "E_SLICE_MEMBER",
        ),
        (
            "two overrides of one slice",
            json!([{"slice": s, "members": one}, {"slice": s, "add": one, "remove": []}]),
            "E_CANON_ARG_DUPLICATE",
        ),
        (
            "two members overrides of one slice",
            json!([{"slice": s, "members": one}, {"slice": s, "members": []}]),
            "E_CANON_ARG_DUPLICATE",
        ),
    ];
    for (what, ov, code) in cases {
        let r = m.whatif(&body(&c, ov)).await;
        assert_eq!(
            (r.status, r.json["outcome"].as_str(), r.json["error"].as_str()),
            (400, Some("bad_request"), Some(code)),
            "{what}: {}",
            r.json
        );
    }
    // a members override that is not an array is a bad request too
    let r = m.whatif(&body(&c, json!([{"slice": s, "members": {}}]))).await;
    assert_eq!((r.status, r.json["outcome"].as_str()), (400, Some("bad_request")), "{}", r.json);
}
