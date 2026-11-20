//! http 0.4.6 (QUESTIONS-build2 D23 again, seen live on datumesh.com with 0.4.4): two stored functions
//! with one signature share a function hash, and every count or listing that is per function NAME must
//! count the cells of that name, not of the hash. These tests were written first and FAIL until then.
//!
//! * `GET /find?summary=1`: `cells` of each of the two is its own count (the live page showed
//!   `Bank of England Bank Rate (14,945 cells)` and `Euro short-term rate (14,945 cells)`);
//! * the function page `/fn/<name>`, as JSON and HTML, is the page of its own function;
//! * the series index, month pages and CSV list only the cells of their own function.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use std::collections::BTreeSet;

use common::lite::*;
use common::pub031::{function_line, run, write};
use common::readpath::*;
use datumesh_http::Config;
use serde_json::{json, Value};

const BANK: &str = "mesh-test.datumesh.com/relay/bankofengland.co.uk/bank_rate";
const ESTR: &str = "mesh-test.datumesh.com/relay/ecb.europa.eu/estr";
const BOUND_CLOCK: &str = "2026-01-01T00:00:00.000000Z";

fn sig() -> String {
    r#"{"args":{"date":{"type":"date","valid":true}},"order":["date"],"returns":{"type":"number","unit":"1"}}"#
        .to_string()
}

fn cell(f: &str, domain: &str, date: &str, value: &str) -> String {
    let tx = format!("{date}T12:00:00.000000Z");
    json!({"cell": {"fn": f, "args": {"date": {"type": "date", "value": date}}}, "licence": "CC0-1.0",
        "summary": "Synthetic test data.", "value": {"type": "number", "value": value},
        "source": {"name": domain, "url": format!("https://{domain}/data"), "retrieved": "2026-01-01T00:00:00.000000Z"},
        "tx": tx, "asof": tx, "issued": tx})
    .to_string()
}

/// BANK has four cells (March and May), ESTR one (July): one descriptor, two names.
fn lines() -> Vec<String> {
    let mut v = Vec::new();
    for (d, x) in [("2026-03-19", "4.5"), ("2026-03-20", "4.5"), ("2026-05-07", "4.25"), ("2026-05-08", "4.25")] {
        v.push(cell(BANK, "bankofengland.co.uk", d, x));
    }
    v.push(cell(ESTR, "ecb.europa.eu", "2026-07-01", "2.9"));
    v.push(catalog_line(BANK, "function", "Bank Rate", "The bank rate.", json!({"date": "the day"}), json!({})));
    v.push(catalog_line(ESTR, "function", "Euro short-term rate", "The estr.", json!({"date": "the day"}), json!({})));
    v
}

async fn serve(label: &str) -> Served {
    let lite = Lite::new(label);
    let f0 = write(&lite, "functions.jsonl", &[function_line(BANK, &sig(), None), function_line(ESTR, &sig(), None)]);
    run(&lite, &f0, BOUND_CLOCK).unwrap_or_else(|e| panic!("functions refused: {e}"));
    let f = write(&lite, "fixture.jsonl", &lines());
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("fixture refused: {e}"));
    let series = json!([
        {"function": BANK, "granularity": "month", "date_arg": "date", "unit": "percent", "kind": "relayed"},
        {"function": ESTR, "granularity": "month", "date_arg": "date", "unit": "percent", "kind": "relayed"},
    ]);
    let p = lite.path("series.json");
    std::fs::write(&p, series.to_string()).unwrap();
    let path = p.to_string_lossy().into_owned();
    let cfg = Config::from_vars(move |k| match k {
        "DATUMESH_HOST" => Some(HOST.to_string()),
        "DATUMESH_SERIES_FILE" => Some(path.clone()),
        _ => None,
    })
    .unwrap();
    build_served(lite, Some(cfg), |_| {}).await
}

