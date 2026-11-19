//! http 0.4.2 (spec-0.6.1, CONTRACT-build2-spec-0.6.1 S1, S3): names with DNS segments end to end,
//! and `publisher` / `source` as separate members of `describe` and `find`.
//!
//! * A namespace segment after the host may be a DNS name (2.3): the stored function
//!   `mesh-test.datumesh.com/relay/ecb.europa.eu/fx_reference(currency, date)` publishes with a
//!   `source` line, and `/cell` (by call and by cell URI), `/slice`, `/describe`, `/fn/<name>` and
//!   `/find` serve it; the receipt verifies with core 0.3.10. A formula under that namespace reads it
//!   (a dotted segment beside a divide), and a bundle (`POST /bundles`) whose names have DNS
//!   segments imports. What 2.3 refuses (an uppercase DNS segment, a DNS name as the last segment)
//!   is refused at publish with `E_CANON_NAME`.
//! * `describe` of a function and of a class answers `publisher`, the host of the namespace, never
//!   the domain inside the name; for a relayed function `source.name` is the source domain. Core
//!   0.3.10's describe check accepts the answers (and reads `publisher`).
//! * `find` results carry `publisher` (the host) and, for a relayed function only, `source` (the
//!   domain), as separate members; the query `ecb` finds the relay and its publisher is the mesh
//!   host.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use calculus::*;
use common::lite::*;
use common::pub031::*;
use common::readpath::*;
use common::seed_for;
use common::world::call_path;
use datumesh_http::bundle;
use publish4::*;
use serde_json::{json, Value};

const RELAY_NS: &str = "mesh-test.datumesh.com/relay/ecb.europa.eu";
const FXR: &str = "mesh-test.datumesh.com/relay/ecb.europa.eu/fx_reference";
const FXI: &str = "mesh-test.datumesh.com/relay/ecb.europa.eu/fx_inverse";
const PLAIN: &str = "mesh-test.datumesh.com/scratch_b5/plain/level";
const CLASS: &str = "mesh-test.datumesh.com/relay/ecb.europa.eu/pair";
const DOMAIN: &str = "ecb.europa.eu";
const TAKEOVER: &str = "https://mesh-test.datumesh.com/takeover/ecb.europa.eu";

fn fx_sig() -> String {
    r#"{"args":{"currency":{"type":"string","valid":false},"date":{"type":"date","valid":true}},"order":["currency","date"],"returns":{"type":"number","unit":"1"}}"#
        .to_string()
}

fn source() -> Value {
    json!({"name": DOMAIN,
        "url": "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-hist.zip",
        "retrieved": "2026-11-15T10:00:00.000000Z",
        "takeover": TAKEOVER})
}

fn fx_cell(date: &str, value: &str) -> String {
    let mut v: Value = serde_json::from_str(&cell_line(
        FXR,
        json!({"currency": {"type": "string", "value": "USD"}, "date": {"type": "date", "value": date}}),
        value,
    ))
    .unwrap();
    v["source"] = source();
    v.to_string()
}

fn lines() -> Vec<String> {
    vec![
        function_line(FXR, &fx_sig(), None),
        fx_cell("2026-11-13", "1.0892"),
        fx_cell("2026-11-14", "1.091"),
        // computed, in the same namespace: a dotted segment beside a divide (formula 0.5.1)
        formula_line(FXI, &format!("1 / {FXR}(currency: currency, date: date)"), &fx_sig(), None),
        catalog_line(
            FXR,
            "function",
            "Euro reference rate",
            "The euro foreign exchange reference rate, relayed from the ECB.",
            json!({"currency": "the currency", "date": "the day"}),
            json!({}),
        ),
        catalog_line(
            FXI,
            "function",
            "Inverse euro reference rate",
            "One over the euro reference rate; computed here, not relayed.",
            json!({"currency": "the currency", "date": "the day"}),
            json!({}),
        ),
        // a function that is not relayed and has no DNS segment
        function_line(PLAIN, &reading_sig(), None),
        site_cell(PLAIN, "N", "7"),
        catalog_line(PLAIN, "function", "Plain level", "A plain level.", json!({"site": "the site"}), json!({})),
    ]
}

