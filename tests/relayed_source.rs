//! Build 2 Part 5, F1/F5, the http needs: a relayed value.
//!
//! * `publish-stored` accepts a `source` object on a stored cell line (spec-0.6.0 C2): `name`, `url`,
//!   `retrieved`, optional `sha256` and `takeover`; the receipt signs it in (`receipt.source`); the
//!   envelope's trust is `relayed`; the receipt verifies with datumesh-core 0.3.9 (`check_answer`,
//!   `verify`), and a changed `source` breaks the signature;
//! * refused, nothing written: `retrieved` later than the cell's `issued`; a `source.name` that is
//!   the publisher's own host; a `source` with a member the spec does not know, or without `url`, or a
//!   takeover that is not `https://`;
//! * a relayed function's `describe` answers `source` {`name`, `relayed_by` (this mesh's host),
//!   `takeover`}, core 0.3.9 accepts it; its function page (HTML and JSON) names the source, "relayed by
//!   <mesh host>" and the takeover page; a function that is not relayed has none of these.

mod common;
#[path = "common/calculus.rs"]
mod calculus;
#[path = "common/publish4.rs"]
mod publish4;

use calculus::*;
use common::lite::*;
use common::pub031::*;
use common::readpath::*;
use common::world::call_path;
use publish4::get;
use serde_json::{json, Value};

const FX: &str = "mesh-test.datumesh.com/scratch_b5/relay/fx";
const OWN: &str = "mesh-test.datumesh.com/scratch_b5/own/level";
const TAKEOVER: &str = "https://mesh-test.datumesh.com/takeover/ecb.europa.eu";

