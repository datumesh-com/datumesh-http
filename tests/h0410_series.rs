//! http 0.4.10, CONTRACT-round-0.7-P HP7 and HP8: the series paths do bounded work.
//! Written first; FAIL until then.
//!
//! * HP8 (A16): a series period page reads at most `slice_max + 1` members, as `/slice` does; the page shows
//!   at most `slice_max` of them. With `DATUMESH_SLICE_MAX=5` and 20 daily cells in the month, the CSV of the
//!   page has at most 5 data rows (a page the mesh refuses outright is also within the contract: the test
//!   pins that nothing past the cap is shown, and, with the default cap, that the page is whole).
//! * HP7 (A15): the overview orders the dictionary values of a column argument with
//!   `dictionary_values_ordered` (one read of `key_values`) and the names come from `catalog_entries_all`, not
//!   one `text_find` of the whole catalogue per value (0.4.9: 12 scans of 21 rows and more). Counted with the
//!   probe of `common/h0410.rs`.
//!
//! Not testable from outside: that the overview folds a series in pages of `slice_values_page` rather than one
//! vector (the difference is memory, and the store is sealed so a wrapper cannot count the calls); the build
//! of a series of 10,000 members gives the same answer either way. The report says so.
//! Also not testable at the sizes a test can hold: HP8's second half (a formula's slice read stops at the
//! remaining fuel plus one): the fuel is a constant of 10,000,000, so an over-budget single read needs more than
//! ten million cells, and a formula that burns the fuel by repeated reads gives `E_EVAL_BUDGET` in 0.4.9 as
//! well. A configurable fuel (a `Config` field, or the engine's budget argument) would make it a small test.

mod common;
#[path = "common/h0410.rs"]
mod h0410;

use common::lite::*;
use common::pub031::function_line;
use common::readpath::catalog_line;
use h0410::*;
use serde_json::{json, Value};

const POLICY: &str = "mesh-test.datumesh.com/relay/ecb.europa.eu/policy_rate";
const FXR: &str = "mesh-test.datumesh.com/relay/ecb.europa.eu/fx_reference";
const CLOCK: &str = "2026-01-01T00:00:00.000000Z";

fn date_sig() -> String {
    r#"{"args":{"date":{"type":"date","valid":true}},"order":["date"],"returns":{"type":"number","unit":"1"}}"#
        .to_string()
}

fn fx_sig() -> String {
    r#"{"args":{"currency":{"type":"string","valid":false},"date":{"type":"date","valid":true}},"order":["currency","date"],"returns":{"type":"number","unit":"1"}}"#.to_string()
}

fn source() -> Value {
    json!({"name": "ecb.europa.eu", "url": "https://ecb.europa.eu/data", "retrieved": CLOCK,
        "takeover": format!("https://{HOST}/takeover/ecb.europa.eu")})
}

fn daily(f: &str, args: Value, value: &str, day: &str) -> String {
    let tx = format!("{day}T12:00:00.000000Z");
    json!({"cell": {"fn": f, "args": args}, "licence": "CC0-1.0", "summary": "Synthetic test data.",
        "value": {"type": "number", "value": value}, "source": source(), "tx": tx, "asof": tx, "issued": tx})
    .to_string()
}

fn series_file(fx: &Fx, series: Value) -> String {
    let p = fx.dir.join("series.json");
    std::fs::write(&p, series.to_string()).unwrap();
    p.to_string_lossy().into_owned()
}

fn date(v: &str) -> Value {
    json!({"type": "date", "value": v})
}

// ---- HP8: the period page ---------------------------------------------------------------------------------

fn data_rows(csv: &str) -> usize {
    csv.lines().filter(|l| !l.trim().is_empty()).count().saturating_sub(1)
}