fn core_keys() -> datumesh_core::Keys {
    let mut k = datumesh_core::Keys::new();
    k.insert_response(
        HOST,
        200,
        Some(&datumesh_http::keydoc::with_key(HOST, None, &common::key_for(HOST), K_FROM).unwrap()),
    );
    k
}

async fn served(label: &str) -> Served {
    serve_lite(label, &lines(), None, |_| {}).await
}

fn describe_path(name: &str) -> String {
    format!("/describe/{}", call_path(name))
}

// ---- a name with a DNS segment, end to end ---------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stored_function_with_a_dns_segment_is_served_by_uri_and_by_call() {
    let m = served("dn-cell").await;
    // by cell URI (5.1): the namespace is split from the function at the last `/`
    let r = m.get_any(&format!("/cell/{FXR}?currency=USD&date=2026-11-13")).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!((r.json["outcome"].as_str(), r.json["trust"].as_str()), (Some("ok"), Some("relayed")), "{}", r.json);
    assert_eq!(r.json["receipt"]["cell"]["ns"], RELAY_NS, "{}", r.json);
    assert_eq!(r.json["receipt"]["cell"]["fn"], "fx_reference");
    assert_eq!(r.json["receipt"]["value"], "1.0892", "{}", r.json);
    assert_eq!(r.json["receipt"]["source"]["name"], DOMAIN);
    // the receipt's signer and publisher: the host and the namespace, not the domain of the name
    assert_eq!(r.json["receipt"]["publisher"], RELAY_NS);
    assert!(r.json["receipt"]["computed_by"].as_str().unwrap().starts_with(&format!("{HOST}#")), "{}", r.json);
    // by call (5.8)
    let call = format!("{FXR}(currency: \"USD\", date: 2026-11-13)");
    let c = m.get_any(&format!("/cell/{}", call_path(&call))).await;
    assert_eq!(c.status, 200, "{}", String::from_utf8_lossy(&c.body));
    assert_eq!(c.json["outcome"], "ok", "{}", c.json);
    assert_eq!(c.json["receipt"]["hash"], r.json["receipt"]["hash"], "the same cell, the same receipt");
    // core 0.3.10 checks the answer and the receipt by itself
    let cell = cell_json(&m, FXR, &[("currency", "string", "USD"), ("date", "date", "2026-11-13")]);
    let req = datumesh_core::Request::new(&core_id(&cell), None).unwrap();
    let a = datumesh_core::check_answer(r.status, &r.body, &core_keys(), &req).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(a.outcome.as_str(), "ok");
    let v = datumesh_core::verify(&serde_json::to_vec(&r.json["receipt"]).unwrap(), &core_keys(), None, None);
    assert!(v.ok, "{:?}", v.error());
    // the receipt by its hash
    let hash = datumesh_core::Receipt::from_json(&serde_json::to_vec(&r.json["receipt"]).unwrap())
        .unwrap()
        .hash()
        .to_string();
    assert_eq!(m.get_any(&format!("/receipt/{hash}")).await.status, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_computed_function_reads_a_function_with_a_dns_segment() {
    let m = served("dn-computed").await;
    let r = m.get_any(&format!("/cell/{FXI}?currency=USD&date=2026-11-13")).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json["outcome"], "ok", "{}", r.json);
    assert_eq!(r.json["trust"], "computed", "{}", r.json);
    assert_eq!(r.json["receipt"]["cell"]["ns"], RELAY_NS);
    let inputs = r.json["receipt"]["inputs"].as_array().unwrap_or_else(|| panic!("{}", r.json));
    assert_eq!(inputs.len(), 1, "{}", r.json);
    let cell = cell_json(&m, FXI, &[("currency", "string", "USD"), ("date", "date", "2026-11-13")]);
    let req = datumesh_core::Request::new(&core_id(&cell), None).unwrap();
    datumesh_core::check_answer(r.status, &r.body, &core_keys(), &req).unwrap_or_else(|e| panic!("{e}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slice_of_a_function_with_a_dns_segment() {
    let m = served("dn-slice").await;
    let r = m.get_any(&format!("/slice/{FXR}?currency=USD&date=*")).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json["members"].as_array().map(Vec::len), Some(2), "{}", r.json);
    assert_eq!(r.json["slice"]["ns"], RELAY_NS, "{}", r.json);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_function_page_of_a_function_with_a_dns_segment() {
    let m = served("dn-page").await;
    let html = get(&m, &format!("/fn/{FXR}"), &[]).await;
    assert_eq!(html.status, 200, "{}", html.text());
    assert!(html.text().contains(FXR), "{}", html.text());
    let j = get(&m, &format!("/fn/{FXR}?format=json"), &[]).await.json();
    assert_eq!(j["name"], FXR, "{j}");
    assert_eq!(j["lineage"]["source"]["name"], DOMAIN, "{j}");
    assert_eq!(j["lineage"]["source"]["relayed_by"], HOST, "{j}");
    // an unbound name with a DNS segment is not found, in the not-covered shape
    let none = get(&m, &format!("/fn/{RELAY_NS}/nothing"), &[]).await;
    assert_eq!(none.status, 404, "{}", none.text());
    assert!(none.text().contains("E_NAME_UNBOUND"), "{}", none.text());
}

// ---- describe: the publisher is the host --------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describe_of_a_function_names_the_host_as_publisher_and_the_domain_as_source() {
    let m = served("dn-describe").await;
    let r = m.get_any(&describe_path(FXR)).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json["kind"], "function", "{}", r.json);
    assert_eq!(r.json["fn"], FXR);
    assert_eq!(r.json["publisher"], HOST, "the publisher is the host of the namespace: {}", r.json);
    assert_ne!(r.json["publisher"], DOMAIN);
    assert_eq!(r.json["source"]["name"], DOMAIN, "{}", r.json);
    assert_eq!(r.json["source"]["relayed_by"], HOST, "{}", r.json);
    // core 0.3.10's describe check accepts it, and reads publisher
    let held = datumesh_core::FunctionDescriptor::from_json(
        &m.get_any(&format!("/descriptor/{}", fn_hash(&m, FXR))).await.body,
    )
    .unwrap();
    let got = datumesh_core::check_describe_with(&r.body, &Default::default(), &[held]).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(got.publisher.as_deref(), Some(HOST));
    assert_eq!(got.function.as_deref(), Some(FXR));
    assert_eq!(got.source.expect("source").name, DOMAIN);

    // a computed function in the same namespace: the same publisher, and no source (its name has a
    // domain in it, and that is a label only)
    let c = m.get_any(&describe_path(FXI)).await;
    assert_eq!(c.status, 200, "{}", String::from_utf8_lossy(&c.body));
    assert_eq!(c.json["publisher"], HOST, "{}", c.json);
    assert!(c.json.get("source").is_none_or(Value::is_null), "{}", c.json);
    let held = datumesh_core::FunctionDescriptor::from_json(
        &m.get_any(&format!("/descriptor/{}", fn_hash(&m, FXI))).await.body,
    )
    .unwrap();
    let reading = datumesh_core::FunctionDescriptor::from_json(
        &m.get_any(&format!("/descriptor/{}", fn_hash(&m, FXR))).await.body,
    )
    .unwrap();
    let got = datumesh_core::check_describe_with(&c.body, &Default::default(), &[held, reading])
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(got.publisher.as_deref(), Some(HOST));

    // a function without a DNS segment gets a publisher too
    let p = m.get(&describe_path(PLAIN)).await;
    assert_eq!(p["publisher"], HOST, "{p}");
    assert!(p.get("source").is_none(), "{p}");
}

