//! http 0.4.9, CONTRACT-0.4.9 H5: bounded reads on public paths. The store reads of `/slice`, `/x/interval`
//! and `/versions` carry a SQL limit (`slice_max + 1`, `slice_max + 1`, 1,001), and the answers are what
//! 0.4.8 gave:
//!
//! * `/slice` of a function with more cells than `DATUMESH_SLICE_MAX`: the first `slice_max` members in key
//!   order, in every form, and no `next` past them (pinned from 0.4.8: `total = min(members, slice_max)`);
//! * `/versions/<cell>`: at most 1,000 versions, ascending by tx, and a `next` link for the rest;
//! * `/x/interval` of a YEAR-granularity series: at most 50 years, `400` `E_RANGE_TOO_LARGE` beyond (a month
//!   series keeps its 10).
//!
//! The first and second pass on 0.4.8 (the slice and the versions); the interval cap is new and FAILS there.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use common::lite::*;
use common::pub031::{function_line, run, write};
use common::readpath::*;
use datumesh_http::Config;
use publish4::get;
use serde_json::{json, Value};

fn config(vars: &[(&str, &str)]) -> Config {
    let mut env: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    env.push(("DATUMESH_HOST".into(), HOST.into()));
    for k in ["DATUMESH_RATE_PER_IP", "DATUMESH_RATE_IP_BURST", "DATUMESH_RATE_SERIES_PER_IP"] {
        env.push((k.into(), "1000000".into()));
    }
    Config::from_vars(move |k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).unwrap()
}

const ITEMS: &str = "mesh-test.datumesh.com/h049/bounded/items";

fn key_sig() -> String {
    r#"{"args":{"k":{"type":"string","valid":false}},"order":["k"],"returns":{"type":"number","unit":"1"}}"#.to_string()
}

fn item(k: &str, v: &str) -> String {
    json!({"cell": {"fn": ITEMS, "args": {"k": {"type": "string", "value": k}}}, "licence": "CC0-1.0",
        "summary": "Synthetic test data.", "value": {"type": "number", "value": v}})
    .to_string()
}

fn member_key(m: &Value) -> String {
    m["receipt"]["cell"]["args"]["k"]["value"].as_str().unwrap_or_else(|| panic!("{m}")).to_string()
}

