//! The 0.7 round (CONTRACT-round-0.7.md), R1: **withdrawal is the null version** (SPEC 3.6, 5.5, 8.1, 10.1, 15.11-15.13, 16.1, 16.7,
//! D123). Failing tests: the behaviour is not written yet.
//!
//! * A publish line `{"cell": ..., "withdrawn": "<reason>"}` (signed by the namespace's key like any stored cell line) writes a
//!   withdrawal: a receipt whose `withdrawn` takes the place of `type` and `value`, with no inputs.
//! * A read whose latest version as of the asof is a withdrawal is `not_covered` (404) with `receipt` (the withdrawal), `trust`
//!   and `reason` (its `withdrawn` text), and no `fault` and no `error`; `/cells` answers it the same; `/slice` and a slice
//!   read of a formula leave the cell out; `/versions` lists it; a computed cell over it, read alone, is plain `not_covered`
//!   (`E_FORMULA_NOT_COVERED`: SPEC 15.12 says it has no `fault` member, which the coordinator's "fault per 15.11" is);
//!   a later valued line ends the withdrawal.
//! * A label is withdrawn by a withdrawal of its lookup cell: a request through it is `not_covered` with `reason` and no
//!   receipt; the lookup's return type no longer allows `""` (`E_KEY_NORMALISATION` at publish).
//! * A test's `inputs` entry for a cell may be `{"withdrawn": "<reason>"}`.
//! * Malformed withdrawals are refused at publish, and nothing is written: an empty reason, a non-string, both `withdrawn` and
//!   `value`, a control character in the reason.
//!
//! The world is the calculus one (`reading(site)`: N = 10, S = 30; `log(site, day)`; the formulas `double`, `all_sum`) published
//! at `NOW`; the withdrawal of `reading(site: "S")` is a second run at `T2`, a valued line that ends it a third at `T3`.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/crworld.rs"]
mod crworld;

use calculus::*;
use common::lite::*;
use common::pub031::{function_line, refused_whole, T2};
use serde_json::{json, Value};

const T3: &str = "2026-11-15T12:00:03.000000Z";
const BETWEEN: &str = "2026-11-15T12:00:01.000000Z";
const REASON: &str = "site S closed";

fn withdrawal(function: &str, args: Value, reason: &str) -> String {
    json!({"cell": {"fn": function, "args": args}, "licence": "CC0-1.0", "summary": "Synthetic test data.",
        "withdrawn": reason})
    .to_string()
}

fn site(s: &str) -> Value {
    json!({"site": {"type": "string", "value": s}})
}

/// The calculus world at `NOW`, `reading(S)` withdrawn at `T2`; with `ended`, a valued line (31) at `T3`.
async fn world(label: &str, ended: bool) -> common::readpath::Served {
    let c = calc();
    let mut runs: Vec<(&'static str, Vec<String>)> =
        vec![(NOW, whatif_lines(&c)), (T2, vec![withdrawal(&c.reading, site("S"), REASON)])];
    if ended {
        runs.push((T3, vec![site_cell(&c.reading, "S", "31")]));
    }
    serve_runs(label, &runs, None).await
}

fn reading_path(c: &Calc, s: &str, asof: Option<&str>) -> String {
    let mut p = format!("/cell/{}?site={s}", c.reading);
    if let Some(a) = asof {
        p.push_str(&format!("&asof={a}"));
    }
    p
}

/// A read of a withdrawn cell: 404 `not_covered`, `receipt` the withdrawal (`withdrawn`, and neither `type` nor `value`, no
/// inputs), `trust` `primary`, `reason` the withdrawn text; no `fault`, no `error`.
fn assert_withdrawn(a: &crworld::Ans, reason: &str) {
    assert_eq!((a.status, a.outcome()), (404, "not_covered"), "{}", a.text);
    let r = &a.v["receipt"];
    assert_eq!(r["withdrawn"], reason, "{}", a.text);
    assert!(r.get("value").is_none() && r.get("type").is_none(), "a withdrawal has no value: {}", a.text);
    assert_eq!(r["inputs"], json!([]), "{}", a.text);
    assert_eq!(a.v["trust"], "primary", "{}", a.text);
    assert_eq!(a.v["reason"], reason, "{}", a.text);
    assert!(a.v.get("fault").is_none() && a.v.get("error").is_none(), "{}", a.text);
}

// ---------------------------------------------------------------------------------------------------------------------
// reading

/// `GET /cell` of the withdrawn cell: the withdrawal answers from `T2` on; before it the old value answers (history stays).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r1_a_read_of_a_withdrawn_cell_is_not_covered_with_the_withdrawal() {
    let c = calc();
    let m = world("r07-wd-read", false).await;
    for asof in [None, Some(T2)] {
        let a = crworld::ask(&m, &reading_path(&c, "S", asof)).await;
        assert_withdrawn(&a, REASON);
    }
    let was = crworld::ask(&m, &reading_path(&c, "S", Some(BETWEEN))).await;
    assert_eq!((was.outcome(), was.value()), ("ok", Some("30")), "as of {BETWEEN}: {}", was.text);
    // the other cell is untouched
    let n = crworld::ask(&m, &reading_path(&c, "N", None)).await;
    assert_eq!(n.value(), Some("10"), "{}", n.text);
}