/// The class `…/relay/ecb.europa.eu/pair`, steward the host: describe answers `publisher`.
async fn served_with_class(label: &str) -> Served {
    let lite = Lite::new(label);
    let (rate, convert) = (stored_descriptor(&rate_sig()), stored_descriptor(&convert_sig()));
    let (rate_hash, convert_hash) = (descriptor_hash(&rate), descriptor_hash(&convert));
    let f1 = write(&lite, "d.jsonl", &[descriptor_line(&rate), descriptor_line(&convert)]);
    let f2 = write(
        &lite,
        "c.jsonl",
        &[
            class_line(CLASS, &pair_class(&[("convert", &convert_hash), ("rate", &rate_hash)]), None),
            catalog_line(
                CLASS,
                "class",
                "Euro pairs",
                "Currency pairs relayed from the ECB, by base and quote.",
                json!({"base": "the base currency", "quote": "the quote currency"}),
                json!({}),
            ),
        ],
    );
    let _ = run(&lite, &f1, NOW);
    for d in [&rate, &convert] {
        use datumesh_store::Store;
        lite.store.put_descriptor(&descriptor_canonical(d)).unwrap();
    }
    run(&lite, &f2, T1).unwrap_or_else(|e| panic!("the class is refused: {e}"));
    build_served(lite, None, |_| {}).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describe_of_a_class_in_a_dns_namespace_names_the_host_as_publisher() {
    use datumesh_store::Store;
    let m = served_with_class("dn-class").await;
    let r = m.get_any(&describe_path(CLASS)).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json["kind"], "class", "{}", r.json);
    assert_eq!(r.json["publisher"], HOST, "{}", r.json);
    let hash = m.lite.store.resolve_class(CLASS, END).unwrap().unwrap();
    let held = datumesh_core::ClassDescriptor::from_json(&m.lite.store.class(&hash).unwrap().unwrap()).unwrap();
    let functions: Vec<_> = held
        .methods
        .values()
        .map(|h| datumesh_core::FunctionDescriptor::from_json(&m.lite.store.descriptor(h).unwrap()).unwrap())
        .collect();
    let held = std::collections::BTreeMap::from([(CLASS.to_string(), held)]);
    let got = datumesh_core::check_describe_with(&r.body, &held, &functions).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(got.publisher.as_deref(), Some(HOST));
    // the spec's schema (spec-0.6.1) knows the member
    common::schemas().assert_describe(&r.body);
}

