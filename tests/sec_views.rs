//! Build 2 Part 7 (CONTRACT-build2-part7-sec K4, K5, K6): the normalised views and the ratio library of
//! `deploy/prod/feeds/sec_views.jsonl`, published as `publish-stored` does on mesh-test, with the public
//! test rule ON (`publish_stored_checked`, the call the binary makes on datumesh-http), over stub relay
//! cells for two companies (`deploy/prod/feeds/fixtures/sec_views_stub_relay.jsonl`: Caterpillar and
//! JPMorgan Chase, the SEC's companyfacts as filed, fiscal years 2019 to 2024), generated from `sec.emit` by
//! `fixtures/make_sec_stub.py` (D38), so the relay the views read is the relay the fetcher writes.
//!
//! The mesh is on PostgreSQL (`DATABASE_URL`). Checked here:
//!
//! * the file is accepted with the rule on, which runs every `test` line and demands the derived cases
//!   and every branch arm, and a second run changes nothing;
//! * every stored test row is a pass, and there is one row for every `test` line;
//! * the catalogue's examples, and the chains that fall back from tag to tag, give the same answers
//!   when the live mesh evaluates them (the harness of the rule is not the engine);
//! * the rule is really on: without the tests, or without one of them, the file is refused.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::lite::{Lite, END, HOST, NOW};
use common::pub031::{opts, write};
use common::{key_doc, key_for, Node, Setup, TestDb, BIND_TX};
use datumesh_http::publish::{publish_stored_checked, PublishErrorKind};
use datumesh_store::Store;
use serde_json::Value;

const SEC_LICENCE: &str = "https://www.sec.gov/about/privacy-information";

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

