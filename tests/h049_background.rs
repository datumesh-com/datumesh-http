//! http 0.4.9, CONTRACT-0.4.9 H2, H3 and H4: three public paths that built their answer on the request now
//! read the last build of a background thread, and answer `503` with `Retry-After` and `no-store` before the
//! first build. Written first; FAIL until then.
//!
//! * H2: `GET /takeover/<domain>` (knob `DATUMESH_SERIES_FIRST_BUILD_DELAY_SECS`, the series knob: the
//!   takeover list is built by the series thread);
//! * H3: `GET /describe/<64 or 12 hex object id>` (knob `DATUMESH_INDEX_FIRST_BUILD_DELAY_SECS`);
//! * H4: `GET /find?summary=1` and the empty query (knob `DATUMESH_FIND_FIRST_BUILD_DELAY_SECS`).
//!
//! Each knob holds the first background build back for that many seconds (a test knob, default 0), as the
//! series knob does, so that "before the first build" can be observed without a race.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use std::time::{Duration, Instant};

use calculus::*;
use common::lite::*;
use common::pub031::*;
use common::readpath::*;
use common::world::call_path;
use datumesh_core::{KeyValue, ObjectKey, Scalar, Type};
use datumesh_http::Config;
use datumesh_store::Store;
use publish4::{get, Reply4};
use serde_json::{json, Value};

fn config(vars: &[(&str, &str)]) -> Config {
    let mut env: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    env.push(("DATUMESH_HOST".into(), HOST.into()));
    for k in ["DATUMESH_RATE_PER_IP", "DATUMESH_RATE_IP_BURST", "DATUMESH_RATE_SERIES_PER_IP"] {
        env.push((k.into(), "1000000".into()));
    }
    Config::from_vars(move |k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).unwrap()
}

#[track_caller]
fn assert_cold(r: &Reply4, what: &str) {
    assert_eq!(r.status, 503, "{what}: {}", r.text());
    let ra: u64 = r.header("retry-after").unwrap_or_else(|| panic!("{what}: no Retry-After")).parse().unwrap();
    assert!((1..=300).contains(&ra), "{what}: Retry-After {ra}");
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"), "{what}");
}