// ---- find: publisher and source, apart ------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn find_results_carry_publisher_and_for_a_relay_the_source() {
    let m = served("dn-find").await;
    let v = m.find("ecb").await;
    let relay = result(&v, FXR);
    assert_eq!(relay["publisher"], HOST, "the publisher is the mesh host, never the domain in the name: {relay}");
    assert_eq!(relay["source"], DOMAIN, "{relay}");
    // the computed function of the same namespace is found by the same query, its publisher is the host
    // and a domain inside its name is not a source
    let inv = result(&v, FXI);
    assert_eq!(inv["publisher"], HOST, "{inv}");
    assert!(inv.get("source").is_none_or(Value::is_null), "{inv}");
    for r in results(&v) {
        assert_ne!(r["publisher"], DOMAIN, "{r}");
        assert_eq!(r["publisher"], HOST, "{r}");
    }
    // the members that were there stay
    for k in ["kind", "name", "call", "label", "match", "score", "descriptor", "fn_hash", "since"] {
        assert!(relay.get(k).is_some(), "{k} in {relay}");
    }
    // a plain function: a publisher, no source
    let p = m.find("level").await;
    let plain = result(&p, PLAIN);
    assert_eq!(plain["publisher"], HOST, "{plain}");
    assert!(plain.get("source").is_none_or(Value::is_null), "{plain}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn find_returns_a_class_with_its_publisher() {
    let m = served_with_class("dn-find-class").await;
    // the catalogue is the mesh's own text: the class is found by its name
    let v = m.find("pair").await;
    let class = results(&v)
        .into_iter()
        .find(|r| r["kind"] == "class" && r["name"] == CLASS)
        .unwrap_or_else(|| panic!("the class is not found: {v}"));
    assert_eq!(class["publisher"], HOST, "{class}");
    assert!(class.get("source").is_none_or(Value::is_null), "{class}");
}

// ---- refused at publish -----------------------------------------------------------------------------------

fn refused_name(label: &str, name: &str) {
    let lite = Lite::new(label);
    let e = refused_whole(&lite, "f.jsonl", &[function_line(name, &fx_sig(), None)], 1, NOW);
    assert!(e.detail.contains("E_CANON_NAME"), "{name}: the detail names E_CANON_NAME: {e}");
}

