//! The guard patch, P2 (CONTRACT-guard-patch.md): the mesh's own pages at phone width. `/fn/...`, `/series/...`, the
//! catalogue and the takeover page have no horizontal scroll at 390 and 320 px: long words break, and `pre` scrolls in its
//! own box.
//!
//! Two kinds of test:
//!
//! * **Fast, always run:** the `<style>` of every page, read as CSS rules. The pinned form, for each of the four kinds of
//!   page:
//!   - a rule for `body` sets `overflow-wrap:anywhere` (so a long hash, name, URL or call breaks anywhere, inline code
//!     included, by inheritance);
//!   - no rule for `code` sets an `overflow-wrap` other than `anywhere`, nor `white-space:nowrap` or `pre` (inline code
//!     keeps breaking);
//!   - the rules for `pre` set `overflow-x:auto` and `max-width:100%` (a block of code scrolls in its own box).
//! * **The browser, `phone_pages_have_no_horizontal_scroll`:** skipped (it passes with a line on stderr) unless
//!   `DATUMESH_MESH_BROWSER=1`; it needs Docker and the local image `zenika/alpine-chrome:124`. The technique of the site's
//!   `tests/test_browser.py`: every page is fetched from a mesh served in process, a probe script is added before
//!   `</body>`, and the pages are served from a small loopback server; headless Chromium loads each at 390x844 and at
//!   320x700 with `--dump-dom`, and the probe writes the document's `scrollWidth` and the viewport's `clientWidth` into the
//!   DOM. The test asserts `scrollWidth <= clientWidth` (scrollbars hidden, so the viewport is the window width) and
//!   `clientWidth <= width`, for every page at both sizes, and lists every failure at once.
//!
//!   The Chromium container reaches the loopback server by the network `DATUMESH_BROWSER_NETWORK` when it is set; else,
//!   when the test itself runs in a container (`/.dockerenv`), by that container's network namespace
//!   (`container:<hostname>`); else by `host`. Inside `rust:1.99` the Docker client and socket are mounted:
//!   `-v /usr/bin/docker:/usr/bin/docker:ro -v /var/run/docker.sock:/var/run/docker.sock`.
//!
//! The fixtures, chosen for long words:
//!
//! * the function pages: a computed function under a long namespace, its tests and an example (long calls, the 64-hex
//!   `fn_hash`, the binding history), the stored function it reads, and a relayed function (a long source URL, a sha256,
//!   the takeover link); the takeover page of the relayed function's domain;
//! * the series pages: a keyed daily relay with a long name, a month page (its `=IMPORTDATA("<long URL>")` line) and the
//!   index;
//! * the catalogue: world S of the catalogue tests, the job run: `/catalog`, `/catalog/rule`, `/catalog/finance` and the
//!   first subject page it links to.

#[path = "common/calculus.rs"]
mod calculus;
#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/publish4.rs"]
mod publish4;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::lite::{Lite, HOST, NOW};
use common::pub031::{function_line, run, write};
use common::readpath::{build_served, catalog_line, serve_lite, Served};
use datumesh_http::Config;
use serde_json::{json, Value};

// ---- the fixtures ---------------------------------------------------------------------------------------------------

/// A long namespace: the computed function's page shows calls of it in its tests and examples.
const PREFIX: &str =
    "mesh-test.datumesh.com/scratch_p2/phone_width_measurements_of_rather_long_namespaces/per_site_readings";
const RELAY_DOMAIN: &str = "ecb.europa.eu";
const RELAY: &str =
    "mesh-test.datumesh.com/scratch_p2/relay/ecb.europa.eu/euro_foreign_exchange_reference_rate_of_the_day";
const SERIES: &str =
    "mesh-test.datumesh.com/relay/ecb.europa.eu/euro_foreign_exchange_reference_rates_against_the_euro";
const BOUND_CLOCK: &str = "2026-01-01T00:00:00.000000Z";

