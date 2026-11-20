//! Build 2 Part 6, G6 (CONTRACT-build2-part6-pages, QUESTIONS-build2 D25 and D26): month pages for
//! relayed series, http 0.4.5. These tests are written first and FAIL until the pages exist.
//!
//! # The series file (G2), defined here
//!
//! `DATUMESH_SERIES_FILE` names a JSON file: an array with one object per series.
//!
//! ```json
//! [{"function": "<full function name>",
//!   "granularity": "month" | "year",   // month pages (a daily series) or year pages (a monthly or event series)
//!   "date_arg": "<name of the date argument>",
//!   "column_arg": "<name of the argument that becomes columns>",   // optional: none = one value column
//!   "fixed": {"<arg>": "<value>"},                                  // optional: arguments held fixed
//!   "unit": "<unit text>",
//!   "kind": "relayed" | "computed"}]
//! ```
//!
//! The columns of a keyed series are the dictionary values of `column_arg` in dictionary order (the
//! catalogue entry's `dictionary`), every one of them on every page. Without the file there are no
//! pages. A function that is not listed has no pages.
//!
//! # Choices these tests fix where the contract is open
//!
//! * Header cells: `date`, then `<value> (<unit>)` per column (the dictionary VALUE, not its label);
//!   an unkeyed series has one column headed `value (<unit>)`.
//! * CSV: RFC 4180, CRLF after every record including the last, fields quoted only when they hold a
//!   comma, quote or line break, no byte order mark, `text/csv` (a charset of utf-8 is allowed).
//! * Previous and next: `<a rel="prev" href=...>` and `<a rel="next" href=...>`; an absent neighbour
//!   has no such anchor at all.
//! * Hrefs may be absolute (`https://<host>/...`) or root-relative; `Location` likewise.
//! * One `<script>` is allowed on a page: the JSON-LD (`type="application/ld+json"`), whose text holds
//!   no `<`.
//! * Edge lifetime of a 404 is 600 s: `public` and an `s-maxage` (or else `max-age`) of 600.
//! * Mesh "now" in these tests is 2026-11-16, so 2026-11 and 2026 are the current month and year.
//! * The `/slice` form takes `*` and values, no ranges: a slice whose date is one value lies in one
//!   month (year), a slice whose date is `*` goes to the index.
//!
//! Not tested here: the 50,000-URL split of the sitemap (it needs 50,000 cells) and the production edge (G7).

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use calculus::*;
use common::lite::*;
use common::pub031::{function_line, run, snapshot, write, T2};
use common::readpath::*;
use datumesh_http::Config;
use publish4::Reply4;
use serde_json::{json, Value};

// ---- the fixture ---------------------------------------------------------------------------------

const FXR: &str = "mesh-test.datumesh.com/relay/ecb.europa.eu/fx_reference";
const BANK: &str = "mesh-test.datumesh.com/relay/bankofengland.co.uk/bank_rate";
const POLICY: &str = "mesh-test.datumesh.com/relay/ecb.europa.eu/policy_rate";
const RAIN: &str = "mesh-test.datumesh.com/earth/monthly_rainfall";
const DERIVED: &str = "mesh-test.datumesh.com/earth/rain_doubled";
const VERSE: &str = "mesh-test.datumesh.com/scratch_p6/lexicon/verse";
const PLAIN: &str = "mesh-test.datumesh.com/scratch_p6/plain/level";
const FOREIGN: &str = "foreign.example/relay/x.org/rate";

const BOUND_CLOCK: &str = "2026-01-01T00:00:00.000000Z";
const ECB_LICENCE: &str = "https://www.ecb.europa.eu/services/using-our-site/disclaimer/html/index.en.html";
const FX_LABEL: &str = "Euro reference rate";
const BANK_LABEL: &str = r#"Bank <b>rate</b> & "co" </script><script>alert(1)</script>"#;
const BANK_DESC: &str = "Rate <img src=x onerror=alert(1)> set by the bank";

fn origin() -> String {
    format!("https://{HOST}")
}

fn fx_sig() -> String {
    r#"{"args":{"currency":{"type":"string","valid":false},"date":{"type":"date","valid":true}},"order":["currency","date"],"returns":{"type":"number","unit":"1"}}"#.to_string()
}
fn bank_sig() -> String {
    r#"{"args":{"date":{"type":"date","valid":true}},"order":["date"],"returns":{"type":"number","unit":"1"}}"#
        .to_string()
}
fn policy_sig() -> String {
    r#"{"args":{"date":{"type":"date","valid":true},"rate":{"type":"string","valid":false}},"order":["rate","date"],"returns":{"type":"number","unit":"1"}}"#.to_string()
}
fn rain_sig() -> String {
    r#"{"args":{"dataset":{"type":"string","valid":false},"month":{"type":"date","valid":true},"region":{"type":"string","valid":false}},"order":["dataset","region","month"],"returns":{"type":"number","unit":"1"}}"#.to_string()
}

fn source(domain: &str) -> Value {
    json!({"name": domain, "url": format!("https://{domain}/data"), "retrieved": "2026-01-01T00:00:00.000000Z",
        "takeover": format!("https://{HOST}/takeover/{domain}")})
}

fn stored(f: &str, args: Value, value: &str, tx: &str, licence: &str, src: Option<Value>) -> String {
    let mut v = json!({"cell": {"fn": f, "args": args}, "licence": licence, "summary": "Synthetic test data.",
        "value": {"type": "number", "value": value}, "tx": tx, "asof": tx, "issued": tx});
    if let Some(s) = src {
        v["source"] = s;
    }
    v.to_string()
}

fn s(v: &str) -> Value {
    json!({"type": "string", "value": v})
}
fn d(v: &str) -> Value {
    json!({"type": "date", "value": v})
}

fn fx_cell(cur: &str, date: &str, value: &str) -> String {
    stored(
        FXR,
        json!({"currency": s(cur), "date": d(date)}),
        value,
        &format!("{date}T15:00:00.000000Z"),
        ECB_LICENCE,
        Some(source("ecb.europa.eu")),
    )
}
fn bank_cell(date: &str, value: &str) -> String {
    stored(
        BANK,
        json!({"date": d(date)}),
        value,
        &format!("{date}T12:00:00.000000Z"),
        "OGL-UK-3.0",
        Some(source("bankofengland.co.uk")),
    )
}
/// Event dates are old; the cells were signed (`tx`) in 2026 (a mesh signs from its key's start).
fn policy_cell(rate: &str, date: &str, value: &str) -> String {
    let tx = match date {
        "2022-07-27" => "2026-02-01T09:00:00.000000Z",
        "2022-09-14" => "2026-02-02T09:00:00.000000Z",
        "2023-06-21" => "2026-02-03T09:00:00.000000Z",
        _ => "2026-06-11T14:00:00.000000Z",
    };
    stored(POLICY, json!({"rate": s(rate), "date": d(date)}), value, tx, ECB_LICENCE, Some(source("ecb.europa.eu")))
}
fn rain_cell(region: &str, month: &str, value: &str, tx: &str) -> String {
    stored(RAIN, json!({"dataset": s("chirps"), "region": s(region), "month": d(month)}), value, tx, "CC-BY-4.0", None)
}

fn dict(values: &[(&str, &str)]) -> Vec<Value> {
    values.iter().map(|(v, l)| json!({"value": v, "label": l})).collect()
}

/// The four stored series functions, published first, with the mesh clock at the start of the test key (a binding
/// cannot start before the clock of its run, rule A): their cells are backdated in the second file.
fn early_lines() -> Vec<String> {
    vec![
        function_line(FXR, &fx_sig(), None),
        function_line(BANK, &bank_sig(), None),
        function_line(POLICY, &policy_sig(), None),
        function_line(RAIN, &rain_sig(), None),
    ]
}