async fn until_built(m: &Served, path: &str) -> Reply4 {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let r = get(m, path, &[]).await;
        if r.status != 503 || Instant::now() > end {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn plain(h: &str) -> String {
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
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---- H2: the takeover page ----------------------------------------------------------------------

const A1: &str = "mesh-test.datumesh.com/scratch_b5/relay/a1";
const A2: &str = "mesh-test.datumesh.com/scratch_b5/relay/a2";
const B1: &str = "mesh-test.datumesh.com/scratch_b5/relay/b1";
const OWN: &str = "mesh-test.datumesh.com/scratch_b5/own/level";
const ECB: &str = "ecb.europa.eu";
const OTHER: &str = "stats.example.org";

fn relayed(f: &str, domain: &str) -> String {
    let mut v: Value = serde_json::from_str(&site_cell(f, "N", "1.5")).unwrap();
    v["source"] = json!({"name": domain, "url": format!("https://{domain}/data.csv"),
        "retrieved": "2026-11-15T10:00:00.000000Z",
        "takeover": format!("https://mesh-test.datumesh.com/takeover/{domain}")});
    v.to_string()
}

fn takeover_lines() -> Vec<String> {
    let mut v = Vec::new();
    for (f, d) in [(A1, ECB), (A2, ECB), (B1, OTHER)] {
        v.push(function_line(f, &reading_sig(), None));
        v.push(relayed(f, d));
        v.push(catalog_line(f, "function", "Rate", "A relayed rate.", json!({"site": "the site"}), json!({})));
    }
    v.push(function_line(OWN, &reading_sig(), None));
    v.push(site_cell(OWN, "N", "7"));
    v.push(catalog_line(OWN, "function", "Level", "A stored level.", json!({"site": "the site"}), json!({})));
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn before_the_first_build_the_takeover_page_answers_503() {
    let cfg = config(&[("DATUMESH_SERIES_FIRST_BUILD_DELAY_SECS", "3600")]);
    let m = serve_lite("h2-cold", &takeover_lines(), Some(cfg), |_| {}).await;
    for path in [format!("/takeover/{ECB}"), format!("/takeover/{ECB}?format=json"), format!("/takeover/{OTHER}")] {
        let t = Instant::now();
        let r = get(&m, &path, &[]).await;
        assert_cold(&r, &path);
        assert!(t.elapsed() < Duration::from_secs(1), "{path}: answered at once, took {:?}", t.elapsed());
    }
    // still cold after the requests: they built nothing
    assert_cold(&get(&m, &format!("/takeover/{ECB}"), &[]).await, "again");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_the_first_build_the_page_lists_the_relays_and_the_policy_is_unchanged() {
    let m = serve_lite("h2-built", &takeover_lines(), Some(config(&[])), |_| {}).await;
    let ecb = until_built(&m, &format!("/takeover/{ECB}?format=json")).await;
    assert_eq!(ecb.status, 200, "{}", ecb.text());
    let names = |j: &Value| -> Vec<String> {
        j["functions"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(names(&ecb.json()), vec![A1.to_string(), A2.to_string()], "{}", ecb.text());
    let other = get(&m, &format!("/takeover/{OTHER}?format=json"), &[]).await;
    assert_eq!(other.status, 200, "{}", other.text());
    assert_eq!(names(&other.json()), vec![B1.to_string()]);
    for f in ecb.json()["functions"].as_array().unwrap() {
        let name = f["name"].as_str().unwrap();
        assert_eq!(f["page"], format!("https://{HOST}/fn/{name}"), "{f}");
        let page = get(&m, &format!("/fn/{name}?format=json"), &[]).await.json();
        assert_eq!(f["fn_hash"], page["fn_hash"], "{f}");
    }
    let mut keys: Vec<String> = ecb.json().as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["contact", "domain", "functions", "host", "policy", "steps"]);
    let html = plain(&get(&m, &format!("/takeover/{ECB}"), &[]).await.text());
    assert!(html.contains(&format!("This mesh ({HOST}) relays these values from {ECB} unchanged")), "{html}");
    assert!(html.contains(&format!("{ECB} remains their source")), "{html}");
    // quick, and the same every time
    for _ in 0..5 {
        let t = Instant::now();
        let r = get(&m, &format!("/takeover/{ECB}?format=json"), &[]).await;
        assert_eq!(r.status, 200);
        assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
        assert_eq!(r.body, ecb.body);
    }
}

// ---- H3: describe of an object id ---------------------------------------------------------------

const CLASS: &str = "mesh-test.datumesh.com/h049/k/c";
const RATE: &str = "mesh-test.datumesh.com/h049/k/c/rate";

/// The class `CLASS` with the method `rate`, and one stored cell of it (EUR, USD); its object's id.
async fn with_object(label: &str, vars: &[(&str, &str)]) -> (Served, String) {
    let lite = Lite::new(label);
    let rate = stored_descriptor(&rate_sig());
    let rate_hash = descriptor_hash(&rate);
    let f1 = write(&lite, "d.jsonl", &[descriptor_line(&rate)]);
    let class = write(&lite, "c.jsonl", &[class_line(CLASS, &pair_class(&[("rate", &rate_hash)]), None)]);
    let _ = run(&lite, &f1, NOW);
    lite.store.put_descriptor(&descriptor_canonical(&rate)).unwrap();
    run(&lite, &class, T1).unwrap_or_else(|e| panic!("{e}"));
    let cells = write(&lite, "x.jsonl", &[rate_cell(RATE, "1.0832", None, None, None)]);
    run(&lite, &cells, T2).unwrap_or_else(|e| panic!("{e}"));
    let class_hash = lite.store.resolve_class(CLASS, END).unwrap().unwrap();
    let key = ObjectKey::new(
        CLASS,
        [
            ("base".to_string(), KeyValue::Scalar(Scalar::from_lexical(Type::String, "EUR").unwrap())),
            ("quote".to_string(), KeyValue::Scalar(Scalar::from_lexical(Type::String, "USD").unwrap())),
        ],
    )
    .unwrap();
    let id = key.object_id(&class_hash).unwrap();
    let m = build_served(lite, Some(config(vars)), |_| {}).await;
    (m, id)
}

fn encoding() -> String {
    format!(r#"{CLASS}(base:"EUR",quote:"USD")"#)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn before_the_first_build_describe_of_an_object_id_answers_503() {
    let (m, id) = with_object("h3-cold", &[("DATUMESH_INDEX_FIRST_BUILD_DELAY_SECS", "3600")]).await;
    for what in [id.clone(), id[..12].to_string(), "f".repeat(64), "f".repeat(12)] {
        let t = Instant::now();
        let r = get(&m, &format!("/describe/{}", call_path(&what)), &[]).await;
        assert_cold(&r, &what);
        assert!(t.elapsed() < Duration::from_secs(1), "{what}: answered at once, took {:?}", t.elapsed());
    }
    // a class name, a key encoding and a function name are not object ids: they do not wait for the index
    let r = get(&m, &format!("/describe/{}", call_path(CLASS)), &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = get(&m, &format!("/describe/{}", call_path(&encoding())), &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    // and the id still waits
    assert_cold(&get(&m, &format!("/describe/{id}"), &[]).await, "after those");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_the_first_build_describe_of_an_object_id_answers() {
    let (m, id) = with_object("h3-built", &[]).await;
    // the mesh learns of a class when a request names it
    let by_key = get(&m, &format!("/describe/{}", call_path(&encoding())), &[]).await;
    assert_eq!(by_key.status, 200, "{}", by_key.text());
    let end = Instant::now() + Duration::from_secs(60);
    let by_id = loop {
        let r = get(&m, &format!("/describe/{id}"), &[]).await;
        if r.status == 200 || Instant::now() > end {
            break r;
        }
        assert!(r.status == 503 || r.status == 404, "{}: {}", r.status, r.text());
        if r.status == 503 {
            assert_cold(&r, "before the build");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    assert_eq!(by_id.status, 200, "no answer for the object id in 60 s: {}", by_id.text());
    assert_eq!(by_id.body, by_key.body);
    let short = get(&m, &format!("/describe/{}", &id[..12]), &[]).await;
    assert_eq!((short.status, short.body.clone()), (200, by_key.body.clone()), "{}", short.text());
    // requests read the last build: quick
    for _ in 0..5 {
        let t = Instant::now();
        assert_eq!(get(&m, &format!("/describe/{id}"), &[]).await.status, 200);
        assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    }
}

// ---- H4: find ---------------------------------------------------------------------------------

const BANK: &str = "mesh-test.datumesh.com/relay/bankofengland.co.uk/bank_rate";
const ESTR: &str = "mesh-test.datumesh.com/relay/ecb.europa.eu/estr";
const BOUND_CLOCK: &str = "2026-01-01T00:00:00.000000Z";

fn date_sig() -> String {
    r#"{"args":{"date":{"type":"date","valid":true}},"order":["date"],"returns":{"type":"number","unit":"1"}}"#
        .to_string()
}

fn dated(f: &str, domain: &str, date: &str, value: &str) -> String {
    let tx = format!("{date}T12:00:00.000000Z");
    json!({"cell": {"fn": f, "args": {"date": {"type": "date", "value": date}}}, "licence": "CC0-1.0",
        "summary": "Synthetic test data.", "value": {"type": "number", "value": value},
        "source": {"name": domain, "url": format!("https://{domain}/data"), "retrieved": BOUND_CLOCK},
        "tx": tx, "asof": tx, "issued": tx})
    .to_string()
}

/// BANK has four cells, ESTR one: one descriptor, two names.
async fn twins(label: &str, vars: &[(&str, &str)]) -> Served {
    let lite = Lite::new(label);
    let f0 = write(
        &lite,
        "functions.jsonl",
        &[function_line(BANK, &date_sig(), None), function_line(ESTR, &date_sig(), None)],
    );
    run(&lite, &f0, BOUND_CLOCK).unwrap_or_else(|e| panic!("{e}"));
    let mut v = Vec::new();
    for (d, x) in [("2026-03-19", "4.5"), ("2026-03-20", "4.5"), ("2026-05-07", "4.25"), ("2026-05-08", "4.25")] {
        v.push(dated(BANK, "bankofengland.co.uk", d, x));
    }
    v.push(dated(ESTR, "ecb.europa.eu", "2026-07-01", "2.9"));
    v.push(catalog_line(BANK, "function", "Bank Rate", "The bank rate.", json!({"date": "the day"}), json!({})));
    v.push(catalog_line(ESTR, "function", "Euro short-term rate", "The estr.", json!({"date": "the day"}), json!({})));
    let f = write(&lite, "fixture.jsonl", &v);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    build_served(lite, Some(config(vars)), |_| {}).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn before_the_first_build_find_summary_and_the_empty_query_answer_503() {
    let m = twins("h4-cold", &[("DATUMESH_FIND_FIRST_BUILD_DELAY_SECS", "3600")]).await;
    for path in ["/find?summary=1", "/find?q=", "/find?summary=1&format=json"] {
        let t = Instant::now();
        let r = get(&m, path, &[]).await;
        assert_cold(&r, path);
        assert!(t.elapsed() < Duration::from_secs(1), "{path}: answered at once, took {:?}", t.elapsed());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_the_first_build_the_summary_counts_the_cells_of_each_name() {
    let m = twins("h4-built", &[]).await;
    let r = until_built(&m, "/find?summary=1").await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = r.json();
    let f = v["functions"].as_array().unwrap();
    let cells = |n: &str| f.iter().find(|e| e["name"] == n).unwrap_or_else(|| panic!("no {n} in {v}"))["cells"].clone();
    assert_eq!(cells(BANK), json!(4), "{v}");
    assert_eq!(cells(ESTR), json!(1), "{v}");
    let desc = |n: &str| f.iter().find(|e| e["name"] == n).unwrap()["description"].clone();
    assert_eq!(desc(BANK), "The bank rate.", "{v}");
    assert_eq!(desc(ESTR), "The estr.", "{v}");
    // the empty query is served from the same build
    let e = until_built(&m, "/find?q=").await;
    assert_eq!(e.status, 200, "{}", e.text());
    for _ in 0..5 {
        let t = Instant::now();
        let r = get(&m, "/find?summary=1", &[]).await;
        assert_eq!((r.status, r.body.clone()), (200, r.body.clone()));
        assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
        assert_eq!(r.json()["functions"], v["functions"]);
    }
}