/// The withdrawal is the cell's own receipt, signed by the namespace's key: `/receipt/<hash>` of the version `/versions` lists
/// last is the receipt of the answer, and `/versions` lists the withdrawal like any version (the old value first).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r1_versions_list_the_withdrawal() {
    let c = calc();
    let m = world("r07-wd-versions", false).await;
    let a = crworld::ask(&m, &reading_path(&c, "S", None)).await;
    let v = m.get(&format!("/versions/{}?site=S", c.reading)).await;
    let vs = v["versions"].as_array().unwrap_or_else(|| panic!("{v}"));
    assert_eq!(vs.len(), 2, "{v}");
    assert!(vs[0]["tx"].as_str() < vs[1]["tx"].as_str(), "ascending by tx: {v}");
    assert_eq!(vs[1]["tx"], T2, "{v}");
    let hash = vs[1]["receipt"].as_str().unwrap();
    let r = m.get(&format!("/receipt/{hash}")).await;
    assert_eq!(r, a.v["receipt"], "the withdrawal listed is the withdrawal answered");
    assert_eq!(r["withdrawn"], REASON, "{r}");
    // the first version is the value
    let first = m.get(&format!("/receipt/{}", vs[0]["receipt"].as_str().unwrap())).await;
    assert_eq!(first["value"], "30", "{first}");
}

/// `POST /cells` answers a withdrawn cell as `/cell` does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r1_cells_answers_a_withdrawn_cell_as_cell_does() {
    let c = calc();
    let m = world("r07-wd-cells", false).await;
    let body = format!(r#"["{}?site=S", "{}?site=N"]"#, c.reading, c.reading);
    let r = m.post_raw("/cells", body.into_bytes()).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let items = r.json.as_array().unwrap_or_else(|| panic!("{}", String::from_utf8_lossy(&r.body)));
    assert_eq!(items.len(), 2);
    let alone = crworld::ask(&m, &reading_path(&c, "S", None)).await;
    let e = &items[0]["envelope"];
    assert_eq!(e["outcome"], "not_covered", "{e}");
    assert_eq!(e["receipt"], alone.v["receipt"], "{e}");
    assert_eq!((e["trust"].as_str(), e["reason"].as_str()), (Some("primary"), Some(REASON)), "{e}");
    assert!(e.get("fault").is_none() && e.get("error").is_none(), "{e}");
    assert_eq!(items[1]["envelope"]["outcome"], "ok", "{}", items[1]);
}

// ---------------------------------------------------------------------------------------------------------------------
// slices