fn lines() -> Vec<String> {
    let mut v = Vec::new();
    // the keyed daily relay: January (two days, a gap in GBP), March (two days), November (the current month);
    // February and April to October have no cell
    for (date, cells) in [
        ("2026-01-05", vec![("USD", "1.0412"), ("JPY", "163.45"), ("GBP", "0.8567")]),
        ("2026-01-06", vec![("USD", "1.045"), ("JPY", "164.1")]),
        ("2026-03-02", vec![("USD", "1.0832"), ("JPY", "160.25"), ("GBP", "0.8301")]),
        ("2026-03-03", vec![("USD", "1.0791"), ("JPY", "150"), ("GBP", "0.829")]),
        ("2026-11-13", vec![("USD", "1.0892")]),
    ] {
        for (cur, val) in cells {
            v.push(fx_cell(cur, date, val));
        }
    }
    // the unkeyed daily relay
    for (date, val) in [("2026-03-19", "4.5"), ("2026-03-20", "4.5"), ("2026-05-07", "4.25")] {
        v.push(bank_cell(date, val));
    }
    // the yearly event series
    for (date, cells) in [
        ("2022-07-27", vec![("mro", "0.5"), ("dfr", "0"), ("mlfr", "0.75")]),
        ("2022-09-14", vec![("mro", "1.25"), ("dfr", "0.75"), ("mlfr", "1.5")]),
        ("2023-06-21", vec![("mro", "4"), ("dfr", "3.5")]),
        ("2026-06-11", vec![("dfr", "2")]),
    ] {
        for (rate, val) in cells {
            v.push(policy_cell(rate, date, val));
        }
    }
    // the computed-style monthly series: stored, no source
    let (t24, t25) = ("2026-02-10T08:00:00.000000Z", "2026-02-11T08:00:00.000000Z");
    v.push(rain_cell("us_kansas", "2024-11-01", "12.5", t24));
    v.push(rain_cell("ua_kharkiv", "2024-11-01", "40", t24));
    v.push(rain_cell("us_kansas", "2024-12-01", "1234.5", t24));
    v.push(rain_cell("us_kansas", "2025-01-01", "0.25", t25));
    v.push(rain_cell("ua_kharkiv", "2025-01-01", "33.75", t25));
    // a computed function that reads the stored one: it has NO stored cell, so a page of it must not exist
    v.push(formula_line(
        DERIVED,
        &format!("{RAIN}(dataset: dataset, region: region, month: month) * 2"),
        &rain_sig(),
        None,
    ));
    // listed in no series file: a scripture-shaped function and a plain one
    v.push(function_line(VERSE, &reading_sig(), None));
    v.push(site_cell(VERSE, "N", "3"));
    v.push(function_line(PLAIN, &reading_sig(), None));
    v.push(site_cell(PLAIN, "N", "7"));
    // the catalogue
    let usd = [("USD", "US dollar"), ("JPY", "Japanese yen"), ("GBP", "Pound sterling")];
    v.push(catalog_line(
        FXR,
        "function",
        FX_LABEL,
        "The euro foreign exchange reference rates, relayed from the ECB.",
        json!({"currency": "the currency", "date": "the day"}),
        json!({"currency": dict(&usd)}),
    ));
    v.push(catalog_line(BANK, "function", BANK_LABEL, BANK_DESC, json!({"date": "the day"}), json!({})));
    v.push(catalog_line(POLICY, "function", "ECB key interest rates", "The key rates of the ECB, on the days they change.",
        json!({"rate": "the rate", "date": "the day"}),
        json!({"rate": dict(&[("mro", "Main refinancing operations"), ("dfr", "Deposit facility"), ("mlfr", "Marginal lending facility")])})));
    v.push(catalog_line(
        RAIN,
        "function",
        "Monthly rainfall",
        "Rainfall of a region in a month, from CHIRPS v3.",
        json!({"dataset": "the dataset", "region": "the region", "month": "the first day of the month"}),
        json!({"region": dict(&[("us_kansas", "Kansas, United States"), ("ua_kharkiv", "Kharkiv Oblast, Ukraine")])}),
    ));
    v.push(catalog_line(
        DERIVED,
        "function",
        "Doubled rainfall",
        "Twice the rainfall, computed.",
        json!({}),
        json!({}),
    ));
    v
}

fn series_json() -> String {
    json!([
        {"function": FXR, "granularity": "month", "date_arg": "date", "column_arg": "currency", "unit": "units per EUR", "kind": "relayed"},
        {"function": BANK, "granularity": "month", "date_arg": "date", "unit": "percent", "kind": "relayed"},
        {"function": POLICY, "granularity": "year", "date_arg": "date", "column_arg": "rate", "unit": "percent", "kind": "relayed"},
        {"function": RAIN, "granularity": "year", "date_arg": "month", "column_arg": "region", "fixed": {"dataset": "chirps"},
         "unit": "millimetres, per month", "kind": "computed"},
        {"function": DERIVED, "granularity": "year", "date_arg": "month", "column_arg": "region", "fixed": {"dataset": "chirps"},
         "unit": "millimetres, per month", "kind": "computed"},
        {"function": FOREIGN, "granularity": "month", "date_arg": "date", "unit": "percent", "kind": "relayed"},
    ])
    .to_string()
}

/// Publishes the fixture and serves it; `series` is the content of `DATUMESH_SERIES_FILE` (none: not set);
/// `vars` are further environment variables.
async fn serve(label: &str, series: Option<String>, vars: &[(&str, &str)]) -> Served {
    let lite = Lite::new(label);
    let f0 = write(&lite, "functions.jsonl", &early_lines());
    run(&lite, &f0, BOUND_CLOCK).unwrap_or_else(|e| panic!("functions refused: {e}"));
    let f = write(&lite, "fixture.jsonl", &lines());
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("fixture refused: {e}"));
    let mut env: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    if let Some(text) = series {
        let p = lite.path("series.json");
        std::fs::write(&p, text).unwrap();
        env.push(("DATUMESH_SERIES_FILE".into(), p.to_string_lossy().into_owned()));
    }
    let cfg = Config::from_vars(move |k| {
        if k == "DATUMESH_HOST" {
            return Some(HOST.to_string());
        }
        env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    })
    .unwrap();
    build_served(lite, Some(cfg), |_| {}).await
}

async fn served(label: &str) -> Served {
    serve(label, Some(series_json()), &[]).await
}