async fn month_page(w: &W, path: &str) -> Rep {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let r = w.get(path).await;
        if r.status != 503 || std::time::Instant::now() > end {
            return r;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn policy_fx(label: &str) -> (Fx, String) {
    let fx = Fx::lite(label);
    fx.publish(&[function_line(POLICY, &date_sig(), None)], CLOCK);
    let mut v: Vec<String> = (1..=20)
        .map(|d| daily(POLICY, json!({"date": date(&format!("2026-03-{d:02}"))}), "4", &format!("2026-03-{d:02}")))
        .collect();
    v.push(catalog_line(POLICY, "function", "Policy rate", "The rate.", json!({"date": "the day"}), json!({})));
    fx.publish(&v, NOW);
    let path = series_file(
        &fx,
        json!([{"function": POLICY, "granularity": "month", "date_arg": "date", "unit": "percent", "kind": "relayed"}]),
    );
    (fx, path)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_period_page_shows_at_most_the_slice_cap() {
    // the control: the default cap holds the whole month, 20 rows
    let (fx, path) = policy_fx("hp8-page-control");
    let w = fx.serve(config(&[("DATUMESH_SERIES_FILE", &path)], &[])).await;
    let r = month_page(&w, &format!("/series/{POLICY}/2026-03.csv")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(data_rows(&r.text()), 20, "{}", r.text());
    // the cap of 5
    let (fx, path) = policy_fx("hp8-page-cap");
    let w = fx.serve(config(&[("DATUMESH_SERIES_FILE", &path), ("DATUMESH_SLICE_MAX", "5")], &[])).await;
    let r = month_page(&w, &format!("/series/{POLICY}/2026-03.csv")).await;
    if r.status == 200 {
        assert!(
            data_rows(&r.text()) <= 5,
            "the page shows {} of the month's 20 days: {}",
            data_rows(&r.text()),
            r.text()
        );
    } else {
        assert!(matches!(r.status, 404 | 413 | 422), "a refused page: {} {}", r.status, r.text());
    }
    // the HTML page as well
    let h = w.get_h(&format!("/series/{POLICY}/2026-03"), &[("accept", "text/html")]).await;
    if h.status == 200 {
        let rows = h.text().matches("<tr").count();
        assert!(rows <= 5 + 2, "the HTML page has {rows} table rows for a cap of 5");
    }
}

// ---- HP7: the overview's dictionary order -----------------------------------------------------------------

const CURRENCIES: [&str; 12] = ["USD", "JPY", "GBP", "CHF", "AUD", "CAD", "SEK", "NOK", "DKK", "PLN", "CZK", "HUF"];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_overview_orders_dictionary_values_without_a_scan_of_the_catalogue_per_value() {
    let fx = Fx::pg("hp7-order");
    // 20 other entries, so that a scan of the catalogue is 21 rows, and the keyed series with 12 currencies
    let mut early = vec![function_line(FXR, &fx_sig(), None)];
    for i in 0..20 {
        early.push(function_line(&format!("mesh-test.datumesh.com/h0410/ser/f{i:02}"), &date_sig(), None));
    }
    fx.publish(&early, CLOCK);
    let mut v = Vec::new();
    for (d, cur, val) in [("2026-03-02", "USD", "1.08"), ("2026-03-02", "JPY", "160.2"), ("2026-03-03", "USD", "1.07")]
    {
        v.push(daily(FXR, json!({"currency": {"type": "string", "value": cur}, "date": date(d)}), val, d));
    }
    let dict: Vec<Value> =
        CURRENCIES.iter().map(|c| json!({"value": c, "label": format!("The {c} currency")})).collect();
    v.push(catalog_line(
        FXR,
        "function",
        "Euro reference rate",
        "The euro foreign exchange reference rates.",
        json!({"currency": "the currency", "date": "the day"}),
        json!({"currency": dict}),
    ));
    for i in 0..20 {
        v.push(catalog_line(
            &format!("mesh-test.datumesh.com/h0410/ser/f{i:02}"),
            "function",
            &format!("Series number {i}"),
            "Another function of the catalogue.",
            json!({"date": "the day"}),
            json!({}),
        ));
    }
    fx.publish(&v, NOW);
    let path = series_file(
        &fx,
        json!([{"function": FXR, "granularity": "month", "date_arg": "date", "column_arg": "currency",
            "unit": "units per EUR", "kind": "relayed"}]),
    );
    fx.probe().arm();
    let mut vars = held(&["series"]);
    vars.push(("DATUMESH_SERIES_FILE", &path));
    let w = fx.serve(config(&vars, &[])).await;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(90);
    let r = loop {
        let r = w.get(&format!("/series/{FXR}/")).await;
        if r.status != 503 || std::time::Instant::now() > end {
            break r;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    assert_eq!(r.status, 200, "{}", r.text());
    // the columns are all twelve, in dictionary order
    let page = month_page(&w, &format!("/series/{FXR}/2026-03.csv")).await;
    assert_eq!(page.status, 200, "{}", page.text());
    let header = page.text().lines().next().unwrap().to_string();
    let cols: Vec<&str> = header.split(',').skip(1).collect();
    assert_eq!(cols.len(), 12, "{header}");
    assert!(cols[0].starts_with("USD") && cols[1].starts_with("JPY") && cols[11].starts_with("HUF"), "{header}");
    let read = w.fx.probe().reads(Group::Catalog);
    assert!(
        read <= (5 * 21) as u64,
        "the overview build visited {read} catalogue rows: a scan of the 21 for each of the 12 values is 252 and more"
    );
}