fn source() -> Value {
    json!({"name": "ecb.europa.eu",
        "url": "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-hist.zip",
        "retrieved": "2026-11-15T10:00:00.000000Z",
        "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "takeover": TAKEOVER})
}

fn relayed(site: &str, value: &str, src: Value) -> String {
    let mut v: Value = serde_json::from_str(&site_cell(FX, site, value)).unwrap();
    v["source"] = src;
    v.to_string()
}

fn with_source(f: impl FnOnce(&mut Value)) -> Value {
    let mut s = source();
    f(&mut s);
    s
}

fn lines() -> Vec<String> {
    vec![
        function_line(FX, &reading_sig(), None),
        relayed("N", "1.0892", source()),
        function_line(OWN, &reading_sig(), None),
        site_cell(OWN, "N", "7"),
        catalog_line(FX, "function", "Euro reference rate", "A reference rate relayed from the ECB.", json!({"site": "the site"}), json!({})),
    ]
}

fn core_keys() -> datumesh_core::Keys {
    let mut k = datumesh_core::Keys::new();
    k.insert_response(HOST, 200, Some(&datumesh_http::keydoc::with_key(HOST, None, &common::key_for(HOST), K_FROM).unwrap()));
    k
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_relayed_cell_is_signed_with_its_source_and_trusted_as_relayed() {
    let m = serve_lite("rs-cell", &lines(), None, |_| {}).await;
    let r = m.get_any(&format!("/cell/{FX}?site=N")).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json["outcome"], "ok", "{}", r.json);
    assert_eq!(r.json["trust"], "relayed", "{}", r.json);
    assert_eq!(r.json["receipt"]["source"], source(), "{}", r.json);
    // core 0.3.9 checks the answer: the source is covered by the signature, the trust is `relayed`
    let cell = cell_json(&m, FX, &[("site", "string", "N")]);
    let req = datumesh_core::Request::new(&core_id(&cell), None).unwrap();
    let a = datumesh_core::check_answer(r.status, &r.body, &core_keys(), &req).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(a.outcome.as_str(), "ok");
    // the receipt by itself verifies; a changed source does not
    let receipt = serde_json::to_vec(&r.json["receipt"]).unwrap();
    let v = datumesh_core::verify(&receipt, &core_keys(), None, None);
    assert!(v.ok, "{:?}", v.error());
    let mut forged = r.json["receipt"].clone();
    forged["source"]["url"] = json!("https://www.example.org/other.zip");
    let v = datumesh_core::verify(&serde_json::to_vec(&forged).unwrap(), &core_keys(), None, None);
    assert!(!v.ok, "a changed source verifies");
    // the receipt is served byte for byte as signed
    let hash = r.json["receipt"]["hash"].as_str().map(str::to_string).unwrap_or_else(|| {
        datumesh_core::Receipt::from_json(&receipt).unwrap().hash().to_string()
    });
    let again = m.get_any(&format!("/receipt/{hash}")).await;
    assert_eq!(again.status, 200);
    assert_eq!(again.json["source"], source());
    // a cell without a source stays primary
    let own = m.get(&format!("/cell/{OWN}?site=N")).await;
    assert_eq!(own["trust"], "primary", "{own}");
    assert!(own["receipt"].get("source").is_none());
}

/// The source of each member optional: only name, url and retrieved are required.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_optional_members_are_optional() {
    let src = json!({"name": "ecb.europa.eu", "url": "https://www.ecb.europa.eu/x.csv", "retrieved": "2026-11-15T10:00:00.000000Z"});
    let l = vec![function_line(FX, &reading_sig(), None), relayed("N", "1", src.clone())];
    let m = serve_lite("rs-min", &l, None, |_| {}).await;
    let v = m.get(&format!("/cell/{FX}?site=N")).await;
    assert_eq!((v["trust"].as_str(), &v["receipt"]["source"]), (Some("relayed"), &src), "{v}");
}

fn refused_source(label: &str, src: Value) {
    let lite = Lite::new(label);
    let ls = [function_line(FX, &reading_sig(), None), relayed("N", "1", src.clone())];
    // refused at the cell's line, and the store is as it was (refused_whole compares the snapshots)
    refused_whole(&lite, "f.jsonl", &ls, 2, NOW);
    assert!(unbound(&lite, FX), "{src}: the function is bound");
}

/// A source that breaks spec-0.6.0 C2 refuses the file at its line and writes nothing.
#[test]
fn a_source_that_breaks_the_rules_is_refused() {
    // retrieved later than issued (the run's clock is NOW, 2026-11-15T12:00:00)
    refused_source("rs-late", with_source(|s| s["retrieved"] = json!("2026-11-15T12:00:00.000001Z")));
    // the publisher relaying itself
    refused_source("rs-self", with_source(|s| s["name"] = json!(HOST)));
    // an unknown member, a missing url, a takeover that is not https, a name that is not a host
    refused_source("rs-extra", with_source(|s| s["extra"] = json!("x")));
    refused_source("rs-nourl", with_source(|s| { s.as_object_mut().unwrap().remove("url"); }));
    refused_source("rs-http", with_source(|s| s["takeover"] = json!("http://mesh-test.datumesh.com/takeover")));
    refused_source("rs-name", with_source(|s| s["name"] = json!("not a host")));
    refused_source("rs-sha", with_source(|s| s["sha256"] = json!("abc")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describe_names_the_source_and_the_takeover() {
    let m = serve_lite("rs-describe", &lines(), None, |_| {}).await;
    let r = m.get_any(&format!("/describe/{}", call_path(FX))).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json["kind"], "function");
    assert_eq!(r.json["source"], json!({"name": "ecb.europa.eu", "relayed_by": HOST, "takeover": TAKEOVER}), "{}", r.json);
    let held = datumesh_core::FunctionDescriptor::from_json(&m.get_any(&format!("/descriptor/{}", fn_hash(&m, FX))).await.body).unwrap();
    let got = datumesh_core::check_describe_with(&r.body, &Default::default(), &[held]).unwrap_or_else(|e| panic!("{e}"));
    let s = got.source.expect("source");
    assert_eq!((s.name.as_str(), s.relayed_by.as_str(), s.takeover.as_str()), ("ecb.europa.eu", HOST, TAKEOVER));
    // a function that is not relayed has no source
    let own = m.get(&format!("/describe/{}", call_path(OWN))).await;
    assert!(own.get("source").is_none(), "{own}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_function_page_names_the_source_and_the_takeover() {
    let m = serve_lite("rs-page", &lines(), None, |_| {}).await;
    let html = get(&m, &format!("/fn/{FX}"), &[]).await;
    assert_eq!(html.status, 200, "{}", html.text());
    let text = html.text();
    let at = text.find("id=\"lineage\"").expect("a lineage section");
    let rest = &text[at..];
    let lineage = &rest[..rest[1..].find("<section").map(|i| i + 1).unwrap_or(rest.len())];
    for want in ["ecb.europa.eu", &format!("relayed by {HOST}"), TAKEOVER] {
        assert!(lineage.contains(want), "{want}: {lineage}");
    }
    assert!(lineage.contains(&format!("href=\"{TAKEOVER}\"")), "the takeover is a link: {lineage}");
    assert!(!text.to_lowercase().contains("<script"));
    let j = get(&m, &format!("/fn/{FX}?format=json"), &[]).await.json();
    assert_eq!(j["lineage"]["source"], json!({"name": "ecb.europa.eu", "relayed_by": HOST, "takeover": TAKEOVER}), "{j}");
    // not relayed: no source
    let own = get(&m, &format!("/fn/{OWN}?format=json"), &[]).await.json();
    assert!(own["lineage"].get("source").map_or(true, Value::is_null), "{own}");
    let own_html = get(&m, &format!("/fn/{OWN}"), &[]).await.text();
    assert!(!own_html.contains("relayed by"), "{own_html}");
}