fn relay_source() -> Value {
    json!({"name": RELAY_DOMAIN,
        "url": "https://www.ecb.europa.eu/stats/policy_and_exchange_rates/euro_reference_exchange_rates/html/eurofxref-graph-usd.en.html?download=eurofxref-hist-complete-with-all-currencies.zip",
        "retrieved": "2026-11-15T10:00:00.000000Z",
        "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "takeover": format!("https://{HOST}/takeover/{RELAY_DOMAIN}")})
}

/// The function pages' world: the computed function `per_unit` (tests, an example), the stored `reading`, the relayed
/// function.
fn fn_lines() -> Vec<String> {
    let c = calculus::Calc::new(PREFIX);
    let mut v = calculus::data_lines(&c);
    v.push(calculus::per_unit_line(&c, None));
    v.extend(calculus::per_unit_tests(&c));
    let ex = json!([{"call": format!("{}(site: \"N\", units: 2)", c.per_unit), "expected": {"type": "number", "value": "5"}}]);
    v.push(
        json!({"catalog": c.per_unit, "kind": "function", "label": "Per unit",
            "description": "Divides the reading at a site by a number of units: https://mesh-test.datumesh.com/scratch_p2/a_link_that_is_one_long_word_of_well_over_three_hundred_and_twenty_pixels",
            "args": {"site": "the measuring site", "units": "units to divide by"}, "examples": ex, "dictionary": {}})
        .to_string(),
    );
    v.push(catalog_line(
        &c.reading,
        "function",
        "Reading",
        "The raw reading at a site.",
        json!({"site": "the site"}),
        json!({}),
    ));
    v.push(function_line(RELAY, &calculus::reading_sig(), None));
    let mut cell: Value = serde_json::from_str(&calculus::site_cell(RELAY, "N", "1.0892")).unwrap();
    cell["source"] = relay_source();
    v.push(cell.to_string());
    v.push(catalog_line(
        RELAY,
        "function",
        "Euro reference rate",
        "A relayed rate.",
        json!({"site": "the site"}),
        json!({}),
    ));
    v
}

async fn fn_world(label: &str) -> (Served, Vec<String>) {
    let m = serve_lite(label, &fn_lines(), None, |_| {}).await;
    let c = calculus::Calc::new(PREFIX);
    let paths = vec![
        format!("/fn/{}", c.per_unit),
        format!("/fn/{}", c.reading),
        format!("/fn/{RELAY}"),
        format!("/takeover/{RELAY_DOMAIN}"),
    ];
    (m, paths)
}

fn series_sig() -> String {
    r#"{"args":{"currency":{"type":"string","valid":false},"date":{"type":"date","valid":true}},"order":["currency","date"],"returns":{"type":"number","unit":"1"}}"#.to_string()
}

fn series_cell(cur: &str, date: &str, value: &str) -> String {
    let tx = format!("{date}T15:00:00.000000Z");
    json!({"cell": {"fn": SERIES, "args": {"currency": {"type": "string", "value": cur}, "date": {"type": "date", "value": date}}},
        "licence": "CC-BY-4.0", "summary": "Synthetic test data.", "value": {"type": "number", "value": value},
        "tx": tx, "asof": tx, "issued": tx,
        "source": {"name": RELAY_DOMAIN, "url": format!("https://{RELAY_DOMAIN}/data"), "retrieved": "2026-01-01T00:00:00.000000Z",
            "takeover": format!("https://{HOST}/takeover/{RELAY_DOMAIN}")}})
    .to_string()
}

async fn series_world(label: &str) -> (Served, Vec<String>) {
    let lite = Lite::new(label);
    let f0 = write(&lite, "functions.jsonl", &[function_line(SERIES, &series_sig(), None)]);
    run(&lite, &f0, BOUND_CLOCK).unwrap_or_else(|e| panic!("functions refused: {e}"));
    let mut lines = Vec::new();
    for (date, cells) in [
        ("2026-01-05", vec![("USD", "1.0412"), ("JPY", "163.45"), ("GBP", "0.8567")]),
        ("2026-01-06", vec![("USD", "1.045"), ("JPY", "164.1"), ("GBP", "0.8571")]),
        ("2026-11-13", vec![("USD", "1.0892")]),
    ] {
        for (cur, val) in cells {
            lines.push(series_cell(cur, date, val));
        }
    }
    let dict: Vec<Value> = [("USD", "US dollar"), ("JPY", "Japanese yen"), ("GBP", "Pound sterling")]
        .iter()
        .map(|(v, l)| json!({"value": v, "label": l}))
        .collect();
    lines.push(catalog_line(
        SERIES,
        "function",
        "Euro reference rate",
        "The euro foreign exchange reference rates, relayed from the ECB.",
        json!({"currency": "the currency", "date": "the day"}),
        json!({"currency": dict}),
    ));
    let f = write(&lite, "fixture.jsonl", &lines);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("fixture refused: {e}"));
    let p = lite.path("series.json");
    std::fs::write(
        &p,
        json!([{"function": SERIES, "granularity": "month", "date_arg": "date", "column_arg": "currency",
            "unit": "units per EUR", "kind": "relayed"}])
        .to_string(),
    )
    .unwrap();
    let file = p.to_string_lossy().into_owned();
    let cfg = Config::from_vars(move |k| match k {
        "DATUMESH_HOST" => Some(HOST.to_string()),
        "DATUMESH_SERIES_FILE" => Some(file.clone()),
        _ => None,
    })
    .unwrap();
    let m = build_served(lite, Some(cfg), |_| {}).await;
    let paths = vec![format!("/series/{SERIES}/2026-01"), format!("/series/{SERIES}/")];
    // the index waits for the background build of the overview (503 until then)
    for _ in 0..300 {
        if fetch(&m, &paths[1]).await.0 != 503 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    (m, paths)
}

async fn catalogue_world(label: &str) -> (Served, Vec<String>) {
    let lite = catworld::scratch(label);
    catjob::build(&lite, catworld::T3);
    let m = catworld::serve(lite, &[]).await;
    catworld::until_built(&m, "/catalog").await;
    let mut paths = vec!["/catalog".to_string(), "/catalog/rule".to_string(), "/catalog/finance".to_string()];
    let (_, domain) = fetch(&m, "/catalog/finance").await;
    let subject = hrefs(&domain)
        .into_iter()
        .map(|h| h.strip_prefix(&format!("https://{HOST}")).map(str::to_string).unwrap_or(h))
        .find(|h| h.starts_with("/catalog/finance/subject?key="))
        .unwrap_or_else(|| panic!("no subject link on /catalog/finance: {domain}"));
    paths.push(subject.replace("&amp;", "&"));
    (m, paths)
}

/// GET `path` as a browser asks for it: status and text.
async fn fetch(m: &Served, path: &str) -> (u16, String) {
    let r = publish4::get(m, path, &[("accept", "text/html,application/xhtml+xml,*/*;q=0.8")]).await;
    (r.status, r.text())
}

/// GET `path`: 200 and HTML; the text.
async fn html(m: &Served, path: &str) -> String {
    let r = publish4::get(m, path, &[("accept", "text/html,application/xhtml+xml,*/*;q=0.8")]).await;
    assert_eq!(r.status, 200, "{path}: {}", r.text());
    let ct = r.header("content-type").unwrap_or_default();
    assert!(ct.starts_with("text/html"), "{path}: content-type {ct:?}");
    r.text()
}

fn hrefs(h: &str) -> Vec<String> {
    h.split("href=\"").skip(1).filter_map(|s| s.split('"').next()).map(str::to_string).collect()
}

// ---- reading the CSS ------------------------------------------------------------------------------------------------

/// A CSS rule: its selectors (trimmed, lower case) and its declarations (property and value, trimmed, lower case,
/// `!important` dropped), in order.
struct Rule {
    selectors: Vec<String>,
    decls: Vec<(String, String)>,
}

/// The text of every `<style>` element of a page, in order.
fn styles(h: &str) -> String {
    let mut out = String::new();
    let mut rest = h;
    while let Some(at) = rest.find("<style") {
        let after = &rest[at..];
        let Some(open) = after.find('>') else { break };
        let body = &after[open + 1..];
        let end = body.find("</style>").unwrap_or(body.len());
        out.push_str(&body[..end]);
        out.push('\n');
        rest = &body[end..];
    }
    out
}

/// The style rules of a stylesheet; the rules inside an at-rule block (`@media ...{...}`) count as rules too.
fn rules(css: &str) -> Vec<Rule> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut stack: Vec<Option<String>> = Vec::new(); // None: an at-rule block; Some(selector): a style rule
    let mut comment = false;
    let cs: Vec<char> = css.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if comment {
            if c == '*' && cs.get(i + 1) == Some(&'/') {
                comment = false;
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == '/' && cs.get(i + 1) == Some(&'*') {
            comment = true;
            i += 2;
            continue;
        }
        match c {
            '{' => {
                let s = buf.trim().to_string();
                stack.push(if s.starts_with('@') { None } else { Some(s) });
                buf.clear();
            }
            '}' => {
                if let Some(Some(sel)) = stack.pop() {
                    let decls = buf
                        .split(';')
                        .filter_map(|d| d.split_once(':'))
                        .map(|(p, v)| (p.trim().to_lowercase(), v.replace("!important", "").trim().to_lowercase()))
                        .collect();
                    out.push(Rule {
                        selectors: sel
                            .split(',')
                            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase())
                            .collect(),
                        decls,
                    });
                }
                buf.clear();
            }
            _ => buf.push(c),
        }
        i += 1;
    }
    out
}

/// The declarations, in order, of the rules whose selector list holds `sel` itself.
fn decls_for<'a>(rs: &'a [Rule], sel: &str) -> Vec<&'a (String, String)> {
    rs.iter().filter(|r| r.selectors.iter().any(|s| s == sel)).flat_map(|r| r.decls.iter()).collect()
}

