//! Build 2 Part 7 (CONTRACT-build2-part7-sec K4, K5, K6) as changed by CONTRACT-sec-rebuild.md N4: the normalised views and
//! the ratio library of `deploy/prod/feeds/sec_views.jsonl`, published as `publish-stored` does on mesh-test, with the public
//! test rule ON (`publish_stored_checked`, the call the binary makes on datumesh-http), over stub relay
//! cells for two companies and a synthetic restated one (`deploy/prod/feeds/fixtures/sec_views_stub_relay.jsonl`: Caterpillar
//! and JPMorgan Chase, the SEC's companyfacts as filed, fiscal years 2019 to 2024; and cik 0000000777, whose fiscal 2020
//! figures four filings report with four values), generated from `sec.emit` by `fixtures/make_sec_stub.py`, so the relay
//! the views read is the relay the fetcher writes. Needs the unreleased core 0.3.11 and formula 0.5.2 (`last_by`, `public:`).
//!
//! The mesh is on PostgreSQL (`DATABASE_URL`). Checked here:
//!
//! * the file is accepted with the rule on, which runs every `test` line and demands the derived cases
//!   and every branch arm, and a second run changes nothing;
//! * every stored test row is a pass, and there is one row for every `test` line;
//! * the catalogue's examples, and the chains that fall back from tag to tag, give the same answers
//!   when the live mesh evaluates them (the harness of the rule is not the engine);
//! * the rule is really on: without the tests, or without one of them, the file is refused;
//! * the views inherit the SEC's `public:` licence and need NO declaration: the key document has none, and every view of
//!   `sec/` answers ok with the licence `public:https://www.sec.gov/about/privacy-information`;
//! * `sec/latest` is exact (`last_by` over the period's members by the filing class's `filed()`): on the restated company
//!   it gives the figure of the latest filing three fiscal years on, and it agrees with a direct reading of the cells.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::lite::{Lite, END, HOST};
use common::pub031::{opts, write};
use common::{key_doc, key_for, Node, Setup, TestDb, BIND_TX};
use datumesh_http::publish::{publish_stored_checked, publish_stored_with, PublishErrorKind};
use datumesh_store::Store;
use serde_json::Value;

/// The publish clock of these tests. The stub's `filed()` cells are staged at tx 2026-11-16T12:00 (make_sec_stub.py
/// `FILED_TX`, since 756e738), and the views' tests (fbd8adf) carry no asof, so they run at the publish clock: it must be
/// at or after that tx, or `filed(accession: *)` is empty at the clock and `sec/company/latest` has no figure (K4: the common
/// NOW is 2026-11-15), and no later than it, since a binding starts at the clock and a cell is not before its binding: so
/// exactly `FILED_TX`.
const PUBLISH_NOW: &str = "2026-11-16T12:00:00.000000Z";

const SEC_LICENCE: &str = "public:https://www.sec.gov/about/privacy-information";

fn feeds() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("deploy/prod/feeds")
}

fn read_lines(p: PathBuf) -> Vec<String> {
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())).lines().map(str::to_string).collect()
}

fn stub() -> Vec<String> {
    read_lines(feeds().join("fixtures/sec_views_stub_relay.jsonl"))
}

fn views() -> Vec<String> {
    read_lines(feeds().join("sec_views.jsonl"))
}

fn json(l: &str) -> Value {
    serde_json::from_str(l).unwrap()
}

/// The names of the file's formula lines.
fn formulas(lines: &[String]) -> Vec<String> {
    lines.iter().filter_map(|l| json(l).get("formula").and_then(|f| f.as_str().map(str::to_string))).collect()
}

fn latest() -> String {
    format!("{HOST}/sec/company/latest")
}

/// The views file without `sec/company/latest` (its formula, tests and catalogue line). K4 finding: since fbd8adf that
/// formula's tests hold on the real relay, where every filing has a filed() cell, so no test reaches the derived case
/// `empty(relay/sec.gov/filing/filed)` (the staged filed() cells of the stub are visible only at an asof before them, and
/// the tests no longer carry one), and the public rule refuses the file with E_TESTS_MISSING at any clock. The tests that
/// are not about that finding use this file; `latest_publishes_under_the_public_rule` is the one that pins it.
fn views_without_latest() -> Vec<String> {
    let l = latest();
    views()
        .into_iter()
        .filter(|x| {
            let v = json(x);
            ![v.get("formula"), v.get("test"), v.get("catalog")].iter().flatten().any(|n| *n == l.as_str())
        })
        .collect()
}