/// `/slice` leaves a withdrawn cell out of the members (and, as of before the withdrawal, has it), and a formula that reads the
/// slice (`all_sum`) sees the members without it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r1_a_slice_leaves_the_withdrawn_cell_out() {
    let c = calc();
    let m = world("r07-wd-slice", false).await;
    let sites = |v: &Value| -> Vec<String> {
        v["members"]
            .as_array()
            .unwrap_or_else(|| panic!("{v}"))
            .iter()
            .map(|x| x["receipt"]["cell"]["args"]["site"]["value"].as_str().unwrap().to_string())
            .collect()
    };
    let now = m.get(&format!("/slice/{}?site=*", c.reading)).await;
    assert_eq!(sites(&now), ["N"], "{now}");
    let before = m.get(&format!("/slice/{}?site=*&asof={BETWEEN}", c.reading)).await;
    assert_eq!(sites(&before), ["N", "S"], "{before}");
    // a formula over the slice: 10 now, 40 before
    let sum = crworld::ask(&m, &format!("/cell/{}?scale=1", c.all_sum)).await;
    assert_eq!((sum.outcome(), sum.value()), ("ok", Some("10")), "{}", sum.text);
    let was = crworld::ask(&m, &format!("/cell/{}?scale=1&asof={BETWEEN}", c.all_sum)).await;
    assert_eq!((was.outcome(), was.value()), ("ok", Some("40")), "{}", was.text);
    // the withdrawn cell is not in the lineage: the slice's manifest lists the one member
    let manifest_members = sum.v["receipt"]["inputs"].as_array().unwrap();
    assert_eq!(manifest_members.len(), 1, "one slice entry: {}", sum.text);
}

/// A computed cell over a withdrawn input, read alone, has no value: `E_FORMULA_NOT_COVERED` is plain `not_covered` (no
/// receipt, no `fault`, no `error`); before the withdrawal it computes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r1_a_computed_cell_over_a_withdrawn_input_is_not_covered() {
    let c = calc();
    let m = world("r07-wd-computed", false).await;
    let a = crworld::ask(&m, &format!("/cell/{}?site=S", c.double)).await;
    assert_eq!((a.status, a.outcome()), (404, "not_covered"), "{}", a.text);
    assert!(a.v.get("receipt").is_none() && a.v.get("fault").is_none() && a.v.get("error").is_none(), "{}", a.text);
    let n = crworld::ask(&m, &format!("/cell/{}?site=N", c.double)).await;
    assert_eq!(n.value(), Some("20"), "{}", n.text);
    let was = crworld::ask(&m, &format!("/cell/{}?site=S&asof={BETWEEN}", c.double)).await;
    assert_eq!(was.value(), Some("60"), "{}", was.text);
}

/// A later version with a value ends the withdrawal: now it answers 31; as of `T2` it is still withdrawn, and as of before the
/// withdrawal it is 30; the slice has both members again; `/versions` lists three.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r1_a_later_valued_line_ends_the_withdrawal() {
    let c = calc();
    let m = world("r07-wd-ended", true).await;
    let now = crworld::ask(&m, &reading_path(&c, "S", None)).await;
    assert_eq!((now.outcome(), now.value()), ("ok", Some("31")), "{}", now.text);
    assert_withdrawn(&crworld::ask(&m, &reading_path(&c, "S", Some(T2))).await, REASON);
    let was = crworld::ask(&m, &reading_path(&c, "S", Some(BETWEEN))).await;
    assert_eq!(was.value(), Some("30"), "{}", was.text);
    let sum = crworld::ask(&m, &format!("/cell/{}?scale=1", c.all_sum)).await;
    assert_eq!(sum.value(), Some("41"), "{}", sum.text);
    let slice = m.get(&format!("/slice/{}?site=*", c.reading)).await;
    assert_eq!(slice["members"].as_array().unwrap().len(), 2, "{slice}");
    let v = m.get(&format!("/versions/{}?site=S", c.reading)).await;
    assert_eq!(v["versions"].as_array().unwrap().len(), 3, "{v}");
}

// ---------------------------------------------------------------------------------------------------------------------
// labels