/// The last value given to `prop` by the rules for `sel`.
fn last<'a>(rs: &'a [Rule], sel: &str, prop: &str) -> Option<&'a str> {
    decls_for(rs, sel).into_iter().rfind(|(p, _)| p == prop).map(|(_, v)| v.as_str())
}

/// What is wrong with the page's CSS, by the rules of this file's header (empty: nothing).
fn css_faults(path: &str, page: &str) -> Vec<String> {
    let css = styles(page);
    let rs = rules(&css);
    let mut f = Vec::new();
    if css.trim().is_empty() {
        f.push(format!("{path}: no <style>"));
        return f;
    }
    if last(&rs, "body", "overflow-wrap") != Some("anywhere") {
        f.push(format!("{path}: body has no overflow-wrap:anywhere"));
    }
    if let Some(v) = last(&rs, "code", "overflow-wrap") {
        if v != "anywhere" {
            f.push(format!("{path}: code sets overflow-wrap:{v}"));
        }
    }
    if let Some(v) = last(&rs, "code", "white-space") {
        if v == "nowrap" || v == "pre" {
            f.push(format!("{path}: code sets white-space:{v}, so a long inline word cannot break"));
        }
    }
    if last(&rs, "pre", "overflow-x") != Some("auto") {
        f.push(format!("{path}: pre has no overflow-x:auto"));
    }
    if last(&rs, "pre", "max-width") != Some("100%") {
        f.push(format!("{path}: pre has no max-width:100%"));
    }
    f
}