#[test]
fn names_that_2_3_refuses_are_refused_at_publish_with_e_canon_name() {
    // an uppercase letter in a DNS segment
    refused_name("dn-upper", "mesh-test.datumesh.com/relay/ECB.europa.eu/fx_reference");
    // a DNS name as the last segment (the function is a `name`)
    refused_name("dn-last", "mesh-test.datumesh.com/relay/ecb.europa.eu");
    refused_name("dn-last2", "mesh-test.datumesh.com/relay/fx_reference.ecb.europa.eu");
    // a trailing dot, an underscore, an empty label, a leading hyphen, an all-digit last label
    refused_name("dn-dot", "mesh-test.datumesh.com/relay/ecb.europa.eu./fx_reference");
    refused_name("dn-us", "mesh-test.datumesh.com/relay/ecb_x.europa.eu/fx_reference");
    refused_name("dn-empty", "mesh-test.datumesh.com/relay/ecb..eu/fx_reference");
    refused_name("dn-hyphen", "mesh-test.datumesh.com/relay/-ecb.europa.eu/fx_reference");
    refused_name("dn-digits", "mesh-test.datumesh.com/relay/10.0.0.1/fx_reference");
}

#[test]
fn names_that_2_3_accepts_publish() {
    for (i, name) in [
        "mesh-test.datumesh.com/relay/bank-x.example/rate",
        "mesh-test.datumesh.com/relay/xn--bcher-kva.example/rate",
        "mesh-test.datumesh.com/relay/bankofengland.co.uk/bank_rate",
    ]
    .iter()
    .enumerate()
    {
        let lite = Lite::new(&format!("dn-ok{i}"));
        let f = write(&lite, "f.jsonl", &[function_line(name, &fx_sig(), None)]);
        run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{name} is refused: {e}"));
        assert!(!unbound(&lite, name), "{name} is not bound");
    }
}

// ---- a bundle with DNS segments -----------------------------------------------------------------------------

const B_NS: &str = "pub.example/relay/ecb.europa.eu";
const B_NAME: &str = "pub.example/relay/ecb.europa.eu/level";

fn dns_bundle(name: &str, ns: &str) -> Vec<u8> {
    use datumesh_core::{Cell, ReceiptBuilder};
    let cell = Cell { ns: ns.to_string(), fn_name: "level".into(), fn_hash: stored_fn().hash(), args: common::args(&[("site", common::s("N"))]) };
    let r = ReceiptBuilder::new()
        .cell(cell)
        .value(common::n("42"))
        .valid(common::args(&[]))
        .asof(TX)
        .tx(TX)
        .issued(TX)
        .publisher(ns)
        .signer_host(PUB)
        .licence("CC0-1.0")
        .summary("test")
        .sign(&common::key_for(PUB))
        .unwrap();
    bundle::build_with_catalog(
        ns,
        vec![stored_fn().canonical()],
        vec![(name.to_string(), stored_fn().hash())],
        vec![common::receipt_json(&r)],
        vec![catalog_entry(name, "function", "Level", "A relayed level.", json!({"site": "the site"}), json!({}))],
        &seed_for(PUB),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bundle_whose_names_have_dns_segments_imports() {
    let stub = Stub::start(PUB).await;
    let p = public("dn-bundle", &[stub.peer()], Some(PUB)).await;
    let r = p.post(dns_bundle(B_NAME, B_NS)).await;
    assert_eq!(r.status, 201, "{}", r.text());
    let j = r.json();
    assert_eq!(j["names"], json!([B_NAME]), "{j}");
    assert_eq!(j["functions"][0]["page"], format!("https://{HOST}/fn/{B_NAME}"), "{j}");
    let c = p.get(&format!("/cell/{B_NAME}?site=N"), &[]).await;
    assert_eq!(c.status, 200, "{}", c.text());
    assert_eq!(c.json()["outcome"], "ok");
    assert_eq!(c.json()["trust"], "primary");
    // the publisher of it is the host of the namespace: pub.example, not ecb.europa.eu
    let d = p.get(&describe_path(B_NAME), &[]).await.json();
    assert_eq!(d["publisher"], PUB, "{d}");
    let f = p.m.find("relay").await;
    assert_eq!(result(&f, B_NAME)["publisher"], PUB, "{f}");
}

use datumesh_core::Resolver as _;