/// The shop's label `old` is withdrawn by a withdrawal of its lookup cell (`crworld::shop`, second run at `T2`): a request through
/// the label is `not_covered` with `reason` and no receipt and no trust; the lookup cell itself answers its withdrawal; as of
/// before it the label names the object; a third run that stores the label again (`T3`) ends the withdrawal.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r1_a_label_withdrawn_by_its_lookup_cell_answers_not_covered_with_a_reason() {
    use crworld::*;
    let m = shop("r07-wd-label", true).await;
    let call = |l: &str| format!("{}[{l}].price({SHOP_DAY})", product());
    let a = ask(&m, &path(&call("old"), None)).await;
    assert_eq!((a.status, a.outcome(), a.error()), (404, "not_covered", None), "{}", a.text);
    assert_eq!(a.v["reason"], OLD_WITHDRAWN, "{}", a.text);
    assert!(a.v.get("receipt").is_none() && a.v.get("trust").is_none(), "no cell was resolved: {}", a.text);
    // a folded spelling is the same label
    let b = ask(&m, &path(&call("OLD"), None)).await;
    assert_eq!(b.v["reason"], OLD_WITHDRAWN, "{}", b.text);
    // the lookup cell is the withdrawal, with its receipt and trust
    let cell = ask(&m, &format!("/cell/{}/from_label?label=old", product())).await;
    assert_withdrawn(&cell, OLD_WITHDRAWN);
    // before the withdrawal the label named A3
    let was = ask(&m, &path(&call("old"), Some(BETWEEN))).await;
    assert_eq!((was.outcome(), was.value()), ("ok", Some("7")), "{}", was.text);
    // `describe` of the label is not found while it is withdrawn
    let d = ask(&m, &format!("/describe/{}", common::world::call_path(&format!("{}[old]", product())))).await;
    assert_eq!(d.status, 404, "{}", d.text);
    // the others are untouched
    assert_eq!(ask(&m, &path(&call("apple"), None)).await.value(), Some("1.5"));
}

/// A label stored again after its withdrawal names the object again (the withdrawal is ended by a later valued version); as of
/// the instant between, it is still withdrawn.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r1_a_label_reassigned_after_its_withdrawal_resolves_again() {
    use crworld::*;
    let lite = Lite::new("r07-wd-label-back");
    publish(&lite, "functions.jsonl", &shop_functions(), common::pub031::BEFORE).unwrap_or_else(|e| panic!("{e}"));
    let mut v = shop_classes(&lite);
    v.extend(shop_cells());
    publish(&lite, "classes.jsonl", &v, NOW).unwrap_or_else(|e| panic!("{e}"));
    let w = withdrawal_line(&format!("{}/from_label", product()), &[("label", "string", "old")], OLD_WITHDRAWN);
    publish(&lite, "withdraw.jsonl", &[w], T2).unwrap_or_else(|e| panic!("withdrawal: {e}"));
    publish(&lite, "again.jsonl", &[label_cell("old", &pk("A2"))], T3).unwrap_or_else(|e| panic!("again: {e}"));
    let m = common::readpath::build_served(lite, None, |_| {}).await;
    let call = format!("{}[old].price({SHOP_DAY})", product());
    let now = ask(&m, &path(&call, None)).await;
    assert_eq!((now.outcome(), now.value()), ("ok", Some("2.5")), "now it names A2: {}", now.text);
    let between = ask(&m, &path(&call, Some("2026-11-15T12:00:02.500000Z"))).await;
    assert_eq!(between.v["reason"], OLD_WITHDRAWN, "{}", between.text);
}