async fn assert_css(m: &Served, paths: &[String]) {
    let mut faults = Vec::new();
    for p in paths {
        let h = html(m, p).await;
        let f = css_faults(p, &h);
        if !f.is_empty() {
            faults.extend(f);
            faults.push(format!("  (the style of {p}: {})", styles(&h).trim()));
        }
    }
    assert!(faults.is_empty(), "the CSS faults:\n{}", faults.join("\n"));
}

/// The CSS reader itself, on known input.
#[test]
fn the_css_reader_reads_rules() {
    let page = "<style>body{font:16px/1.5 x;OVERFLOW-WRAP: anywhere !important}code,pre{overflow-x:auto}\
                @media (max-width:40rem){pre{max-width:100%}} /* a comment {x} */ main{max-width:52rem}</style>";
    let rs = rules(&styles(page));
    assert_eq!(last(&rs, "body", "overflow-wrap"), Some("anywhere"));
    assert_eq!(last(&rs, "pre", "overflow-x"), Some("auto"));
    assert_eq!(last(&rs, "pre", "max-width"), Some("100%"));
    assert_eq!(last(&rs, "main", "max-width"), Some("52rem"));
    assert_eq!(last(&rs, "code", "max-width"), None);
    assert!(css_faults("/x", page).is_empty(), "{:?}", css_faults("/x", page));
    // today's function page style: every rule but the pre box's overflow-x fails
    let today = "<style>body{font:16px/1.5 system-ui,sans-serif;margin:0;padding:16px;color:#1d1d1f;background:#fff}main{max-width:52rem;margin:0 auto}code,pre{font-family:ui-monospace,monospace;font-size:14px;overflow-x:auto}</style>";
    assert_eq!(css_faults("/fn/x", today).len(), 2, "{:?}", css_faults("/fn/x", today));
    let nowrap =
        "<style>body{overflow-wrap:anywhere}code{white-space:nowrap}pre{overflow-x:auto;max-width:100%}</style>";
    assert_eq!(css_faults("/fn/x", nowrap).len(), 1);
}