fn publish_all(
    store: &dyn Store,
    key: &datumesh_core::SigningKey,
    dir: &PathBuf,
    lines: &[String],
) -> Result<datumesh_http::publish::Published, datumesh_http::publish::PublishError> {
    std::fs::create_dir_all(dir).unwrap();
    let p = dir.join("sec_views_all.jsonl");
    std::fs::write(&p, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    publish_stored_checked(store, key, HOST, &[HOST.to_string()], &p, &opts(PUBLISH_NOW))
}

fn tmp(label: &str) -> PathBuf {
    common::out_dir().join(format!("sec-views-{label}-{}", std::process::id()))
}

/// `name(k: v, ...)` as a `/cell/<ns>/<fn>?k=v` path: strings are quoted, numbers and dates are bare.
fn path_of(call: &str) -> String {
    let (name, rest) = call.split_once('(').unwrap();
    let args = rest.strip_suffix(')').unwrap();
    let mut q = Vec::new();
    for a in args.split(", ") {
        let (k, v) = a.split_once(": ").unwrap();
        q.push(format!("{k}={}", v.trim_matches('"').replace('/', "%2F")));
    }
    q.sort();
    format!("/cell/{name}?{}", q.join("&"))
}

async fn answer(node: &Node, call: &str) -> Value {
    let r = node.get(&path_of(call)).await;
    serde_json::from_slice(&r.body).unwrap_or_else(|e| panic!("{call}: not JSON ({e}): {}", r.text()))
}

fn value_of(v: &Value) -> String {
    v["receipt"]["value"].as_str().unwrap_or_else(|| panic!("no value: {v}")).to_string()
}

async fn expect_value(node: &Node, call: &str, want: &str) {
    let v = answer(node, call).await;
    assert_eq!(v["outcome"], "ok", "{call}: {v}");
    assert_eq!(value_of(&v), want, "{call}: {v}");
}

async fn expect_not_covered(node: &Node, call: &str, fault: Option<&str>) {
    let v = answer(node, call).await;
    assert_eq!(v["outcome"], "not_covered", "{call}: {v}");
    if let Some(f) = fault {
        assert_eq!(v["fault"], f, "{call}: {v}");
    }
}

/// The public rule accepts the whole file, every test passes, and the live engine agrees.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_views_publish_with_the_public_rule_and_every_test_passes() {
    let db = Arc::new(TestDb::migrated());
    let key = key_for(HOST);
    let store = Arc::new(common::bl(|| db.store(8)));
    common::bl(|| store.put_namespace(HOST, &key_doc(HOST, &key), BIND_TX).unwrap());
    let mut lines = stub();
    let stub_lines = lines.len();
    lines.extend(views_without_latest());
    let dir = tmp("pg");
    let p = common::bl(|| publish_all(&*store, &key, &dir, &lines)).unwrap_or_else(|e| panic!("{e}"));
    let formula_names = formulas(&lines);
    // the three stored functions of the stub's sec/company file (annual_report, loaded, from_label: 98100ab), the two
    // relay classes (relay/sec.gov/company with its method fact, and the filing class with its six stored methods, whose
    // descriptors are descriptor lines, not functions) and the formulas
    assert_eq!(p.functions, 3 + formula_names.len(), "{p:?}");
    assert_eq!(p.classes, 2, "{p:?}");
    assert!(formula_names.contains(&format!("{HOST}/calendar/age_at")) && formula_names.contains(&format!("{HOST}/calendar/days_between")));
    assert!(!formula_names.contains(&format!("{HOST}/ratio/age_at")) && !formula_names.contains(&format!("{HOST}/ratio/days_between")));
    assert!(formula_names.len() >= 28, "the K4 views and the K5 ratios: {}", formula_names.len());
    assert!(p.cells > 300 && stub_lines > 300, "{p:?}");

    // a second run changes nothing
    let again = common::bl(|| publish_all(&*store, &key, &dir, &lines)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((again.functions, again.classes, again.versions), (0, 0, 0), "{again:?}");

    // one stored row for each test line, each a pass
    let tests: Vec<Value> =
        views_without_latest().iter().map(|l| json(l)).filter(|v| v.get("test").is_some()).collect();
    let mut total = 0;
    for f in &formula_names {
        let hash =
            common::bl(|| store.resolve_binding(f, END).unwrap().unwrap_or_else(|| panic!("{f} unbound")).fn_hash);
        let want = tests.iter().filter(|t| t["test"] == f.as_str()).count();
        // sec/company/fact (98100ab) is the subclass's override of the relay's fact: a thin formula, with two tests
        let least = if f.ends_with("/sec/company/fact") { 2 } else { 4 };
        assert!(want >= least, "{f} has {want} tests");
        let rows = common::bl(|| store.function_tests(&hash).unwrap());
        assert_eq!(rows.len(), want, "{f}: stored rows");
        for r in &rows {
            assert_eq!(r.last_result.as_deref(), Some("pass"), "{f}: {:?}", r.last_detail);
            assert_eq!(r.branches_covered, r.branches_total, "{f}: every branch arm taken");
        }
        total += rows.len();
    }
    assert_eq!(total, tests.len());
    println!("sec_views: {} formulas, {} tests, {} stub cells", formula_names.len(), total, p.cells);

    // the live mesh
    let node = Node::start_with(Setup { db: Some(db.clone()), ..Setup::new(HOST) }, |_| {}, |b| b).await;
    // No declaration (spec-0.6.2 P4): the relay cells carry `public:<the SEC's terms page>`, an open licence, so a view over
    // them inherits it without the steward declaring anything. The key document of the host has no `declarations`.
    let keydoc = common::bl(|| store.namespace(HOST).unwrap().unwrap().keydoc);
    let kd: Value = serde_json::from_slice(&keydoc).unwrap();
    assert!(kd.get("declarations").is_none(), "the views need no declaration: {kd}");
    // the examples of the catalogue lines; a view of sec/ answers with the public licence
    let mut examples = 0;
    for l in views_without_latest() {
        let v = json(&l);
        if v.get("catalog").is_none() {
            continue;
        }
        let name = v["catalog"].as_str().unwrap().to_string();
        for e in v["examples"].as_array().unwrap() {
            let call = e["call"].as_str().unwrap();
            expect_value(&node, call, e["expected"]["value"].as_str().unwrap()).await;
            if name.starts_with(&format!("{HOST}/sec/")) {
                let a = answer(&node, call).await;
                assert_eq!(a["receipt"]["licence"], SEC_LICENCE, "{call}: {a}");
            }
            examples += 1;
        }
    }
    assert!(examples >= formula_names.len(), "an example for each function: {examples}");

    let s = |f: &str| format!("{HOST}/sec/company/{f}");
    let jpm = "0000019617";
    let cat = "0000018230";
    // as reported against the restated figure (JPMorgan's 2019 revenue was restated twice)
    expect_value(
        &node,
        &format!(
            "{}(cik: \"{jpm}\", concept: \"us-gaap:Revenues\", unit: \"USD\", fiscal_year: 2019, period_kind: \"annual\")",
            s("reported")
        ),
        "115627000000",
    )
    .await;
    // a tag chain falling back through a real read that has no value (Caterpillar tags no NetIncomeLoss)
    expect_value(&node, &format!("{}(cik: \"{cat}\", fiscal_year: 2023)", s("net_income")), "10332000000").await;
    // the same figures through the filing class and the selection, read directly
    let jpm_fy2019 = "0000019617-20-000257";
    let filing = |m: &str, a: &str| format!("{HOST}/relay/sec.gov/filing/{m}(accession: \"{a}\")");
    expect_value(&node, &format!("{}(cik: \"{jpm}\", fiscal_year: 2019)", s("annual_report")), jpm_fy2019).await;
    expect_value(&node, &filing("period_end", jpm_fy2019), "2019-12-31").await;
    expect_value(&node, &filing("form", jpm_fy2019), "10-K").await;
    expect_value(&node, &filing("fiscal_year", jpm_fy2019), "2019").await;
    expect_value(&node, &filing("fiscal_period", jpm_fy2019), "FY").await;
    expect_value(&node, &filing("cik", jpm_fy2019), jpm).await;
    let a = answer(&node, &format!("{}(cik: \"{jpm}\", fiscal_year: 2019)", s("annual_report"))).await;
    assert_eq!((a["trust"].as_str(), a["receipt"]["licence"].as_str()), (Some("primary"), Some(SEC_LICENCE)), "{a}");
    assert!(a["receipt"].get("source").is_none_or(Value::is_null), "no source: {a}");
    // a bank has no inventory: no value, and its fault is a fault of the cell
    expect_not_covered(&node, &format!("{}(cik: \"{jpm}\", fiscal_year: 2023)", s("inventory")), None).await;
    // the first fiscal year in scope has no opening balance
    expect_not_covered(&node, &format!("{HOST}/sec/company/return_on_assets(cik: \"{cat}\", fiscal_year: 2019)"), None)
        .await;
    // a zero divisor is a fault, not a guess
    expect_not_covered(&node, &format!("{HOST}/ratio/margin(part: 1, whole: 0)"), Some("E_FORMULA_DIV_ZERO")).await;
    expect_value(
        &node,
        &format!("{HOST}/sec/company/current_ratio(cik: \"{cat}\", fiscal_year: 2023)"),
        "1.351906242801197880672656070029947",
    )
    .await;
}

/// The rule is on: the same file is refused without its tests, and without any one derived case.
#[test]
fn the_public_rule_refuses_the_views_without_their_tests() {
    let mut all = stub();
    all.extend(views());
    let mut without = stub();
    without.extend(views_without_latest());
    // no test lines at all: the first formula without tests
    let no_tests: Vec<String> = all.iter().filter(|l| json(l).get("test").is_none()).cloned().collect();
    let lite = Lite::new("sv-none");
    let f = write(&lite, "all.jsonl", &no_tests);
    let e = publish_stored_checked(&*lite.store, &lite.key, HOST, &[HOST.to_string()], &f, &opts(PUBLISH_NOW))
        .expect_err("refused");
    assert!(matches!(e.kind, PublishErrorKind::TestsMissing(_)), "{e}");
    assert!(e.detail.contains("E_TESTS_MISSING") && e.detail.contains("sec/company/fact_in"), "{e}");
    // one test removed from each function in turn: the zero divisor of margin
    let margin = format!("{HOST}/ratio/margin");
    let drop_zero: Vec<String> = without
        .iter()
        .filter(|l| {
            let v = json(l);
            !(v.get("test").is_some_and(|t| t == margin.as_str()) && v["expect"].get("fault").is_some())
        })
        .cloned()
        .collect();
    let lite = Lite::new("sv-nozero");
    let f = write(&lite, "all.jsonl", &drop_zero);
    let e = publish_stored_checked(&*lite.store, &lite.key, HOST, &[HOST.to_string()], &f, &opts(PUBLISH_NOW))
        .expect_err("refused");
    assert!(matches!(e.kind, PublishErrorKind::TestsMissing(_)) && e.detail.contains("zero(whole)"), "{e}");
}

/// N4: `sec/latest` is exact. The restated company (cik 0000000777, `sec_helpers.restated_company`) files fiscal 2020
/// assets as 100 (A, 10-K), 110 (E, the 10-K/A of fiscal 2020), 120 (B, fiscal 2021) and 130 (D, fiscal 2023, three fiscal
/// years on); C (fiscal 2022) reports nothing of them. The latest filing that reports the period is D, so the latest figure is
/// 130, and `sec/reported` stays the figure of the fiscal year's own (latest filed) report, E's 110.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sec_latest_is_exact_over_the_restated_company() {
    let db = Arc::new(TestDb::migrated());
    let key = key_for(HOST);
    let store = Arc::new(common::bl(|| db.store(8)));
    common::bl(|| store.put_namespace(HOST, &key_doc(HOST, &key), BIND_TX).unwrap());
    let mut lines = stub();
    lines.extend(views());
    let dir = tmp("exact");
    // Without the public rule: whether `latest` can be published under it is `latest_publishes_under_the_public_rule`'s
    // question (K4 finding); here the question is its figures.
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("exact_all.jsonl");
    std::fs::write(&file, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    common::bl(|| publish_stored_with(&*store, &key, HOST, &[HOST.to_string()], &file, &opts(PUBLISH_NOW)))
        .unwrap_or_else(|e| panic!("{e}"));
    let node = Node::start_with(Setup { db: Some(db.clone()), ..Setup::new(HOST) }, |_| {}, |b| b).await;
    let s = |f: &str| format!("{HOST}/sec/company/{f}");
    let r = "0000000777";
    let call = |f: &str, concept: &str, fy: u32, kind: &str| {
        format!("{}(cik: \"{r}\", concept: \"us-gaap:{concept}\", unit: \"USD\", fiscal_year: {fy}, period_kind: \"{kind}\")", s(f))
    };
    // the selection: the latest filed of fiscal 2020 is the 10-K/A, whose accession is lower than B's
    expect_value(&node, &format!("{}(cik: \"{r}\", fiscal_year: 2020)", s("annual_report")), "0000000777-21-000005").await;
    expect_value(&node, &format!("{}(cik: \"{r}\", fiscal_year: 2023)", s("annual_report")), "0000000777-24-000004").await;
    expect_value(&node, &call("reported", "Assets", 2020, "instant"), "110").await;
    expect_value(&node, &call("latest", "Assets", 2020, "instant"), "130").await;
    expect_value(&node, &call("reported", "Revenues", 2020, "annual"), "55").await;
    expect_value(&node, &call("latest", "Revenues", 2020, "annual"), "70").await;
    expect_value(&node, &call("latest", "Assets", 2021, "instant"), "150").await;
    expect_value(&node, &call("latest", "Assets", 2022, "instant"), "200").await;
    expect_value(&node, &call("latest", "Revenues", 2023, "annual"), "80").await;

    // a direct reading of the filings: every accession's value for the period, ordered by the filed date of the filing
    let mut best: Option<(String, String, String)> = None; // (filed, accession, value)
    for accn in ["0000000777-21-000001", "0000000777-21-000005", "0000000777-22-000002", "0000000777-23-000003", "0000000777-24-000004"] {
        let v = answer(
            &node,
            &format!(
                "{HOST}/relay/sec.gov/company/fact(cik: \"{r}\", concept: \"us-gaap:Assets\", unit: \"USD\", accession: \"{accn}\", period_start: 2020-12-31, period_end: 2020-12-31)"
            ),
        )
        .await;
        if v["outcome"] != "ok" {
            assert_eq!(accn, "0000000777-23-000003", "only C reports nothing of 2020: {v}");
            continue;
        }
        let f = answer(&node, &format!("{HOST}/relay/sec.gov/filing/filed(accession: \"{accn}\")")).await;
        let filed = f["receipt"]["value"].as_str().unwrap().to_string();
        let cand = (filed, accn.to_string(), value_of(&v));
        if best.as_ref().is_none_or(|b| (&cand.0, &cand.1) > (&b.0, &b.1)) {
            best = Some(cand);
        }
    }
    let (filed, accn, value) = best.unwrap();
    assert_eq!((filed.as_str(), accn.as_str(), value.as_str()), ("2024-02-23", "0000000777-24-000004", "130"));
    expect_value(&node, &call("latest", "Assets", 2020, "instant"), &value).await;
    // JPMorgan's 2019 revenue: restated twice, the latest filing's figure
    let jpm = "0000019617";
    expect_value(
        &node,
        &format!(
            "{}(cik: \"{jpm}\", concept: \"us-gaap:Revenues\", unit: \"USD\", fiscal_year: 2019, period_kind: \"annual\")",
            s("latest")
        ),
        "115720000000",
    )
    .await;
    expect_value(
        &node,
        &format!(
            "{}(cik: \"{jpm}\", concept: \"us-gaap:Assets\", unit: \"USD\", fiscal_year: 2019, period_kind: \"instant\")",
            s("latest")
        ),
        "2686477000000",
    )
    .await;

    // and the real restatement: JPMorgan's 2019 assets, latest = the FY2021 report's figure
    expect_value(
        &node,
        &format!("{}(cik: \"0000019617\", concept: \"us-gaap:Assets\", unit: \"USD\", fiscal_year: 2019, period_kind: \"instant\")", s("latest")),
        "2686477000000",
    )
    .await;
}

/// K4 finding, pinned: the whole views file, `sec/company/latest` included, is accepted with the public rule on. It is not:
/// since fbd8adf `latest`'s tests carry no asof and hold on the real relay, where every filing has its filed() cell, so none
/// reaches the derived case `empty(relay/sec.gov/filing/filed)` (`last_by` over an empty slice), and the rule answers
/// E_TESTS_MISSING at any clock (before the stub's staged filed() cells the tests fail instead). K1 does not reach it either:
/// remove and add cannot empty an open, growing slice (CONTRACT-classes-round K1, "No clearing"). Fails until the
/// coordinator decides what the file's tests are.
#[test]
#[ignore = "pins a defect that cannot be fixed in this round: by the user's decision of 2026-11-22, `latest` stays out until test inputs can empty an open slice (CONTRACT-classes-round K1, No clearing)"]
fn latest_publishes_under_the_public_rule() {
    let lite = Lite::new("sv-latest");
    let mut all = stub();
    all.extend(views());
    let f = write(&lite, "all.jsonl", &all);
    let r = publish_stored_checked(&*lite.store, &lite.key, HOST, &[HOST.to_string()], &f, &opts(PUBLISH_NOW));
    r.unwrap_or_else(|e| panic!("{e}"));
}