const BROWSER: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,*/*;q=0.8";
const ARROW_TYPE: &str = "application/vnd.apache.arrow.stream";

/// A GET that does not follow redirects.
async fn fetch(m: &Served, path: &str, headers: &[(&str, &str)]) -> Reply4 {
    let http = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let mut rb = http.get(format!("{}{path}", m.base));
    for (k, v) in headers {
        rb = rb.header(*k, *v);
    }
    let r = rb.send().await.unwrap();
    let (status, headers) = (r.status().as_u16(), r.headers().clone());
    Reply4 { status, headers, body: r.bytes().await.unwrap().to_vec() }
}

fn page(f: &str, period: &str) -> String {
    format!("/series/{f}/{period}")
}
fn index(f: &str) -> String {
    format!("/series/{f}/")
}

/// 200 and HTML; returns the text.
async fn html(m: &Served, path: &str) -> String {
    let r = fetch(m, path, &[]).await;
    assert_eq!(r.status, 200, "{path}: {}", r.text());
    assert!(
        r.header("content-type").unwrap_or_default().starts_with("text/html"),
        "{path}: {:?}",
        r.header("content-type")
    );
    r.text()
}

// ---- reading HTML ---------------------------------------------------------------------------------

fn decode(t: &str) -> String {
    let mut out = String::new();
    let mut rest = t;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').filter(|e| *e <= 10);
        let ch = end.and_then(|e| match &rest[1..e] {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            n if n.starts_with("#x") || n.starts_with("#X") => {
                u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32)
            }
            n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        });
        match (ch, end) {
            (Some(c), Some(e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn strip_tags(h: &str) -> String {
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
    out
}

/// Visible text: tags removed (the JSON-LD script's text too), entities decoded, white space folded.
fn plain(h: &str) -> String {
    let mut h = h.to_string();
    while let Some(i) = h.to_lowercase().find("<script") {
        let end = h.to_lowercase()[i..].find("</script>").map(|e| i + e + 9).unwrap_or(h.len());
        h.replace_range(i..end, " ");
    }
    decode(&strip_tags(&h)).replace('\u{a0}', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn squash(t: &str) -> String {
    t.split_whitespace().collect()
}

/// The text of the first `<tag ...>...</tag>`, decoded and trimmed.
fn tag_text(h: &str, tag: &str) -> String {
    let i = h.find(&format!("<{tag}")).unwrap_or_else(|| panic!("no <{tag}> in {h}"));
    let from = i + h[i..].find('>').unwrap() + 1;
    let to = from + h[from..].find(&format!("</{tag}>")).unwrap();
    decode(&strip_tags(&h[from..to])).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The cells (`th` or `td`) of a piece of HTML, as text.
fn cells(seg: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = seg.as_bytes();
    let mut i = 0;
    while let Some(p) = seg[i..].find("<t") {
        let at = i + p;
        let kind = b.get(at + 2).copied();
        let after = b.get(at + 3).copied();
        if matches!(kind, Some(b'h' | b'd')) && matches!(after, Some(b'>' | b' ')) {
            let from = at + seg[at..].find('>').unwrap() + 1;
            let close = if kind == Some(b'h') { "</th>" } else { "</td>" };
            let to = from + seg[from..].find(close).unwrap_or_else(|| panic!("unclosed cell in {seg}"));
            out.push(decode(&strip_tags(&seg[from..to])).trim().to_string());
            i = to;
        } else {
            i = at + 2;
        }
    }
    out
}

/// The header row and the body rows of THE table of a page, after checking its structure: one `<table>`, one
/// `<thead>` with one row, a `<tbody>` with a row per date, no merged cells, no footer, headers only in the head.
fn table(h: &str) -> (Vec<String>, Vec<Vec<String>>) {
    assert_eq!(h.matches("<table").count(), 1, "exactly one table");
    let t = &h[h.find("<table").unwrap()..h.find("</table>").expect("a closed table")];
    for no in ["colspan", "rowspan", "<tfoot"] {
        assert!(!t.contains(no), "{no} in the table: {t}");
    }
    assert_eq!(t.matches("<thead").count(), 1);
    let head = &t[t.find("<thead").unwrap()..t.find("</thead>").unwrap()];
    assert_eq!(head.matches("<tr").count(), 1, "exactly one header row: {head}");
    let body = &t[t.find("<tbody").expect("a tbody")..];
    assert!(!body.contains("<th"), "header cells in the body: {body}");
    let rows: Vec<Vec<String>> =
        body.split("<tr").skip(1).map(|r| cells(&r[..r.find("</tr>").expect("closed row")])).collect();
    assert_eq!(t.matches("<tr").count(), 1 + rows.len(), "no row outside the head and the body: {t}");
    let header = cells(head);
    for r in &rows {
        assert_eq!(r.len(), header.len(), "a rectangular grid: {t}");
    }
    assert_eq!(head.matches("<th>").count() + head.matches("<th ").count(), header.len());
    (header, rows)
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}
fn grid(v: &[&[&str]]) -> Vec<Vec<String>> {
    v.iter().map(|r| strs(r)).collect()
}

struct Anchor {
    attrs: String,
    href: String,
    #[allow(dead_code)]
    text: String,
}

fn attr(attrs: &str, name: &str) -> Option<String> {
    for q in ['"', '\''] {
        let key = format!("{name}={q}");
        if let Some(i) = attrs.find(&key) {
            let from = i + key.len();
            let to = from + attrs[from..].find(q)?;
            return Some(decode(&attrs[from..to]));
        }
    }
    None
}

fn anchors(h: &str) -> Vec<Anchor> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = h[i..].find("<a ") {
        let at = i + p;
        let attrs_end = at + h[at..].find('>').unwrap();
        let attrs = h[at + 3..attrs_end].to_string();
        let close = attrs_end + h[attrs_end..].find("</a>").unwrap_or(h.len() - attrs_end);
        let text = decode(&strip_tags(&h[attrs_end + 1..close])).trim().to_string();
        if let Some(href) = attr(&attrs, "href") {
            out.push(Anchor { attrs, href, text });
        }
        i = attrs_end;
    }
    out
}

/// An href as a root-relative path (the mesh's own origin removed).
fn path_of(href: &str) -> String {
    href.strip_prefix(&origin()).unwrap_or(href).to_string()
}

fn rel(h: &str, token: &str) -> Option<String> {
    anchors(h)
        .into_iter()
        .find(|a| attr(&a.attrs, "rel").is_some_and(|r| r.split_whitespace().any(|t| t == token)))
        .map(|a| path_of(&a.href))
}

fn has_link(h: &str, path: &str) -> bool {
    anchors(h).iter().any(|a| path_of(&a.href) == path)
}

fn ld_text(h: &str) -> String {
    let i = h.find("application/ld+json").expect("a JSON-LD block");
    let from = i + h[i..].find('>').unwrap() + 1;
    let to = from + h[from..].find("</script>").unwrap();
    h[from..to].to_string()
}

fn ld(h: &str) -> Value {
    let t = ld_text(h);
    serde_json::from_str(&t).unwrap_or_else(|e| panic!("the JSON-LD does not parse ({e}): {t}"))
}

/// The name of an organisation given as a string or as an object.
fn org(v: &Value) -> String {
    v.as_str().map(str::to_string).or_else(|| v["name"].as_str().map(str::to_string)).unwrap_or_default()
}

/// (public, edge lifetime in seconds: `s-maxage`, else `max-age`).
fn edge(r: &Reply4) -> (bool, Option<u64>) {
    let cc = r.header("cache-control").unwrap_or_default();
    let mut public = false;
    let (mut max, mut smax) = (None, None);
    for d in cc.split(',').map(str::trim) {
        if d == "public" {
            public = true;
        } else if let Some(n) = d.strip_prefix("s-maxage=") {
            smax = n.parse().ok();
        } else if let Some(n) = d.strip_prefix("max-age=") {
            max = n.parse().ok();
        }
    }
    (public, smax.or(max))
}

fn csv_lines(body: &str) -> Vec<String> {
    body.strip_suffix("\r\n").unwrap_or(body).split("\r\n").map(str::to_string).collect()
}

// ---- shape: the table -------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_keyed_month_page_is_one_clean_table() {
    let m = served("sp-keyed").await;
    let r = fetch(&m, &page(FXR, "2026-03"), &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(r.header("content-type").unwrap().starts_with("text/html"));
    let h = r.text();
    assert!(h.contains("<html lang=\"en\""), "{h}");
    let (head, rows) = table(&h);
    // the unit is in the header; the columns are in dictionary order (USD, JPY, GBP), not alphabetical
    assert_eq!(head, strs(&["date", "USD (units per EUR)", "JPY (units per EUR)", "GBP (units per EUR)"]));
    // ISO dates, the stored decimal strings as they are (no thousands separators, no rounding)
    assert_eq!(rows, grid(&[&["2026-03-02", "1.0832", "160.25", "0.8301"], &["2026-03-03", "1.0791", "150", "0.829"]]));
    // the heading and the title: the catalogue label and the month
    assert_eq!(h.matches("<h1").count(), 1);
    for t in [tag_text(&h, "h1"), tag_text(&h, "title")] {
        assert!(t.contains(FX_LABEL) && t.contains("2026-03"), "{t}");
    }
    // empty cells where there is no value, and a date with at least one cell is a row
    let (_, jan) = table(&html(&m, &page(FXR, "2026-01")).await);
    assert_eq!(jan, grid(&[&["2026-01-05", "1.0412", "163.45", "0.8567"], &["2026-01-06", "1.045", "164.1", ""]]));
    let (_, nov) = table(&html(&m, &page(FXR, "2026-11")).await);
    assert_eq!(nov, grid(&[&["2026-11-13", "1.0892", "", ""]]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_unkeyed_month_page_has_one_value_column() {
    let m = served("sp-unkeyed").await;
    let (head, rows) = table(&html(&m, &page(BANK, "2026-03")).await);
    assert_eq!(head, strs(&["date", "value (percent)"]));
    assert_eq!(rows, grid(&[&["2026-03-19", "4.5"], &["2026-03-20", "4.5"]]));
    let (_, may) = table(&html(&m, &page(BANK, "2026-05")).await);
    assert_eq!(may, grid(&[&["2026-05-07", "4.25"]]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_year_page_of_an_event_series() {
    let m = served("sp-year").await;
    let h = html(&m, &page(POLICY, "2022")).await;
    let (head, rows) = table(&h);
    assert_eq!(head, strs(&["date", "mro (percent)", "dfr (percent)", "mlfr (percent)"]));
    assert_eq!(rows, grid(&[&["2022-07-27", "0.5", "0", "0.75"], &["2022-09-14", "1.25", "0.75", "1.5"]]));
    let t = tag_text(&h, "h1");
    assert!(t.contains("ECB key interest rates") && t.contains("2022"), "{t}");
    assert_eq!(table(&html(&m, &page(POLICY, "2023")).await).1, grid(&[&["2023-06-21", "4", "3.5", ""]]));
    assert_eq!(table(&html(&m, &page(POLICY, "2026")).await).1, grid(&[&["2026-06-11", "", "2", ""]]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_computed_monthly_series_has_year_pages() {
    let m = served("sp-rain").await;
    let (head, rows) = table(&html(&m, &page(RAIN, "2024")).await);
    // dictionary order (us_kansas, ua_kharkiv); the unit text holds a comma; the fixed dataset is no column
    assert_eq!(head, strs(&["date", "us_kansas (millimetres, per month)", "ua_kharkiv (millimetres, per month)"]));
    assert_eq!(rows, grid(&[&["2024-11-01", "12.5", "40"], &["2024-12-01", "1234.5", ""]]));
    assert_eq!(table(&html(&m, &page(RAIN, "2025")).await).1, grid(&[&["2025-01-01", "0.25", "33.75"]]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_page_has_no_script_but_the_json_ld() {
    let m = served("sp-noscript").await;
    for p in [page(FXR, "2026-03"), page(POLICY, "2022"), page(RAIN, "2024"), index(FXR)] {
        let h = html(&m, &p).await;
        let lower = h.to_lowercase();
        assert_eq!(lower.matches("<script").count(), 1, "{p}: only the JSON-LD");
        assert_eq!(lower.matches("</script").count(), 1, "{p}");
        assert!(
            lower.contains("<script type=\"application/ld+json\"")
                || lower.contains("<script type='application/ld+json'"),
            "{p}"
        );
        for no in
            ["javascript:", " onload=", " onclick=", " onerror=", "<iframe", "<link rel=\"stylesheet\"", "<style src"]
        {
            assert!(!lower.contains(no), "{p}: {no}");
        }
        assert!(!ld_text(&h).contains('<'), "{p}: the JSON-LD escapes `<`");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn text_from_the_publisher_is_escaped() {
    let m = served("sp-escape").await;
    for p in [page(BANK, "2026-03"), index(BANK)] {
        let h = html(&m, &p).await;
        assert!(!h.contains("<b>rate</b>"), "{p}: the label is escaped");
        assert!(!h.contains("<img src=x"), "{p}: the description is escaped");
        assert!(!h.contains("</script><script>"), "{p}");
        assert_eq!(h.to_lowercase().matches("<script").count(), 1, "{p}");
        let t = tag_text(&h, "h1");
        assert!(t.contains(BANK_LABEL), "{p}: the label shows as text: {t}");
        let j = ld(&h);
        assert!(j["name"].as_str().unwrap().contains(BANK_LABEL), "{j}");
        assert!(!ld_text(&h).contains('<'), "{p}");
    }
}

// ---- CSV --------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_csv_twin_has_the_same_rows_and_header() {
    let m = served("sp-csv").await;
    for (p, want) in [
        (
            page(FXR, "2026-03.csv"),
            "date,USD (units per EUR),JPY (units per EUR),GBP (units per EUR)\r\n2026-03-02,1.0832,160.25,0.8301\r\n2026-03-03,1.0791,150,0.829\r\n",
        ),
        (
            page(FXR, "2026-01.csv"),
            "date,USD (units per EUR),JPY (units per EUR),GBP (units per EUR)\r\n2026-01-05,1.0412,163.45,0.8567\r\n2026-01-06,1.045,164.1,\r\n",
        ),
        (page(BANK, "2026-03.csv"), "date,value (percent)\r\n2026-03-19,4.5\r\n2026-03-20,4.5\r\n"),
        (
            page(POLICY, "2022.csv"),
            "date,mro (percent),dfr (percent),mlfr (percent)\r\n2022-07-27,0.5,0,0.75\r\n2022-09-14,1.25,0.75,1.5\r\n",
        ),
        (
            page(RAIN, "2024.csv"),
            "date,\"us_kansas (millimetres, per month)\",\"ua_kharkiv (millimetres, per month)\"\r\n2024-11-01,12.5,40\r\n2024-12-01,1234.5,\r\n",
        ),
    ] {
        let r = fetch(&m, &p, &[]).await;
        assert_eq!(r.status, 200, "{p}: {}", r.text());
        let ct = r.header("content-type").unwrap();
        assert!(ct.starts_with("text/csv"), "{p}: {ct}");
        assert!(!ct.contains("charset") || ct.to_lowercase().contains("charset=utf-8"), "{ct}");
        assert_eq!(r.body, want.as_bytes(), "{p}: {}", r.text());
        assert!(!r.body.starts_with(&[0xEF, 0xBB, 0xBF]), "no byte order mark");
    }
    // the same grid as the table of the page
    let (head, rows) = table(&html(&m, &page(FXR, "2026-03")).await);
    let csv = csv_lines(&fetch(&m, &page(FXR, "2026-03.csv"), &[]).await.text());
    let mut want = vec![head.join(",")];
    want.extend(rows.iter().map(|r| r.join(",")));
    assert_eq!(csv, want);
    // no page, no CSV
    assert_eq!(fetch(&m, &page(FXR, "2026-02.csv"), &[]).await.status, 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_page_links_its_csv_and_says_how_to_import_it() {
    let m = served("sp-csvlink").await;
    for (f, period) in [(FXR, "2026-03"), (POLICY, "2022"), (RAIN, "2024")] {
        let h = html(&m, &page(f, period)).await;
        let csv = format!("{}{}", origin(), page(f, &format!("{period}.csv")));
        assert!(has_link(&h, &page(f, &format!("{period}.csv"))), "{f}: a link to the CSV");
        let want = squash(&format!(
            "Excel (Data > From Web) and Google Sheets (=IMPORTDATA(\"{csv}\")) can import this CSV from its address."
        ));
        assert!(squash(&plain(&h)).contains(&want), "{f}: {}", plain(&h));
    }
}

// ---- the redirect from the machine form ---------------------------------------------------------------

fn is_to(r: &Reply4, path: &str) -> bool {
    r.status == 303
        && r.header("cache-control").as_deref() == Some("no-store")
        && r.header("location").is_some_and(|l| path_of(&l) == path)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_browser_on_the_slice_address_is_sent_to_the_page() {
    let m = served("sp-redirect").await;
    let cases = [
        // one date: the month (the year of a yearly series)
        (format!("/slice/{FXR}?currency=*&date=2026-03-02"), page(FXR, "2026-03")),
        (format!("/slice/{FXR}?currency=*&date=2026-01-06"), page(FXR, "2026-01")),
        (format!("/slice/{BANK}?date=2026-05-07"), page(BANK, "2026-05")),
        (format!("/slice/{POLICY}?rate=*&date=2022-07-27"), page(POLICY, "2022")),
        (format!("/slice/{RAIN}?dataset=chirps&region=*&month=2024-12-01"), page(RAIN, "2024")),
        // wider: the index
        (format!("/slice/{FXR}?currency=USD&date=*"), index(FXR)),
        (format!("/slice/{FXR}?currency=*&date=*"), index(FXR)),
        (format!("/slice/{POLICY}?rate=mro&date=*"), index(POLICY)),
    ];
    for accept in ["text/html", BROWSER] {
        for (from, to) in &cases {
            let r = fetch(&m, from, &[("accept", accept)]).await;
            assert!(
                is_to(&r, to),
                "{from} ({accept}): {} {:?} {:?}",
                r.status,
                r.header("location"),
                r.header("cache-control")
            );
            // the target is a page
            let t = fetch(&m, to, &[]).await;
            assert_eq!(t.status, 200, "{to}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_json_client_gets_json_unchanged() {
    let configured = served("sp-json-a").await;
    let bare = serve("sp-json-b", None, &[]).await;
    let path = format!("/slice/{FXR}?currency=*&date=2026-03-02");
    // positive control: the browser is redirected here
    let r = fetch(&configured, &path, &[("accept", "text/html")]).await;
    assert!(is_to(&r, &page(FXR, "2026-03")), "{} {:?}", r.status, r.header("location"));
    let baseline = fetch(&bare, &path, &[]).await;
    assert_eq!(baseline.status, 200, "{}", baseline.text());
    for accept in [None, Some("application/json"), Some("*/*"), Some("application/json, text/html;q=0.9")] {
        let headers: Vec<(&str, &str)> = accept.map(|a| ("accept", a)).into_iter().collect();
        let r = fetch(&configured, &path, &headers).await;
        assert_eq!(r.status, 200, "{accept:?}: {}", r.text());
        assert_eq!(r.json(), baseline.json(), "{accept:?}");
        assert_eq!(r.header("content-type"), baseline.header("content-type"), "{accept:?}");
        assert_eq!(r.header("cache-control"), baseline.header("cache-control"), "{accept:?}");
        assert_ne!(r.header("cache-control").as_deref(), Some("no-store"), "{accept:?}: JSON is the cacheable form");
    }
    // a function that no series lists is not redirected, whatever the Accept
    let plain = fetch(&configured, &format!("/slice/{PLAIN}?site=*"), &[("accept", BROWSER)]).await;
    assert_eq!(plain.status, 200, "{}", plain.text());
    let verse = fetch(&configured, &format!("/slice/{VERSE}?site=*"), &[("accept", "text/html")]).await;
    assert_ne!(verse.status, 303, "no page, no redirect (a bare `text/html` is 406 here today, unchanged)");
    let same = fetch(&bare, &format!("/slice/{VERSE}?site=*"), &[("accept", "text/html")]).await;
    assert_eq!((verse.status, verse.header("cache-control")), (same.status, same.header("cache-control")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn csv_and_arrow_are_addresses_of_their_own() {
    let m = served("sp-formats").await;
    let base = format!("/slice/{FXR}?currency=*&date=2026-03-02");
    let json = fetch(&m, &base, &[]).await;
    assert_eq!(json.status, 200);
    // ?format=csv: CSV, whatever the Accept (a browser is not redirected)
    for accept in [None, Some(BROWSER)] {
        let headers: Vec<(&str, &str)> = accept.map(|a| ("accept", a)).into_iter().collect();
        let r = fetch(&m, &format!("{base}&format=csv"), &headers).await;
        assert_eq!(r.status, 200, "{accept:?}: {}", r.text());
        assert!(r.header("content-type").unwrap().starts_with("text/csv"), "{:?}", r.header("content-type"));
        assert!(r.text().contains("1.0832"), "{}", r.text());
        assert_eq!(r.header("cache-control"), json.header("cache-control"), "one form, cached like its address");
    }
    // ?format=arrow: the same stream as the Accept form, whatever the Accept
    let by_accept = fetch(&m, &base, &[("accept", ARROW_TYPE)]).await;
    assert_eq!(by_accept.status, 200);
    assert_eq!(by_accept.header("content-type").as_deref(), Some(ARROW_TYPE));
    // an Accept-chosen answer other than the default JSON is not cached: the address is only ever cached as JSON
    assert_eq!(by_accept.header("cache-control").as_deref(), Some("no-store"));
    for accept in [None, Some(BROWSER)] {
        let headers: Vec<(&str, &str)> = accept.map(|a| ("accept", a)).into_iter().collect();
        let r = fetch(&m, &format!("{base}&format=arrow"), &headers).await;
        assert_eq!(r.status, 200, "{accept:?}: {}", r.text());
        assert_eq!(r.header("content-type").as_deref(), Some(ARROW_TYPE));
        assert_eq!(r.body, by_accept.body, "{accept:?}");
        assert_eq!(r.header("cache-control"), json.header("cache-control"), "one form, cached like its address");
    }
}

// ---- navigation ---------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn previous_and_next_skip_the_empty_periods() {
    let m = served("sp-nav").await;
    let nav = |p: String| {
        let m = &m;
        async move {
            let h = html(m, &p).await;
            (rel(&h, "prev"), rel(&h, "next"), h)
        }
    };
    // daily series: 2026-01, (02 has no cell), 03, (04 to 10 have none), 11
    let (prev, next, h) = nav(page(FXR, "2026-03")).await;
    assert_eq!((prev, next), (Some(page(FXR, "2026-01")), Some(page(FXR, "2026-11"))));
    assert!(has_link(&h, &index(FXR)), "the series index");
    assert!(has_link(&h, &format!("/fn/{FXR}")), "the function page");
    let (prev, next, _) = nav(page(FXR, "2026-01")).await;
    assert_eq!((prev, next), (None, Some(page(FXR, "2026-03"))), "the first page has no previous");
    let (prev, next, _) = nav(page(FXR, "2026-11")).await;
    assert_eq!((prev, next), (Some(page(FXR, "2026-03")), None), "the last page has no next");
    let (prev, next, _) = nav(page(BANK, "2026-03")).await;
    assert_eq!((prev, next), (None, Some(page(BANK, "2026-05"))));
    let (prev, next, _) = nav(page(BANK, "2026-05")).await;
    assert_eq!((prev, next), (Some(page(BANK, "2026-03")), None));
    // yearly series: 2022, (2023), 2026 for the policy rates
    let (prev, next, h) = nav(page(POLICY, "2023")).await;
    assert_eq!((prev, next), (Some(page(POLICY, "2022")), Some(page(POLICY, "2026"))));
    assert!(has_link(&h, &index(POLICY)) && has_link(&h, &format!("/fn/{POLICY}")));
    assert_eq!(nav(page(POLICY, "2022")).await.0, None);
    assert_eq!(nav(page(POLICY, "2026")).await.1, None);
    let (prev, next, _) = nav(page(RAIN, "2024")).await;
    assert_eq!((prev, next), (None, Some(page(RAIN, "2025"))));
}

// ---- index and sitemap ----------------------------------------------------------------------------------

fn listed(h: &str, f: &str) -> BTreeSet<String> {
    let own = index(f);
    anchors(h)
        .iter()
        .map(|a| path_of(&a.href))
        .filter(|p| p.starts_with(&own) && *p != own && !p.ends_with(".csv"))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_index_lists_exactly_the_periods_with_data() {
    let m = served("sp-index").await;
    let set = |f: &str, ps: &[&str]| ps.iter().map(|p| page(f, p)).collect::<BTreeSet<_>>();
    let h = html(&m, &index(FXR)).await;
    assert_eq!(listed(&h, FXR), set(FXR, &["2026-01", "2026-03", "2026-11"]));
    assert!(plain(&h).contains("2026"), "the months are grouped under their year");
    assert!(tag_text(&h, "h1").contains(FX_LABEL));
    assert_eq!(listed(&html(&m, &index(BANK)).await, BANK), set(BANK, &["2026-03", "2026-05"]));
    // a yearly series lists years, each with its page
    assert_eq!(listed(&html(&m, &index(POLICY)).await, POLICY), set(POLICY, &["2022", "2023", "2026"]));
    assert_eq!(listed(&html(&m, &index(RAIN)).await, RAIN), set(RAIN, &["2024", "2025"]));
    // the index is a page of the series, with the function page one click away
    assert!(has_link(&h, &format!("/fn/{FXR}")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_sitemap_lists_every_page_with_its_as_of() {
    let m = served("sp-sitemap").await;
    let r = fetch(&m, "/series/sitemap.xml", &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(r.header("content-type").unwrap().contains("xml"), "{:?}", r.header("content-type"));
    let x = r.text();
    assert!(x.contains("http://www.sitemaps.org/schemas/sitemap/0.9"), "{x}");
    let mut got: Vec<(String, String)> = Vec::new();
    for u in x.split("<url>").skip(1) {
        let loc = u.split("<loc>").nth(1).and_then(|t| t.split("</loc>").next()).expect("a loc").trim().to_string();
        let last = u
            .split("<lastmod>")
            .nth(1)
            .and_then(|t| t.split("</lastmod>").next())
            .expect("a lastmod")
            .trim()
            .to_string();
        if !loc.ends_with(".csv") {
            got.push((path_of(&decode(&loc)), last));
        }
    }
    let want: Vec<(String, &str)> = vec![
        (index(FXR), "2026-11-13"),
        (page(FXR, "2026-01"), "2026-01-06"),
        (page(FXR, "2026-03"), "2026-03-03"),
        (page(FXR, "2026-11"), "2026-11-13"),
        (index(BANK), "2026-05-07"),
        (page(BANK, "2026-03"), "2026-03-20"),
        (page(BANK, "2026-05"), "2026-05-07"),
        (index(POLICY), "2026-06-11"),
        (page(POLICY, "2022"), "2026-02-02"),
        (page(POLICY, "2023"), "2026-02-03"),
        (page(POLICY, "2026"), "2026-06-11"),
        (index(RAIN), "2026-02-11"),
        (page(RAIN, "2024"), "2026-02-10"),
        (page(RAIN, "2025"), "2026-02-11"),
    ];
    let got_paths: BTreeSet<&String> = got.iter().map(|g| &g.0).collect();
    let want_paths: BTreeSet<&String> = want.iter().map(|w| &w.0).collect();
    assert_eq!(
        got_paths, want_paths,
        "every index and page, and nothing else (no derived function, no scripture): {x}"
    );
    assert_eq!(got.len(), want.len(), "each once: {x}");
    for (loc, date) in &want {
        let (_, last) = got.iter().find(|g| &g.0 == loc).unwrap();
        assert!(last.starts_with(date), "{loc}: lastmod {last}, the as-of is {date}");
    }
    // absolute URLs on the mesh's own origin
    for l in x.split("<loc>").skip(1) {
        assert!(l.trim_start().starts_with(&origin()), "{l}");
    }
}

// ---- no work for a crawler -------------------------------------------------------------------------------

/// An origin that counts the connections it gets.
async fn counting_origin() -> (String, Arc<AtomicUsize>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let n = Arc::new(AtomicUsize::new(0));
    let c = n.clone();
    tokio::spawn(async move {
        loop {
            let Ok((conn, _)) = l.accept().await else { return };
            c.fetch_add(1, Ordering::SeqCst);
            drop(conn);
        }
    });
    (format!("http://{addr}"), n)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_page_without_stored_cells_is_404_and_nothing_is_computed_or_fetched() {
    let (origin_url, fetched) = counting_origin().await;
    let peers = format!("foreign.example={origin_url}");
    let m = serve("sp-nowork", Some(series_json()), &[("DATUMESH_PEERS", &peers), ("DATUMESH_CROSS_MESH", "1")]).await;
    // positive control: a page of stored cells is served
    assert_eq!(fetch(&m, &page(RAIN, "2024"), &[]).await.status, 200);
    let before = snapshot(&m.lite);
    for p in [
        // a computed function with no stored cell: evaluating it would give values, so a 200 would show a computation
        page(DERIVED, "2024"),
        page(DERIVED, "2025"),
        format!("{}.csv", page(DERIVED, "2024")),
        index(DERIVED),
        // a month with no stored cell of a series that has others
        page(FXR, "2026-02"),
        page(FXR, "2026-04"),
        page(BANK, "2026-04"),
        page(POLICY, "2024"),
        page(RAIN, "2023"),
        // a function of another mesh: nothing is fetched
        page(FOREIGN, "2026-03"),
        index(FOREIGN),
        // not listed in the series file
        page(VERSE, "2026-03"),
        index(VERSE),
        page(PLAIN, "2026-03"),
    ] {
        let r = fetch(&m, &p, &[]).await;
        assert_eq!(r.status, 404, "{p}: {}", r.text());
    }
    let sm = fetch(&m, "/series/sitemap.xml", &[]).await.text();
    assert!(
        !sm.contains(DERIVED) && !sm.contains(FOREIGN) && !sm.contains("verse") && !sm.contains("plain/level"),
        "{sm}"
    );
    assert_eq!(snapshot(&m.lite), before, "the store is as it was: nothing computed, nothing stored");
    assert_eq!(fetched.load(Ordering::SeqCst), 0, "nothing fetched from another mesh");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn periods_that_are_no_period_are_404() {
    let m = served("sp-badperiod").await;
    assert_eq!(fetch(&m, &page(FXR, "2026-03"), &[]).await.status, 200, "control");
    for p in [
        page(FXR, "2026-13"),
        page(FXR, "2026-00"),
        page(FXR, "2026-3"),
        page(FXR, "2026"),       // a daily series has month pages, not year pages
        page(POLICY, "2022-07"), // a yearly series has year pages, not month pages
        page(FXR, "abcd"),
        page(FXR, "2026-03-02"),
        page(FXR, "1999-01"), // before the data
        page(FXR, "9999-12"), // after it
        "/series/mesh-test.datumesh.com/nope/fn/2026-03".to_string(),
    ] {
        let r = fetch(&m, &p, &[]).await;
        assert_eq!(r.status, 404, "{p}: {}", r.text());
    }
}

// ---- caching --------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cache_control_by_age_of_the_period() {
    let m = served("sp-cc").await;
    let past = "public, max-age=300, s-maxage=86400";
    let current = "public, max-age=300, s-maxage=600";
    for (p, want) in [
        (page(FXR, "2026-03"), past),
        (page(FXR, "2026-03.csv"), past),
        (page(FXR, "2026-01"), past),
        (page(FXR, "2026-11"), current),
        (page(FXR, "2026-11.csv"), current),
        (page(BANK, "2026-05"), past),
        (page(POLICY, "2022"), past),
        (page(POLICY, "2026"), current),
        (page(POLICY, "2026.csv"), current),
        (page(RAIN, "2025"), past),
    ] {
        let r = fetch(&m, &p, &[]).await;
        assert_eq!(r.status, 200, "{p}");
        assert_eq!(r.header("cache-control").as_deref(), Some(want), "{p}");
    }
    // the 404 of an empty period is cacheable for ten minutes
    for p in [page(FXR, "2026-02"), page(FXR, "2026-02.csv"), page(DERIVED, "2024")] {
        let r = fetch(&m, &p, &[]).await;
        assert_eq!(r.status, 404, "{p}");
        assert_eq!(edge(&r), (true, Some(600)), "{p}: {:?}", r.header("cache-control"));
    }
    // the index and the sitemap are cacheable too
    for p in [index(FXR), "/series/sitemap.xml".to_string()] {
        let r = fetch(&m, &p, &[]).await;
        assert_eq!(r.status, 200, "{p}");
        let (public, ttl) = edge(&r);
        assert!(public && ttl.is_some_and(|t| t > 0 && t <= 86400), "{p}: {:?}", r.header("cache-control"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn etag_and_304() {
    let m = served("sp-etag").await;
    let p = page(FXR, "2026-03");
    let r = fetch(&m, &p, &[]).await;
    assert_eq!(r.status, 200);
    let tag = r.header("etag").expect("an ETag");
    assert!(tag.starts_with('"') || tag.starts_with("W/\""), "{tag}");
    // a conditional GET with the tag: 304, no body, the same validators
    let c = fetch(&m, &p, &[("if-none-match", &tag)]).await;
    assert_eq!(c.status, 304);
    assert!(c.body.is_empty());
    assert_eq!(c.header("etag").as_deref(), Some(tag.as_str()));
    assert_eq!(c.header("cache-control"), r.header("cache-control"));
    // another page, another tag; a stale tag gets the page
    let other = fetch(&m, &page(FXR, "2026-01"), &[]).await;
    assert_ne!(other.header("etag"), Some(tag.clone()));
    assert_eq!(fetch(&m, &p, &[("if-none-match", "\"stale\"")]).await.status, 200);
    assert_eq!(fetch(&m, &page(FXR, "2026-01"), &[("if-none-match", &tag)]).await.status, 200);
    // the CSV twin has its tag and its 304
    let csv_path = page(FXR, "2026-03.csv");
    let csv = fetch(&m, &csv_path, &[]).await;
    let ctag = csv.header("etag").expect("an ETag on the CSV");
    assert_eq!(fetch(&m, &csv_path, &[("if-none-match", &ctag)]).await.status, 304);
    // a new version of a cell of the month changes the tag and the row
    let f = write(
        &m.lite,
        "v2.jsonl",
        &[stored(
            FXR,
            json!({"currency": s("USD"), "date": d("2026-03-02")}),
            "1.0833",
            T2,
            ECB_LICENCE,
            Some(source("ecb.europa.eu")),
        )],
    );
    run(&m.lite, &f, T2).unwrap_or_else(|e| panic!("{e}"));
    let after = fetch(&m, &p, &[("if-none-match", &tag)]).await;
    assert_eq!(after.status, 200, "the old tag no longer matches");
    assert_ne!(after.header("etag").as_deref(), Some(tag.as_str()));
    assert_eq!(table(&after.text()).1[0], strs(&["2026-03-02", "1.0833", "160.25", "0.8301"]));
    assert!(plain(&after.text()).contains("2026-11-15T12:00:02"), "the as-of is the latest tx");
    // the other months' tags are unchanged
    assert_eq!(fetch(&m, &page(FXR, "2026-01"), &[]).await.header("etag"), other.header("etag"));
}

// ---- metadata, source line, licence, as-of, verify, agents ---------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_json_ld_describes_the_dataset() {
    let m = served("sp-ld").await;
    let h = html(&m, &page(FXR, "2026-03")).await;
    let j = ld(&h);
    assert!(j["@context"].as_str().unwrap().contains("schema.org"), "{j}");
    assert_eq!(j["@type"], "Dataset");
    assert!(j["name"].as_str().unwrap().contains(FX_LABEL) && j["name"].as_str().unwrap().contains("2026-03"), "{j}");
    assert!(!j["description"].as_str().unwrap_or("").is_empty(), "{j}");
    assert_eq!(j["url"], format!("{}{}", origin(), page(FXR, "2026-03")));
    let creators = [org(&j["creator"]), org(&j["sourceOrganization"])];
    assert!(creators.iter().any(|c| c.contains("ecb.europa.eu")), "the source: {j}");
    assert!(j["license"].to_string().contains(ECB_LICENCE), "{j}");
    let cov = j["temporalCoverage"].as_str().unwrap();
    assert!(cov == "2026-03" || cov == "2026-03-01/2026-03-31", "{cov}");
    assert!(j["dateModified"].as_str().unwrap().starts_with("2026-03-03T15:00:00"), "the as-of: {j}");
    assert_eq!(j["isAccessibleForFree"], json!(true));
    let vars = j["variableMeasured"].as_array().expect("variableMeasured");
    assert_eq!(vars.len(), 3, "{j}");
    for (v, code) in vars.iter().zip(["USD", "JPY", "GBP"]) {
        let t = v.to_string();
        assert!(
            v["name"].as_str().unwrap_or("").starts_with(code) || v.as_str().is_some_and(|x| x.starts_with(code)),
            "{t}"
        );
        assert!(t.contains("units per EUR"), "the unit: {t}");
    }
    let dist = j["distribution"].as_array().expect("distribution");
    let url_of = |fmt: &str| {
        dist.iter().find(|x| x["encodingFormat"] == fmt).map(|x| x["contentUrl"].as_str().unwrap_or("").to_string())
    };
    assert_eq!(url_of("text/csv"), Some(format!("{}{}", origin(), page(FXR, "2026-03.csv"))), "{j}");
    let json_url = url_of("application/json").expect("a JSON distribution");
    assert!(json_url.contains(&format!("/slice/{FXR}")), "{json_url}");
    // the yearly event series: the year
    let y = ld(&html(&m, &page(POLICY, "2022")).await);
    let cov = y["temporalCoverage"].as_str().unwrap();
    assert!(cov == "2022" || cov == "2022-01-01/2022-12-31", "{cov}");
    assert_eq!(y["variableMeasured"].as_array().unwrap().len(), 3);
    // the computed series: its creator is not the relayed source
    let c = ld(&html(&m, &page(RAIN, "2024")).await);
    assert_eq!(c["@type"], "Dataset");
    assert_eq!(c["url"], format!("{}{}", origin(), page(RAIN, "2024")));
    assert!(c["license"].to_string().contains("CC-BY-4.0"), "{c}");
    assert!(c["dateModified"].as_str().unwrap().starts_with("2026-02-10T08:00:00"), "{c}");
    let who = [org(&c["creator"]), org(&c["sourceOrganization"])].join(" ").to_lowercase();
    assert!(who.contains(HOST) || who.contains("chirps") || who.contains("chc.ucsb.edu"), "{c}");
    assert!(!who.contains("ecb.europa.eu"));
    // the index covers the whole series
    let i = ld(&html(&m, &index(FXR)).await);
    assert_eq!(i["@type"], "Dataset");
    assert_eq!(i["url"], format!("{}{}", origin(), index(FXR)));
    let cov = i["temporalCoverage"].as_str().unwrap();
    assert!(cov.contains("2026-01") && cov.contains("2026-11"), "{cov}");
    assert!(i["dateModified"].as_str().unwrap().starts_with("2026-11-13T15:00:00"), "{i}");
    assert_eq!(i["isAccessibleForFree"], json!(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_relayed_page_says_who_relays_and_how_the_source_takes_over() {
    let m = served("sp-source").await;
    for (f, period, domain) in
        [(FXR, "2026-03", "ecb.europa.eu"), (BANK, "2026-03", "bankofengland.co.uk"), (POLICY, "2022", "ecb.europa.eu")]
    {
        for p in [page(f, period), index(f)] {
            let h = html(&m, &p).await;
            let t = plain(&h);
            assert!(t.contains(&format!("{HOST} relays these values unchanged from {domain}")), "{p}: {t}");
            assert!(t.contains(&format!("{domain} can publish them under its own name:")), "{p}: {t}");
            assert!(has_link(&h, &format!("/takeover/{domain}")), "{p}: a link to the takeover page");
            assert!(!t.contains("Computed by"), "{p}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_computed_page_says_it_is_computed_by_the_mesh() {
    let m = served("sp-computed").await;
    for p in [page(RAIN, "2024"), index(RAIN)] {
        let h = html(&m, &p).await;
        let t = plain(&h);
        assert!(t.contains(&format!("Computed by {HOST} from CHIRPS v3; not CHIRPS's own values.")), "{p}: {t}");
        assert!(!t.contains("relays these values"), "{p}: {t}");
        assert!(
            !anchors(&h).iter().any(|a| path_of(&a.href).starts_with("/takeover/")),
            "{p}: no takeover for a computed series"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn licence_as_of_and_verify() {
    let m = served("sp-meta").await;
    for (f, period, licence, as_of) in [
        (FXR, "2026-03", ECB_LICENCE, "2026-03-03T15:00:00"),
        (BANK, "2026-03", "OGL-UK-3.0", "2026-03-20T12:00:00"),
        (POLICY, "2022", ECB_LICENCE, "2026-02-02T09:00:00"),
        (RAIN, "2024", "CC-BY-4.0", "2026-02-10T08:00:00"),
    ] {
        let h = html(&m, &page(f, period)).await;
        let t = plain(&h);
        assert!(t.contains(licence), "{f}: the licence notice: {t}");
        assert!(t.contains(as_of), "{f}: the as-of, the latest tx of the page's cells: {t}");
        // the verify link: the slice address of the page's interval, and the note on verifying
        assert!(
            anchors(&h).iter().any(|a| {
                let p = path_of(&a.href);
                p.starts_with(&format!("/slice/{f}")) && !p.contains("format=")
            }),
            "{f}: a verify link to the slice address"
        );
        assert!(t.contains("datumesh verify") && t.contains("@datumesh/verify"), "{f}: {t}");
    }
    // the as-of of a month is that month's, not the series'
    assert!(plain(&html(&m, &page(FXR, "2026-01")).await).contains("2026-01-06T15:00:00"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_block_for_agents() {
    let m = served("sp-agents").await;
    for (f, period, keyed) in
        [(FXR, "2026-03", true), (BANK, "2026-03", false), (POLICY, "2022", true), (RAIN, "2024", true)]
    {
        for p in [page(f, period), index(f)] {
            let h = html(&m, &p).await;
            assert_eq!(h.matches("<pre").count(), 1, "{p}: one block for agents");
            let pre = squash(&decode(&strip_tags(&h[h.find("<pre").unwrap()..h.find("</pre>").unwrap()])));
            assert!(pre.contains(&format!("/slice/{f}")), "{p}: the JSON call: {pre}");
            assert!(pre.contains("format=csv") && pre.contains("format=arrow"), "{p}: CSV and Arrow: {pre}");
            assert!(pre.contains("get") && pre.contains("{\"request\":\""), "{p}: the MCP form: {pre}");
            if keyed {
                let key = match f {
                    FXR => "USD",
                    POLICY => "mro",
                    _ => "us_kansas",
                };
                assert!(pre.contains(key), "{p}: an example with a key: {pre}");
            }
        }
    }
}

// ---- rate limit ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_series_budget_is_enforced() {
    let m = serve("sp-rate-series", Some(series_json()), &[("DATUMESH_RATE_SERIES_PER_IP", "3")]).await;
    let p = page(FXR, "2026-03");
    let (mut ok, mut limited) = (0, 0);
    for _ in 0..12 {
        let r = fetch(&m, &p, &[]).await;
        match r.status {
            200 => ok += 1,
            429 => {
                limited += 1;
                assert!(r.header("retry-after").is_some_and(|v| v.parse::<u64>().is_ok()), "Retry-After on a refusal");
            }
            other => panic!("{other}: {}", r.text()),
        }
    }
    assert!((1..=6).contains(&ok), "{ok} served of 12 with a budget of 3 a minute");
    assert!(limited >= 6, "{limited} refused");
    // the budget is the series' own: another route of the same client is not refused by it
    assert_eq!(fetch(&m, &format!("/fn/{FXR}"), &[]).await.status, 200);
    assert_eq!(fetch(&m, &format!("/cell/{FXR}?currency=USD&date=2026-03-02"), &[]).await.status, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_per_ip_limit_applies_to_series_routes_too() {
    let m = serve(
        "sp-rate-ip",
        Some(series_json()),
        &[("DATUMESH_RATE_PER_IP", "2"), ("DATUMESH_RATE_IP_BURST", "2"), ("DATUMESH_RATE_SERIES_PER_IP", "1000")],
    )
    .await;
    let p = page(FXR, "2026-03");
    assert_eq!(fetch(&m, &p, &[]).await.status, 200, "control: the first is served");
    let mut limited = 0;
    for _ in 0..8 {
        if fetch(&m, &p, &[]).await.status == 429 {
            limited += 1;
        }
    }
    assert!(limited >= 4, "{limited} refused of 8 with 2 a minute");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_default_series_budget_is_thirty_a_minute() {
    let m = served("sp-rate-default").await;
    let p = page(FXR, "2026-03");
    for k in 0..25 {
        assert_eq!(fetch(&m, &p, &[]).await.status, 200, "request {k} of 25 is within 30 a minute");
    }
    let mut limited = 0;
    for _ in 0..60 {
        if fetch(&m, &p, &[]).await.status == 429 {
            limited += 1;
        }
    }
    assert!(limited >= 1, "a crawler that misses the cache is refused at some point");
}

// ---- no config, no pages --------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_a_series_file_there_are_no_pages() {
    let with = served("sp-conf-with").await;
    let bare = serve("sp-conf-without", None, &[]).await;
    for p in [page(FXR, "2026-03"), page(FXR, "2026-03.csv"), index(FXR), page(RAIN, "2024"), page(VERSE, "2026-03")] {
        let r = fetch(&bare, &p, &[]).await;
        assert_eq!(r.status, 404, "{p}: {}", r.text());
    }
    assert_eq!(fetch(&bare, "/series/sitemap.xml", &[]).await.status, 404);
    // control: with the file they are there
    assert_eq!(fetch(&with, &page(FXR, "2026-03"), &[]).await.status, 200);
    assert_eq!(fetch(&with, "/series/sitemap.xml", &[]).await.status, 200);
    // /slice is as it was: a browser gets JSON, not a redirect
    let path = format!("/slice/{FXR}?currency=*&date=2026-03-02");
    let r = fetch(&bare, &path, &[("accept", BROWSER)]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["outcome"], "ok", "{}", r.text());
    assert_eq!(r.json(), fetch(&bare, &path, &[]).await.json());
    // apart from the no-store on the answers Accept chose
    let a = fetch(&bare, &path, &[("accept", ARROW_TYPE)]).await;
    assert_eq!(a.status, 200);
    assert_eq!(a.header("content-type").as_deref(), Some(ARROW_TYPE));
    assert_eq!(a.header("cache-control").as_deref(), Some("no-store"));
    // a file that lists no series: no pages either
    let empty = serve("sp-conf-empty", Some("[]".into()), &[]).await;
    assert_eq!(fetch(&empty, &page(FXR, "2026-03"), &[]).await.status, 404);
    assert_eq!(fetch(&empty, &path, &[("accept", BROWSER)]).await.status, 200, "no series, no redirect");
}

// ---- the readers above, on a sample (this one passes before the pages exist) -----------------------------------

#[test]
fn the_html_readers_read_a_sample_page() {
    let h = r#"<!doctype html><html lang="en"><head><title>A &amp; B, 2026-03</title>
<script type="application/ld+json">{"@type":"Dataset","name":"A < B"}</script></head><body>
<h1>A &lt;b&gt; &amp; B <small>2026-03</small></h1>
<p>Excel (Data &gt; From Web) and Sheets (<code>=IMPORTDATA("x")</code>) can import.</p>
<a rel="prev" href="https://mesh-test.datumesh.com/series/f/2026-01">Previous</a>
<a class="n" rel="next nofollow" href='/series/f/2026-11'>Next</a><a href="/fn/f">f</a>
<table><thead><tr><th>date</th><th>USD (units per EUR)</th></tr></thead>
<tbody><tr><td>2026-03-02</td><td>1.0832</td></tr><tr><td>2026-03-03</td><td></td></tr></tbody></table></body></html>"#;
    let (head, rows) = table(h);
    assert_eq!(head, strs(&["date", "USD (units per EUR)"]));
    assert_eq!(rows, grid(&[&["2026-03-02", "1.0832"], &["2026-03-03", ""]]));
    assert_eq!(tag_text(h, "h1"), "A <b> & B 2026-03");
    assert_eq!(tag_text(h, "title"), "A & B, 2026-03");
    assert_eq!(rel(h, "prev").as_deref(), Some("/series/f/2026-01"));
    assert_eq!(rel(h, "next").as_deref(), Some("/series/f/2026-11"));
    assert!(has_link(h, "/fn/f"));
    assert_eq!(ld(h)["name"], "A < B");
    assert!(!plain(h).contains("Dataset"));
    assert!(squash(&plain(h)).contains(&squash("Excel (Data > From Web) and Sheets (=IMPORTDATA(\"x\")) can import.")));
    assert_eq!(csv_lines("a,b\r\n1,\r\n"), strs(&["a,b", "1,"]));
    assert_eq!(decode("&#60;&#x3C;&amp;lt;"), "<<&lt;");
}

// ---- 0.4.6: the call form of the slice address, and the interval call (QUESTIONS-build2 D32) -------------------
//
// * a browser on `/slice/<fn>(<args>)` (percent-encoded or not) is sent to the page like the query form;
// * `GET /x/interval/<function>?<key>=<value|*>...&from=YYYY-MM-DD&to=YYYY-MM-DD[&format=csv|arrow]`: the stored
//   cells of a listed series in `[from, to]`, each with its receipt, JSON by default; CSV (the page's columns
//   and a final `receipt` column holding the hashes of the row's receipts in column order, space-separated) and
//   Arrow are addresses of their own. At most 10 years for a month series (`400` `E_RANGE_TOO_LARGE`),
//   `from <= to`, an unlisted function `404`; cached as the pages are (a past range `s-maxage=86400`, one
//   touching the current period 600); the series' rate-limit bucket.

fn interval(f: &str, query: &str) -> String {
    format!("/x/interval/{f}?{query}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_browser_on_the_call_form_of_the_slice_address_is_sent_to_the_page() {
    let m = served("sp-redirect-call").await;
    let cases = [
        (format!("/slice/{FXR}(currency:%20*,%20date:%202026-03-02)"), page(FXR, "2026-03")),
        (format!("/slice/{FXR}(currency:%20%22USD%22,%20date:%202026-01-06)"), page(FXR, "2026-01")),
        (format!("/slice/{FXR}%28currency%3A%20%2A%2C%20date%3A%202026-03-03%29"), page(FXR, "2026-03")),
        (format!("/slice/{BANK}(date:%202026-05-07)"), page(BANK, "2026-05")),
        (format!("/slice/{POLICY}(rate:%20*,%20date:%202022-07-27)"), page(POLICY, "2022")),
        (format!("/slice/{RAIN}(dataset:%20%22chirps%22,%20region:%20*,%20month:%202024-12-01)"), page(RAIN, "2024")),
        (format!("/slice/{FXR}(currency:%20*,%20date:%20*)"), index(FXR)),
        (format!("/slice/{POLICY}(rate:%20%22mro%22,%20date:%20*)"), index(POLICY)),
    ];
    for accept in ["text/html", BROWSER] {
        for (from, to) in &cases {
            let r = fetch(&m, from, &[("accept", accept)]).await;
            assert!(is_to(&r, to), "{from} ({accept}): {} {:?} {:?}", r.status, r.header("location"), r.text());
            assert_eq!(fetch(&m, to, &[]).await.status, 200, "{to}");
        }
    }
    // JSON clients, other formats and unlisted functions are as they were
    let call = format!("/slice/{FXR}(currency:%20*,%20date:%202026-03-02)");
    let r = fetch(&m, &call, &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = fetch(&m, &format!("{call}?format=csv"), &[("accept", "text/html")]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(r.header("content-type").unwrap_or_default().starts_with("text/csv"));
    let r = fetch(&m, &format!("/slice/{PLAIN}(site:%20*)"), &[("accept", "text/html")]).await;
    assert_ne!(r.status, 303);
    // a call that names no function, or an unknown one, is not redirected
    let r = fetch(&m, "/slice/no.such.host/fn(date:%20*)", &[("accept", "text/html")]).await;
    assert_ne!(r.status, 303);
}

fn members_of(v: &Value) -> Vec<Value> {
    v["members"].as_array().unwrap_or_else(|| panic!("no members in {v}")).clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_interval_call_answers_json_with_receipts() {
    let m = served("sp-interval-json").await;
    let r = fetch(&m, &interval(FXR, "currency=*&from=2026-01-01&to=2026-03-31"), &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(r.header("content-type").unwrap_or_default().starts_with("application/json"));
    assert_eq!(edge(&r), (true, Some(86400)), "{:?}", r.header("cache-control"));
    let v = r.json();
    assert_eq!(v["function"], FXR, "{v}");
    assert_eq!((v["from"].as_str(), v["to"].as_str()), (Some("2026-01-01"), Some("2026-03-31")));
    let ms = members_of(&v);
    assert_eq!(ms.len(), 11, "{v}");
    for x in &ms {
        let date = x["keys"]["date"].as_str().unwrap();
        assert!(("2026-01-01"..="2026-03-31").contains(&date), "{x}");
        assert!(x["receipt"].is_object(), "each member carries its receipt: {x}");
        assert_eq!(x["receipt_hash"].as_str().map(str::len), Some(64), "{x}");
        assert!(x["value"].is_string() && x["tx"].is_string() && x["type"] == "number", "{x}");
    }
    let usd = ms.iter().find(|x| x["keys"]["currency"] == "USD" && x["keys"]["date"] == "2026-03-02").unwrap();
    assert_eq!(usd["value"], "1.0832", "{usd}");
    // a bound key, and an inclusive range of one day
    let v = fetch(&m, &interval(FXR, "currency=USD&from=2026-01-01&to=2026-03-31"), &[]).await.json();
    assert_eq!(members_of(&v).len(), 4, "{v}");
    let v = fetch(&m, &interval(FXR, "currency=USD&from=2026-03-02&to=2026-03-02"), &[]).await.json();
    assert_eq!(members_of(&v).len(), 1, "{v}");
    // an omitted key is open; an unkeyed series has none
    let v = fetch(&m, &interval(FXR, "from=2026-03-01&to=2026-03-31"), &[]).await.json();
    assert_eq!(members_of(&v).len(), 6, "{v}");
    let v = fetch(&m, &interval(BANK, "from=2026-03-01&to=2026-05-31"), &[]).await.json();
    assert_eq!(members_of(&v).len(), 3, "{v}");
    // fixed arguments of the series file apply
    let v = fetch(&m, &interval(RAIN, "region=*&from=2024-01-01&to=2025-12-31"), &[]).await.json();
    assert_eq!(members_of(&v).len(), 5, "{v}");
    // an empty range is an empty list, not an error
    let r = fetch(&m, &interval(BANK, "from=2025-01-01&to=2025-02-01"), &[]).await;
    assert_eq!(r.status, 200);
    assert!(members_of(&r.json()).is_empty());
    // a browser's Accept does not refuse it
    let r = fetch(&m, &interval(BANK, "from=2026-03-01&to=2026-05-31"), &[("accept", BROWSER)]).await;
    assert_eq!(r.status, 200, "{}", r.text());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_interval_call_is_cached_like_the_pages() {
    let m = served("sp-interval-cache").await;
    let past = fetch(&m, &interval(FXR, "currency=*&from=2026-01-01&to=2026-03-31"), &[]).await;
    assert_eq!(edge(&past), (true, Some(86400)));
    let now = fetch(&m, &interval(FXR, "currency=*&from=2026-11-01&to=2026-11-30"), &[]).await;
    assert_eq!(now.status, 200, "{}", now.text());
    assert_eq!(edge(&now), (true, Some(600)), "{:?}", now.header("cache-control"));
    let wide = fetch(&m, &interval(FXR, "currency=*&from=2026-03-01&to=2026-12-31"), &[]).await;
    assert_eq!(edge(&wide), (true, Some(600)), "a range that touches the current month");
    let tag = past.header("etag").expect("an ETag");
    let again = fetch(&m, &interval(FXR, "currency=*&from=2026-01-01&to=2026-03-31"), &[("if-none-match", &tag)]).await;
    assert_eq!(again.status, 304);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_interval_call_in_csv_and_arrow() {
    let m = served("sp-interval-formats").await;
    let r = fetch(&m, &interval(BANK, "from=2026-03-01&to=2026-05-31&format=csv"), &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(r.header("content-type").unwrap_or_default().starts_with("text/csv"));
    let lines = csv_lines(&r.text());
    assert_eq!(lines[0], "date,value (percent),receipt");
    assert_eq!(lines.len(), 4, "{lines:?}");
    let hash = |l: &str| l.rsplit(',').next().unwrap().to_string();
    assert!(lines[1].starts_with("2026-03-19,4.5,") && hash(&lines[1]).len() == 64, "{}", lines[1]);
    assert!(lines[3].starts_with("2026-05-07,4.25,"), "{}", lines[3]);
    // a keyed series: the page's columns, then the receipts of the row's cells in column order
    let r = fetch(&m, &interval(FXR, "currency=*&from=2026-03-01&to=2026-03-31&format=csv"), &[]).await;
    let lines = csv_lines(&r.text());
    assert_eq!(lines[0], "date,USD (units per EUR),JPY (units per EUR),GBP (units per EUR),receipt");
    assert!(lines[1].starts_with("2026-03-02,1.0832,160.25,0.8301,"), "{}", lines[1]);
    assert_eq!(hash(&lines[1]).split(' ').filter(|h| h.len() == 64).count(), 3, "{}", lines[1]);
    // the same cache as the JSON, and an address of its own
    assert_eq!(edge(&r), (true, Some(86400)));
    let r = fetch(&m, &interval(BANK, "from=2026-03-01&to=2026-05-31&format=arrow"), &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.header("content-type").as_deref(), Some(ARROW_TYPE));
    assert!(r.body.len() > 8 && r.body[..4] == [0xff, 0xff, 0xff, 0xff], "an Arrow IPC stream");
    assert_eq!(edge(&r), (true, Some(86400)));
    let r = fetch(&m, &interval(BANK, "from=2026-03-01&to=2026-05-31&format=xml"), &[]).await;
    assert_eq!(r.status, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_interval_call_refuses_what_it_cannot_take() {
    let m = served("sp-interval-errors").await;
    let status = |q: &str| {
        let m = &m;
        let q = q.to_string();
        async move { fetch(m, &interval(BANK, &q), &[]).await }
    };
    assert_eq!(status("from=2026-05-01&to=2026-03-01").await.status, 400, "from after to");
    assert_eq!(status("to=2026-03-01").await.status, 400, "no from");
    assert_eq!(status("from=2026-03-01").await.status, 400, "no to");
    assert_eq!(status("from=2026-3-1&to=2026-03-02").await.status, 400, "not a date");
    assert_eq!(status("from=2026-02-30&to=2026-03-02").await.status, 400, "not a day");
    assert_eq!(status("from=2026-03-01&to=2026-03-02&colour=red").await.status, 400, "no such argument");
    assert_eq!(status("date=2026-03-01&from=2026-03-01&to=2026-03-02").await.status, 400, "the date is from and to");
    // ten years of a month series is the most
    let r = status("from=2016-01-01&to=2026-01-01").await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = status("from=2015-12-31&to=2026-01-01").await;
    assert_eq!(r.status, 400, "{}", r.text());
    assert!(r.text().contains("E_RANGE_TOO_LARGE"), "{}", r.text());
    // a year series has no such limit
    let r = fetch(&m, &interval(POLICY, "rate=*&from=1990-01-01&to=2026-12-31"), &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(members_of(&r.json()).len(), 9);
    // a function no series lists, one that does not exist, and no series file at all
    assert_eq!(fetch(&m, &interval(PLAIN, "site=*&from=2026-01-01&to=2026-02-01"), &[]).await.status, 404);
    assert_eq!(fetch(&m, &interval("no.such.host/fn", "from=2026-01-01&to=2026-02-01"), &[]).await.status, 404);
    let bare = serve("sp-interval-bare", None, &[]).await;
    assert_eq!(fetch(&bare, &interval(BANK, "from=2026-01-01&to=2026-02-01"), &[]).await.status, 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_interval_call_takes_the_series_budget() {
    let m = serve("sp-interval-rate", Some(series_json()), &[("DATUMESH_RATE_SERIES_PER_IP", "3")]).await;
    let q = interval(BANK, "from=2026-03-01&to=2026-05-31");
    let (mut ok, mut limited) = (0, 0);
    for _ in 0..12 {
        match fetch(&m, &q, &[]).await.status {
            200 => ok += 1,
            429 => limited += 1,
            other => panic!("{other}"),
        }
    }
    assert!((1..=6).contains(&ok) && limited >= 6, "{ok} served, {limited} refused");
    // the budget is shared with the pages
    assert_eq!(fetch(&m, &page(BANK, "2026-03"), &[]).await.status, 429);
    assert_eq!(fetch(&m, &format!("/fn/{BANK}"), &[]).await.status, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mesh_document_and_openapi_list_the_interval_call() {
    let m = served("sp-interval-doc").await;
    let md = fetch(&m, "/.well-known/datumesh-mesh.json", &[]).await.json();
    assert_eq!(md["endpoints"]["x_interval"], format!("https://{HOST}/x/interval/{{function}}"), "{md}");
    assert_eq!(md["endpoints"]["x_calc"], format!("https://{HOST}/x/calc"), "{md}");
    let api = fetch(&m, "/openapi.json", &[]).await.json();
    let paths = api["paths"].as_object().unwrap();
    let p = paths.iter().find(|(k, _)| k.starts_with("/x/interval")).unwrap_or_else(|| panic!("{}", api["paths"])).1;
    assert!(p.get("get").is_some() && p.get("post").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_block_for_agents_gives_the_interval_call() {
    let m = served("sp-agents-interval").await;
    let pre_of = |h: &str| squash(&decode(&strip_tags(&h[h.find("<pre").unwrap()..h.find("</pre>").unwrap()])));
    let pre = pre_of(&html(&m, &page(FXR, "2026-03")).await);
    let base = format!("/x/interval/{FXR}?currency=USD&from=2026-03-01&to=2026-03-31");
    assert!(pre.contains(&base), "the JSON interval call of the page: {pre}");
    assert!(pre.contains(&format!("{base}&format=csv")) && pre.contains(&format!("{base}&format=arrow")), "{pre}");
    assert!(pre.contains("Anyinterval"), "says it takes any interval (whitespace is squashed): {pre}");
    // the old lines stay
    assert!(pre.contains(&format!("/slice/{FXR}")) && pre.contains("format=csv") && pre.contains("{\"request\":\""));
    let pre = pre_of(&html(&m, &page(BANK, "2026-03")).await);
    assert!(pre.contains(&format!("/x/interval/{BANK}?from=2026-03-01&to=2026-03-31")), "{pre}");
    // a year page: the year
    let pre = pre_of(&html(&m, &page(POLICY, "2022")).await);
    assert!(pre.contains("from=2022-01-01&to=2022-12-31"), "{pre}");
    // the index: from the first period to the last
    let pre = pre_of(&html(&m, &index(BANK)).await);
    assert!(pre.contains(&format!("/x/interval/{BANK}?from=2026-03-01&to=2026-05-31")), "{pre}");
}