// ---- the fast tests: the CSS of each kind of page -------------------------------------------------------------------

/// The function pages (computed, stored, relayed) and the takeover page.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn function_and_takeover_pages_break_long_words_and_box_pre() {
    let (m, paths) = fn_world("p2-css-fn").await;
    assert_css(&m, &paths).await;
}

/// A series month page and the series index.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn series_pages_break_long_words_and_box_pre() {
    let (m, paths) = series_world("p2-css-series").await;
    assert_css(&m, &paths).await;
}

/// The catalogue: its root, the rule, a domain and a subject.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catalogue_pages_break_long_words_and_box_pre() {
    let (m, paths) = catalogue_world("p2-css-cat").await;
    assert_css(&m, &paths).await;
}

// ---- the browser ----------------------------------------------------------------------------------------------------

const IMAGE: &str = "zenika/alpine-chrome:124";
const SIZES: [(u32, u32); 2] = [(390, 844), (320, 700)];
const PROBE: &str = "<script>(function(){var d=document.documentElement;\
var r={sw:d.scrollWidth,cw:d.clientWidth,iw:window.innerWidth,bw:document.body.scrollWidth};\
var p=document.createElement('pre');p.id='overflow-probe';p.textContent='PROBE'+JSON.stringify(r)+'EBORP';\
document.body.appendChild(p);})();</script>";

/// The page with the probe at the end of its body.
fn probed(h: &str) -> String {
    match h.rfind("</body>") {
        Some(at) => format!("{}{PROBE}{}", &h[..at], &h[at..]),
        None => format!("{h}{PROBE}"),
    }
}

/// Serves `pages` (`/p/<i>` -> HTML) on 127.0.0.1 until the task is aborted; the port.
async fn serve_pages(pages: Arc<BTreeMap<String, String>>) -> (u16, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { continue };
            let pages = pages.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let mut got = 0;
                while got < buf.len() {
                    match s.read(&mut buf[got..]).await {
                        Ok(0) | Err(_) => break,
                        Ok(k) => got += k,
                    }
                    if buf[..got].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let req = String::from_utf8_lossy(&buf[..got]).into_owned();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, body) = match pages.get(&path) {
                    Some(b) => ("200 OK", b.clone()),
                    None => ("404 Not Found", "<!doctype html><title>404</title>".to_string()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes()).await;
                let _ = s.write_all(body.as_bytes()).await;
                let _ = s.shutdown().await;
            });
        }
    });
    (port, task)
}