fn publish_all(
    store: &dyn Store,
    key: &datumesh_core::SigningKey,
    dir: &PathBuf,
    lines: &[String],
) -> Result<datumesh_http::publish::Published, datumesh_http::publish::PublishError> {
    std::fs::create_dir_all(dir).unwrap();
    let p = dir.join("sec_views_all.jsonl");
    std::fs::write(&p, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    publish_stored_checked(store, key, HOST, &[HOST.to_string()], &p, &opts(NOW))
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
        q.push(format!("{k}={}", v.trim_matches('"')));
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
    lines.extend(views());
    let dir = tmp("pg");
    let p = common::bl(|| publish_all(&*store, &key, &dir, &lines)).unwrap_or_else(|e| panic!("{e}"));
    let formula_names = formulas(&lines);
    // the five stored relay functions of the stub (D38: company_fact, company_fact_shares, company_fact_per_share,
    // filing, filing_for) and the formulas
    assert_eq!(p.functions, 5 + formula_names.len(), "{p:?}");
    assert!(formula_names.len() >= 28, "the K4 views and the K5 ratios: {}", formula_names.len());
    assert!(p.cells > 300 && stub_lines > 300, "{p:?}");

    // a second run changes nothing
    let again = common::bl(|| publish_all(&*store, &key, &dir, &lines)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((again.functions, again.versions), (0, 0), "{again:?}");

    // one stored row for each test line, each a pass
    let tests: Vec<Value> = views().iter().map(|l| json(l)).filter(|v| v.get("test").is_some()).collect();
    let mut total = 0;
    for f in &formula_names {
        let hash =
            common::bl(|| store.resolve_binding(f, END).unwrap().unwrap_or_else(|| panic!("{f} unbound")).fn_hash);
        let want = tests.iter().filter(|t| t["test"] == f.as_str()).count();
        assert!(want >= 4, "{f} has {want} tests");
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
    // The relay cells carry the SEC's licence, an https URL, which licence inheritance (SPEC 8.5) ranks as restricted: a
    // view over them is `unlicensed` unless the steward declares it for that licence (9.2). The stub used to say
    // CC0-1.0, which hid this; the real relay does not, so every view that reads a cell is declared here, as
    // mesh-test's key document must declare them (reported in D38's follow-up).
    for f in &formula_names {
        if f.starts_with(&format!("{HOST}/sec/")) {
            node.declare(f, SEC_LICENCE, "2026-01-01T00:00:00.000000Z");
        }
    }
    // the examples of the catalogue lines
    let mut examples = 0;
    for l in views() {
        let v = json(&l);
        if v.get("catalog").is_none() {
            continue;
        }
        for e in v["examples"].as_array().unwrap() {
            expect_value(&node, e["call"].as_str().unwrap(), e["expected"]["value"].as_str().unwrap()).await;
            examples += 1;
        }
    }
    assert!(examples >= formula_names.len(), "an example for each function: {examples}");

    let s = |f: &str| format!("{HOST}/sec/{f}");
    let jpm = "0000019617";
    let cat = "0000018230";
    // as reported against the restated figure (JPMorgan's 2019 revenue was restated twice)
    expect_value(
        &node,
        &format!(
            "{}(cik: \"{jpm}\", concept: \"Revenues\", unit: \"USD\", fiscal_year: 2019, period_kind: \"annual\")",
            s("reported")
        ),
        "115627000000",
    )
    .await;
    expect_value(
        &node,
        &format!(
            "{}(cik: \"{jpm}\", concept: \"Revenues\", unit: \"USD\", fiscal_year: 2019, period_kind: \"annual\")",
            s("latest")
        ),
        "115720000000",
    )
    .await;
    expect_value(
        &node,
        &format!(
            "{}(cik: \"{jpm}\", concept: \"Assets\", unit: \"USD\", fiscal_year: 2019, period_kind: \"instant\")",
            s("latest")
        ),
        "2686477000000",
    )
    .await;
    // a tag chain falling back through a real read that has no value (Caterpillar tags no NetIncomeLoss)
    expect_value(&node, &format!("{}(cik: \"{cat}\", fiscal_year: 2023)", s("net_income")), "10332000000").await;
    // a bank has no inventory: no value, and its fault is a fault of the cell
    expect_not_covered(&node, &format!("{}(cik: \"{jpm}\", fiscal_year: 2023)", s("inventory")), None).await;
    // the first fiscal year in scope has no opening balance
    expect_not_covered(&node, &format!("{HOST}/sec/ratio/return_on_assets(cik: \"{cat}\", fiscal_year: 2019)"), None)
        .await;
    // a zero divisor is a fault, not a guess
    expect_not_covered(&node, &format!("{HOST}/ratio/margin(part: 1, whole: 0)"), Some("E_FORMULA_DIV_ZERO")).await;
    expect_value(
        &node,
        &format!("{HOST}/sec/ratio/current_ratio(cik: \"{cat}\", fiscal_year: 2023)"),
        "1.351906242801197880672656070029947",
    )
    .await;
}

/// The rule is on: the same file is refused without its tests, and without any one derived case.
#[test]
fn the_public_rule_refuses_the_views_without_their_tests() {
    let mut all = stub();
    all.extend(views());
    // no test lines at all: the first formula without tests
    let no_tests: Vec<String> = all.iter().filter(|l| json(l).get("test").is_none()).cloned().collect();
    let lite = Lite::new("sv-none");
    let f = write(&lite, "all.jsonl", &no_tests);
    let e = publish_stored_checked(&*lite.store, &lite.key, HOST, &[HOST.to_string()], &f, &opts(NOW))
        .expect_err("refused");
    assert!(matches!(e.kind, PublishErrorKind::TestsMissing(_)), "{e}");
    assert!(e.detail.contains("E_TESTS_MISSING") && e.detail.contains("sec/fact_in"), "{e}");
    // one test removed from each function in turn: the zero divisor of margin
    let margin = format!("{HOST}/ratio/margin");
    let drop_zero: Vec<String> = all
        .iter()
        .filter(|l| {
            let v = json(l);
            !(v.get("test").is_some_and(|t| t == margin.as_str()) && v["expect"].get("fault").is_some())
        })
        .cloned()
        .collect();
    let lite = Lite::new("sv-nozero");
    let f = write(&lite, "all.jsonl", &drop_zero);
    let e = publish_stored_checked(&*lite.store, &lite.key, HOST, &[HOST.to_string()], &f, &opts(NOW))
        .expect_err("refused");
    assert!(matches!(e.kind, PublishErrorKind::TestsMissing(_)) && e.detail.contains("zero(whole)"), "{e}");
}