/// The lookup's return type no longer allows `""`: a lookup cell stored as the empty string is refused at publish
/// (`E_KEY_NORMALISATION`, as for any key), at its line, and nothing is written.
#[test]
fn r1_a_lookup_cell_stored_as_the_empty_string_is_refused() {
    use crworld::*;
    let lite = Lite::new("r07-wd-label-empty");
    publish(&lite, "functions.jsonl", &shop_functions(), common::pub031::BEFORE).unwrap_or_else(|e| panic!("{e}"));
    let mut v = shop_classes(&lite);
    v.push(label_cell("apple", &pk("A1")));
    publish(&lite, "classes.jsonl", &v, NOW).unwrap_or_else(|e| panic!("{e}"));
    let e = refused_whole(&lite, "empty.jsonl", &[label_cell("old", "")], 1, T2);
    assert!(e.detail.contains("E_KEY_NORMALISATION"), "{e}");
}

// ---------------------------------------------------------------------------------------------------------------------
// malformed withdrawals

/// A withdrawal that is not a non-empty string under the string rules is refused at its line and nothing is written; so is a
/// line that has both `withdrawn` and `value` (SPEC 8.1: `E_SCHEMA`); a valid one is accepted.
#[test]
fn r1_malformed_withdrawals_are_refused() {
    let c = calc();
    let lite = Lite::new("r07-wd-bad");
    let f = [function_line(&c.reading, &reading_sig(), None), site_cell(&c.reading, "N", "10")];
    crworld::publish(&lite, "base.jsonl", &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    let line = |withdrawn: Value| {
        json!({"cell": {"fn": c.reading, "args": site("N")}, "licence": "CC0-1.0", "summary": "Synthetic test data.",
            "withdrawn": withdrawn})
        .to_string()
    };
    for (what, bad, code) in [
        ("an empty reason", line(json!("")), "E_SCHEMA"),
        ("a null reason", line(json!(null)), "E_SCHEMA"),
        ("a number as the reason", line(json!(7)), "E_SCHEMA"),
        ("a control character", line(json!("a\tb")), "E_CANON_STRING"),
        ("a reason over the limit", line(json!("x".repeat(1025))), "E_CANON_LIMIT"),
        (
            "withdrawn and value",
            json!({"cell": {"fn": c.reading, "args": site("N")}, "licence": "CC0-1.0", "summary": "Synthetic test data.",
                "withdrawn": "why", "value": {"type": "number", "value": "1"}})
            .to_string(),
            "E_SCHEMA",
        ),
    ] {
        let e = refused_whole(&lite, "bad.jsonl", &[bad], 1, T2);
        assert!(e.detail.contains(code), "{what}: {e}");
    }
    crworld::publish(&lite, "good.jsonl", &[line(json!("a reason"))], T2).unwrap_or_else(|e| panic!("{e}"));
}

// ---------------------------------------------------------------------------------------------------------------------
// a test's inputs

fn data(c: &Calc) -> Vec<String> {
    data_lines(c)
}

fn log_cell_call(c: &Calc, day: &str) -> String {
    format!("{}(site: \"N\", day: {day})", c.log)
}

/// `avg_log` tests (the public rule, SPEC 7.3): positive, negative scale, and the empty slice reached by **withdrawing** both of
/// N's log days with `{"withdrawn": ...}` entries of `inputs`: the withdrawn cells are not members, so `mean` is empty.
#[test]
fn r1_a_test_input_may_withdraw_cells() {
    let c = calc();
    let empty = test_line(
        &c.avg_log,
        &avg_call(&c, "N", "1"),
        Some(json!({
            log_cell_call(&c, "2026-03-01"): {"withdrawn": "no reading"},
            log_cell_call(&c, "2026-03-02"): {"withdrawn": "no reading"},
        })),
        None,
        fault("E_FORMULA_EMPTY"),
    );
    let mut lines = data(&c);
    lines.push(avg_log_line(&c));
    lines.push(test_line(&c.avg_log, &avg_call(&c, "N", "2"), None, None, num("6")));
    lines.push(test_line(&c.avg_log, &avg_call(&c, "N", "-1"), None, None, num("-3")));
    lines.push(empty);
    let (_, r) = checked("r07-wd-input", &lines);
    r.unwrap_or_else(|e| panic!("{e}"));
}

/// One withdrawn member of two: `mean` is over the other (the withdrawn day is not a member): N's day worth 2 withdrawn leaves 4.
/// A wrong expectation (the store's mean) fails the test, and the failure says what came out.
#[test]
fn r1_a_withdrawn_input_is_not_a_slice_member_in_a_test() {
    let c = calc();
    let base = |tests: Vec<String>| {
        let mut lines = data(&c);
        lines.push(avg_log_line(&c));
        lines.push(test_line(&c.avg_log, &avg_call(&c, "N", "2"), None, None, num("6")));
        lines.push(test_line(&c.avg_log, &avg_call(&c, "N", "-1"), None, None, num("-3")));
        lines.push(test_line(&c.avg_log, &avg_call(&c, "ZZ", "1"), None, None, fault("E_FORMULA_EMPTY")));
        lines.extend(tests);
        lines
    };
    let one = |want: &str| {
        test_line(
            &c.avg_log,
            &avg_call(&c, "N", "1"),
            Some(json!({ log_cell_call(&c, "2026-03-01"): {"withdrawn": "gone"} })),
            None,
            num(want),
        )
    };
    let (_, r) = checked("r07-wd-input-one", &base(vec![one("4")]));
    r.unwrap_or_else(|e| panic!("{e}"));
    let e = refused("r07-wd-input-one-wrong", &base(vec![one("3")]), 11, "E_TESTS_FAILING", &[]);
    assert!(e.detail.contains('3') && e.detail.contains('4'), "{e}");
}

/// A test input `{"withdrawn": ""}`, or one with `withdrawn` and a type, is refused at the test's line; the same test with a
/// good reason is accepted (the control, so that the refusals are about the reason and not about the form).
#[test]
fn r1_a_malformed_withdrawn_test_input_is_refused() {
    let c = calc();
    {
        let mut lines = data(&c);
        lines.push(avg_log_line(&c));
        lines.push(test_line(&c.avg_log, &avg_call(&c, "N", "2"), None, None, num("6")));
        lines.push(test_line(&c.avg_log, &avg_call(&c, "N", "-1"), None, None, num("-3")));
        lines.push(test_line(&c.avg_log, &avg_call(&c, "ZZ", "1"), None, None, fault("E_FORMULA_EMPTY")));
        lines.push(test_line(
            &c.avg_log,
            &avg_call(&c, "N", "1"),
            Some(json!({ log_cell_call(&c, "2026-03-01"): {"withdrawn": "why"} })),
            None,
            num("4"),
        ));
        let (_, r) = checked("r07-wd-input-control", &lines);
        r.unwrap_or_else(|e| panic!("control: {e}"));
    }
    for (i, entry) in
        [json!({"withdrawn": ""}), json!({"withdrawn": 5}), json!({"withdrawn": "why", "type": "number", "value": "1"})]
            .into_iter()
            .enumerate()
    {
        let mut lines = data(&c);
        lines.push(avg_log_line(&c));
        lines.push(test_line(&c.avg_log, &avg_call(&c, "N", "2"), None, None, num("6")));
        lines.push(test_line(&c.avg_log, &avg_call(&c, "N", "-1"), None, None, num("-3")));
        lines.push(test_line(&c.avg_log, &avg_call(&c, "ZZ", "1"), None, None, fault("E_FORMULA_EMPTY")));
        lines.push(test_line(
            &c.avg_log,
            &avg_call(&c, "N", "1"),
            Some(json!({ log_cell_call(&c, "2026-03-01"): entry })),
            None,
            num("4"),
        ));
        let lite = Lite::new(&format!("r07-wd-input-bad-{i}"));
        let f = common::pub031::write(&lite, "f.jsonl", &lines);
        let before = common::pub031::snapshot(&lite);
        let e = run_checked(&lite, &f, NOW).expect_err("the file must be refused");
        assert_eq!(e.line, 11, "{e}");
        assert_eq!(common::pub031::snapshot(&lite), before, "nothing written: {e}");
    }
}