async fn text(m: &Served, path: &str, accept: &str) -> (u16, String) {
    let r = publish4::get(m, path, &[("accept", accept)]).await;
    (r.status, r.text())
}

fn hrefs(h: &str, prefix: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = h;
    while let Some(i) = rest.find("href=\"") {
        rest = &rest[i + 6..];
        let end = rest.find('"').unwrap();
        let link = &rest[..end];
        let path = link.strip_prefix(&format!("https://{HOST}")).unwrap_or(link);
        if path.starts_with(prefix) && !path.ends_with(".csv") && path != prefix {
            out.insert(path.to_string());
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_summary_counts_the_cells_of_each_name() {
    let m = serve("tc-summary").await;
    let v = m.get("/find?summary=1").await;
    let f = v["functions"].as_array().unwrap();
    let cells = |n: &str| f.iter().find(|e| e["name"] == n).unwrap_or_else(|| panic!("no {n} in {v}"))["cells"].clone();
    assert_eq!(cells(BANK), json!(4), "{v}");
    assert_eq!(cells(ESTR), json!(1), "{v}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_function_pages_are_each_their_own() {
    let m = serve("tc-fn").await;
    for (name, label) in [(BANK, "Bank Rate"), (ESTR, "Euro short-term rate")] {
        let (st, body) = text(&m, &format!("/fn/{name}"), "application/json").await;
        assert_eq!(st, 200, "{body}");
        let page: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(page["name"], name, "{page}");
        assert!(body.contains(label), "{body}");
        let other = if name == BANK { "Euro short-term rate" } else { "Bank Rate" };
        assert!(!body.contains(other), "{name}: {body}");
        assert_eq!(page["tests"]["count"], 0, "{page}");
    }
    // a function page, as HTML, shows no count of the other function's cells
    let (_, h) = text(&m, &format!("/fn/{ESTR}"), "text/html").await;
    assert!(!h.contains("4 cells") && !h.contains(">4<"), "{h}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_series_index_and_pages_list_only_their_own_cells() {
    let m = serve("tc-series").await;
    let own = |f: &str, ps: &[&str]| ps.iter().map(|p| format!("/series/{f}/{p}")).collect::<BTreeSet<_>>();
    let (st, h) = text(&m, &format!("/series/{BANK}/"), "text/html").await;
    assert_eq!(st, 200, "{h}");
    assert_eq!(hrefs(&h, &format!("/series/{BANK}/")), own(BANK, &["2026-03", "2026-05"]));
    let (st, h) = text(&m, &format!("/series/{ESTR}/"), "text/html").await;
    assert_eq!(st, 200, "{h}");
    assert_eq!(hrefs(&h, &format!("/series/{ESTR}/")), own(ESTR, &["2026-07"]));
    let csv = |f: &str, p: &str| format!("/series/{f}/{p}.csv");
    assert_eq!(
        text(&m, &csv(BANK, "2026-03"), "*/*").await.1,
        "date,value (percent)\r\n2026-03-19,4.5\r\n2026-03-20,4.5\r\n"
    );
    assert_eq!(text(&m, &csv(ESTR, "2026-07"), "*/*").await.1, "date,value (percent)\r\n2026-07-01,2.9\r\n");
    assert_eq!(text(&m, &csv(ESTR, "2026-03"), "*/*").await.0, 404);
    assert_eq!(text(&m, &csv(BANK, "2026-07"), "*/*").await.0, 404);
    let (_, sm) = text(&m, "/series/sitemap.xml", "*/*").await;
    for p in ["2026-03", "2026-05"] {
        assert!(sm.contains(&format!("/series/{BANK}/{p}")), "{sm}");
        assert!(!sm.contains(&format!("/series/{ESTR}/{p}")), "{sm}");
    }
    assert!(
        sm.contains(&format!("/series/{ESTR}/2026-07")) && !sm.contains(&format!("/series/{BANK}/2026-07")),
        "{sm}"
    );
}