/// The network the Chromium container joins to reach this process's loopback.
fn browser_network() -> String {
    if let Ok(n) = std::env::var("DATUMESH_BROWSER_NETWORK") {
        if !n.is_empty() {
            return n;
        }
    }
    if std::path::Path::new("/.dockerenv").exists() {
        let host = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
        return format!("container:{}", host.trim());
    }
    "host".to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn phone_pages_have_no_horizontal_scroll() {
    if std::env::var("DATUMESH_MESH_BROWSER").as_deref() != Ok("1") {
        eprintln!("skipped: set DATUMESH_MESH_BROWSER=1 (needs Docker and {IMAGE})");
        return;
    }
    // Every page of the three worlds, fetched and probed: (the page's mesh path, its HTML).
    let mut fetched: Vec<(String, String)> = Vec::new();
    for (m, paths) in
        [fn_world("p2-br-fn").await, series_world("p2-br-series").await, catalogue_world("p2-br-cat").await]
    {
        for p in paths {
            let h = html(&m, &p).await;
            fetched.push((p, probed(&h)));
        }
    }
    assert!(fetched.len() >= 10, "{} pages", fetched.len());
    let pages: BTreeMap<String, String> =
        fetched.iter().enumerate().map(|(i, (_, h))| (format!("/p/{i}"), h.clone())).collect();
    let (port, server) = serve_pages(Arc::new(pages)).await;

    let mut script = Vec::new();
    for (w, h) in SIZES {
        for i in 0..fetched.len() {
            script.push(format!(
                "echo 'BEGIN {w} {i}'; chromium-browser --headless --no-sandbox --disable-gpu --hide-scrollbars \
                 --window-size={w},{h} --virtual-time-budget=3000 --dump-dom 'http://127.0.0.1:{port}/p/{i}' 2>/dev/null; echo 'END'"
            ));
        }
    }
    let network = browser_network();
    let script = script.join("\n");
    let net = network.clone();
    let out = tokio::task::spawn_blocking(move || {
        std::process::Command::new("docker")
            .args(["run", "--rm", "--network", &net, "--entrypoint", "sh", IMAGE, "-c", &script])
            .output()
    })
    .await
    .unwrap()
    .unwrap_or_else(|e| panic!("docker: {e} (the Docker client is needed: see the header of this file)"));
    server.abort();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();

    // (width, page index) -> (scrollWidth, clientWidth)
    let mut got: BTreeMap<(u32, usize), (u64, u64)> = BTreeMap::new();
    for block in stdout.split("BEGIN ").skip(1) {
        let mut head = block.split_whitespace();
        let (Some(w), Some(i)) = (head.next().and_then(|x| x.parse().ok()), head.next().and_then(|x| x.parse().ok()))
        else {
            continue;
        };
        // the probe's own script holds `'PROBE'+...`: the measurement is the `PROBE{...}EBORP` the script wrote
        let Some(probe) =
            block.split("PROBE{").nth(1).and_then(|r| r.split("}EBORP").next()).map(|j| format!("{{{j}}}"))
        else {
            continue;
        };
        // the DOM dump escapes nothing in a <pre>'s JSON but quotes
        let v: Value = serde_json::from_str(&probe.replace("&quot;", "\"")).unwrap_or(Value::Null);
        if let (Some(sw), Some(cw)) = (v["sw"].as_u64(), v["cw"].as_u64()) {
            got.insert((w, i), (sw, cw));
        }
    }
    let mut faults = Vec::new();
    let mut report = String::new();
    for (w, _) in SIZES {
        for (i, (path, _)) in fetched.iter().enumerate() {
            match got.get(&(w, i)) {
                None => faults.push(format!("{path} at {w}px: no measurement")),
                Some(&(sw, cw)) => {
                    report.push_str(&format!("{w}px {sw}/{cw} {path}\n"));
                    if sw > cw || cw > u64::from(w) {
                        faults.push(format!("{path} at {w}px is {sw}px wide (viewport {cw}px)"));
                    }
                }
            }
        }
    }
    let _ = std::fs::write(common::out_dir().join("phone-width.txt"), &report);
    eprint!("{report}");
    assert!(
        faults.is_empty(),
        "{} of {} measurements fail (network {network}; docker exit {:?}, stderr {:?}):\n{}",
        faults.len(),
        SIZES.len() * fetched.len(),
        out.status.code(),
        stderr.chars().rev().take(400).collect::<Vec<_>>().into_iter().rev().collect::<String>(),
        faults.join("\n")
    );
}