async fn items(label: &str, n: usize, max: &str) -> Served {
    let lite = Lite::new(label);
    let mut lines = vec![function_line(ITEMS, &key_sig(), None)];
    for i in 1..=n {
        lines.push(item(&format!("k{i:02}"), &i.to_string()));
    }
    let f = write(&lite, "f.jsonl", &lines);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    build_served(lite, Some(config(&[("DATUMESH_SLICE_MAX", max)])), |_| {}).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slice_larger_than_the_cap_is_cut_at_the_cap() {
    let m = items("h5-slice", 12, "5").await;
    // one page: the first five, no `next`
    let r = get(&m, &format!("/slice/{ITEMS}?k=*"), &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    let keys: Vec<String> = j["members"].as_array().unwrap().iter().map(member_key).collect();
    assert_eq!(keys, ["k01", "k02", "k03", "k04", "k05"], "{j}");
    assert!(j.get("next").is_none_or(Value::is_null), "{j}");
    // pages of two: 2 + 2 + 1, and the last has no `next`
    let mut seen = Vec::new();
    let mut url = format!("/slice/{ITEMS}?k=*&limit=2");
    for _ in 0..10 {
        let j = get(&m, &url, &[]).await.json();
        seen.extend(j["members"].as_array().unwrap().iter().map(member_key));
        match j.get("next").and_then(Value::as_str) {
            Some(n) => url = n.replacen(&format!("https://{HOST}"), "", 1),
            None => break,
        }
    }
    assert_eq!(seen, ["k01", "k02", "k03", "k04", "k05"]);
    // CSV: the header and five rows
    let csv = get(&m, &format!("/slice/{ITEMS}?k=*&format=csv"), &[]).await;
    assert_eq!(csv.status, 200, "{}", csv.text());
    assert_eq!(csv.text().lines().count(), 6, "{}", csv.text());
    // a slice within the cap is whole
    let m2 = items("h5-slice-small", 4, "5").await;
    let j = get(&m2, &format!("/slice/{ITEMS}?k=*"), &[]).await.json();
    assert_eq!(j["members"].as_array().unwrap().len(), 4, "{j}");
}

const CELLV: &str = "mesh-test.datumesh.com/h049/bounded/versioned";

fn version(i: usize) -> String {
    let tx = format!("2026-03-01T12:00:00.{i:06}Z");
    json!({"cell": {"fn": CELLV, "args": {"k": {"type": "string", "value": "N"}}}, "licence": "CC0-1.0",
        "summary": "Synthetic test data.", "value": {"type": "number", "value": i.to_string()},
        "tx": tx, "asof": tx, "issued": tx})
    .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn versions_are_capped_at_a_thousand_with_a_link_to_the_rest() {
    let lite = Lite::new("h5-versions");
    let f0 = write(&lite, "functions.jsonl", &[function_line(CELLV, &key_sig(), None)]);
    run(&lite, &f0, "2026-01-01T00:00:00.000000Z").unwrap_or_else(|e| panic!("{e}"));
    let lines: Vec<String> = (1..=1003).map(version).collect();
    let f = write(&lite, "f.jsonl", &lines);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    let m = build_served(lite, Some(config(&[])), |_| {}).await;
    let r = get(&m, &format!("/versions/{CELLV}?k=N"), &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    let vs = j["versions"].as_array().unwrap();
    assert_eq!(vs.len(), 1000, "{}", vs.len());
    let txs: Vec<&str> = vs.iter().map(|v| v["tx"].as_str().unwrap()).collect();
    assert!(txs.windows(2).all(|w| w[0] < w[1]), "ascending by tx");
    assert_eq!(txs[0], "2026-03-01T12:00:00.000001Z");
    let next = j["next"].as_str().unwrap_or_else(|| panic!("a link to the rest: {j}")).replacen(
        &format!("https://{HOST}"),
        "",
        1,
    );
    let rest = get(&m, &next, &[]).await.json();
    let rest_vs = rest["versions"].as_array().unwrap();
    assert_eq!(rest_vs.len(), 3, "{rest}");
    assert_eq!(rest_vs[0]["tx"], "2026-03-01T12:00:00.001001Z", "{rest}");
    assert!(rest.get("next").is_none_or(Value::is_null), "{rest}");
}

// ---- /x/interval of a year series -----------------------------------------------------------------

const POLICY: &str = "mesh-test.datumesh.com/relay/ecb.europa.eu/policy_rate";

fn date_sig() -> String {
    r#"{"args":{"date":{"type":"date","valid":true}},"order":["date"],"returns":{"type":"number","unit":"1"}}"#
        .to_string()
}

fn dated(date: &str, value: &str) -> String {
    let tx = "2026-03-01T12:00:00.000000Z";
    json!({"cell": {"fn": POLICY, "args": {"date": {"type": "date", "value": date}}}, "licence": "CC0-1.0",
        "summary": "Synthetic test data.", "value": {"type": "number", "value": value},
        "source": {"name": "ecb.europa.eu", "url": "https://ecb.europa.eu/data", "retrieved": "2026-01-01T00:00:00.000000Z",
            "takeover": format!("https://{HOST}/takeover/ecb.europa.eu")},
        "tx": tx, "asof": tx, "issued": tx})
    .to_string()
}

async fn year_series(label: &str) -> Served {
    let lite = Lite::new(label);
    let f0 = write(&lite, "functions.jsonl", &[function_line(POLICY, &date_sig(), None)]);
    run(&lite, &f0, "2026-01-01T00:00:00.000000Z").unwrap_or_else(|e| panic!("{e}"));
    let mut v: Vec<String> = [("1990-06-01", "4"), ("2000-06-01", "5"), ("2020-06-01", "1"), ("2026-02-01", "2")]
        .iter()
        .map(|(d, x)| dated(d, x))
        .collect();
    v.push(catalog_line(POLICY, "function", "Policy rate", "The rate.", json!({"date": "the day"}), json!({})));
    let f = write(&lite, "fixture.jsonl", &v);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    let p = lite.path("series.json");
    std::fs::write(
        &p,
        json!([{"function": POLICY, "granularity": "year", "date_arg": "date", "unit": "percent", "kind": "relayed"}])
            .to_string(),
    )
    .unwrap();
    let path = p.to_string_lossy().into_owned();
    build_served(lite, Some(config(&[("DATUMESH_SERIES_FILE", &path)])), |_| {}).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_year_series_interval_is_capped_at_fifty_years() {
    let m = year_series("h5-interval").await;
    let q = |from: &str, to: &str| format!("/x/interval/{POLICY}?from={from}&to={to}");
    // fifty years is the most (as ten years are for months)
    let r = get(&m, &q("1976-01-01", "2026-01-01"), &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = get(&m, &q("1976-01-01", "2026-01-02"), &[]).await;
    assert_eq!(r.status, 400, "{}", r.text());
    assert!(r.text().contains("E_RANGE_TOO_LARGE"), "{}", r.text());
    for (from, to) in [("1900-01-01", "2026-12-31"), ("1970-01-01", "2026-12-31"), ("1975-12-31", "2026-01-01")] {
        let r = get(&m, &q(from, to), &[]).await;
        assert_eq!(r.status, 400, "{from}..{to}: {}", r.text());
        assert!(r.text().contains("E_RANGE_TOO_LARGE"), "{from}..{to}: {}", r.text());
    }
    // within the cap the members are the stored ones in range
    let r = get(&m, &q("1990-01-01", "2026-12-31"), &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    let n = j.get("members").or_else(|| j.get("cells")).and_then(Value::as_array).map(Vec::len);
    assert_eq!(n, Some(4), "{j}");
    // the cap applies to the other forms too
    for f in ["csv", "arrow"] {
        let r = get(&m, &format!("{}&format={f}", q("1900-01-01", "2026-12-31")), &[]).await;
        assert_eq!(r.status, 400, "{f}: {}", r.text());
    }
}
