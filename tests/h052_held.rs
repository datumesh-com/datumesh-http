//! http 0.5.2: the held sentence on production's shape. Written first; FAIL until then.
//!
//! The 0.5.1 tests passed on fixtures that are not production's shape (the rehearsal of the restored copy read
//! "scripture (provenance, texts)" and "earth (monthly rainfall)"). The shapes here are production's catalogue
//! (`rehearsal-records-0.7/catalog/production-catalog.jsonl`):
//!
//! * scripture is two CLASSES, `scripture/edition` (measure provenance) and `scripture/verse` (measure text); the cells of a
//!   class entry are the stored cells of its methods' functions, so the verse class's many cells put "texts" first;
//! * rainfall is a STORED function of the publisher's own namespace, not a relay, with `source_name` "CHIRPS";
//! * a relay is `<host>/relay/<domain>/...`.

#![allow(dead_code)]

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;

use catjob::*;
use catworld::*;
use common::lite::*;
use common::pub031::{class_line, BEFORE, T1, T2};
use crworld::*;
use serde_json::{json, Value};

const TAIL: &str = "Ask find in plain words for what you need.";

fn f(d: &str, t: &str, m: &str, r: &str, q: &str) -> Value {
    fac(&[d], &[t], &[m], &[r], &[q])
}

/// A class with the key `ref` and one stored method `text`, as an author writes it.
fn class_lines(name: &str) -> String {
    let sig = json!({"args": {"ref": {"type": "string", "valid": false}}, "order": ["ref"], "returns": "string"});
    let d = json!({"keys": {"ref": {"key_normalisation": "none", "type": "string"}},
        "methods": {"text": {"stored": sig}}, "order": ["ref"],
        "steward": {"namespace": HOST, "process": format!("https://{HOST}/classes/change")}});
    class_line(name, &d.to_string(), None)
}

/// `n` stored cells of the class's method `text`.
fn class_cells(name: &str, n: usize) -> Vec<String> {
    (0..n)
        .map(|i| cell_line(&format!("{name}/text"), &[("ref", "string", &format!("r{i:03}"))], ("string", "x")))
        .collect()
}

/// `n` relayed cells of a relay function (a date each).
fn relay_cells(name: &str, n: usize) -> Vec<String> {
    let rest = name.split("/relay/").nth(1).unwrap();
    let domain = rest.split('/').next().unwrap();
    (0..n)
        .map(|i| {
            json!({"cell": {"fn": name, "args": {"date": {"type": "date", "value": format!("2026-10-{:02}", i + 1)}}},
                "licence": "CC0-1.0", "summary": "Synthetic test data.", "value": {"type": "number", "value": "1.5"},
                "source": {"name": domain, "url": format!("https://{domain}/data.csv"), "retrieved": "2026-11-15T10:00:00.000000Z",
                    "takeover": format!("https://{HOST}/takeover/{domain}")}})
            .to_string()
        })
        .collect()
}

/// `n` stored cells of an own (not relayed) function of one date.
fn own_cells(name: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| cell_line(name, &[("date", "date", &format!("2026-10-{:02}", i + 1))], ("number", "1"))).collect()
}

fn entry(name: &str, kind: &str, facets: Value, source_name: Option<&str>) -> String {
    let mut extra = json!({"facets": facets});
    if let Some(s) = source_name {
        extra["source_name"] = json!(s);
    }
    catalog_line_of(name, kind, extra)
}

async fn held_of(lite: Lite) -> String {
    let m = serve(lite, &[]).await;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let r = until_built(&m, "/find?summary=1").await;
        assert_eq!(r.status, 200, "{}", r.text());
        let v = r.json();
        if let Some(h) = v["held"].as_str() {
            return h.to_string();
        }
        assert!(std::time::Instant::now() < end, "no held in {v}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// The two scripture classes of production, with `verse` and `edition` stored cells under their methods.
fn scripture(lite: &Lite, verse: usize, edition: usize) {
    let (v, e) = (host_name("scripture/verse"), host_name("scripture/edition"));
    publish(lite, "scripture-classes.jsonl", &[class_lines(&e), class_lines(&v)], BEFORE)
        .unwrap_or_else(|e| panic!("classes: {e}"));
    let mut lines = class_cells(&v, verse);
    lines.extend(class_cells(&e, edition));
    lines.push(entry(&v, "class", f("scripture", "verse", "text", "world", "static"), None));
    lines.push(entry(&e, "class", f("scripture", "edition", "provenance", "world", "static"), None));
    publish(lite, "scripture.jsonl", &lines, NOW).unwrap_or_else(|e| panic!("scripture: {e}"));
}

/// Scripture as two classes: the verse class holds 50 cells under its method, the edition class 5, so the text measure has
/// the cells and reads first: "texts, provenance" (by name alone it is the other way round).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h52_scripture_as_two_classes_reads_texts_before_provenance() {
    let lite = Lite::new("h52-scripture");
    scripture(&lite, 50, 5);
    build(&lite, T3);
    let held = held_of(lite).await;
    assert_eq!(held, format!("This mesh holds scripture (texts, provenance). {TAIL}"));
}

/// Rainfall as production has it: a stored function of the publisher's own namespace (not a relay, not a formula), with cells,
/// facets monthly and `source_name` "CHIRPS". Any entry that is not a relay and declares a `source_name` reads
/// "computed from <name>" (the user's decision of 2026-11-23, which supersedes the 0.7 R8 rule that own data names none).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h52_rainfall_as_a_stored_function_reads_computed_from_chirps() {
    let lite = Lite::new("h52-rainfall");
    let n = host_name("earth/monthly_rainfall");
    let mut lines = vec![fn_line(&n)];
    lines.extend(own_cells(&n, 3));
    lines.push(entry(&n, "function", f("earth", "region", "rainfall", "world", "monthly"), Some("CHIRPS")));
    publish(&lite, "rain.jsonl", &lines, NOW).unwrap_or_else(|e| panic!("{e}"));
    build(&lite, T3);
    let held = held_of(lite).await;
    assert_eq!(held, format!("This mesh holds earth (monthly rainfall computed from CHIRPS). {TAIL}"));
}

/// Own data that declares no `source_name` names none; a relay still reads "from <name>".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h52_own_data_without_a_source_name_names_none_and_a_relay_still_reads_from() {
    let lite = Lite::new("h52-own-and-relay");
    let own = host_name("register/dataset_count");
    let relay = host_name("relay/treasury.gov/par_yield");
    let mut lines = vec![fn_line(&own), fn_line(&relay)];
    lines.extend(own_cells(&own, 2));
    lines.extend(relay_cells(&relay, 2));
    lines.push(entry(&own, "function", f("registry", "dataset", "licence", "world", "on_change"), None));
    lines.push(entry(&relay, "function", f("finance", "yield", "yield", "us", "daily"), Some("the US Treasury")));
    publish(&lite, "own.jsonl", &lines, NOW).unwrap_or_else(|e| panic!("{e}"));
    build(&lite, T3);
    let held = held_of(lite).await;
    assert_eq!(held, format!("This mesh holds finance (yields from the US Treasury) and registry (licences). {TAIL}"));
}

const PRODUCTION: &str = "This mesh holds finance (exchange rates from the ECB, yields from the US Treasury, interest rates from the ECB and the Bank of England), scripture (texts, provenance), earth (monthly rainfall computed from CHIRPS) and registry (licences). Ask find in plain words for what you need.";

/// The full sentence, from the facets of production's catalogue (`production-catalog.jsonl`): the five relays with their source
/// names, the two scripture classes, the stored rainfall function, the register's dataset class. The cells are the rehearsal's
/// order of size (exchange rates most, then yields, then interest rates; verse over edition). It is the rehearsal's sentence with
/// only two changes: "texts, provenance" and "computed from CHIRPS".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h52_the_production_sentence() {
    let lite = Lite::new("h52-production");
    let relays: [(&str, &str, Value, &str, usize); 5] = [
        (
            "relay/ecb.europa.eu/fx_reference",
            "the ECB",
            f("finance", "currency", "exchange_rate", "ea", "daily"),
            "",
            30,
        ),
        ("relay/ecb.europa.eu/estr", "the ECB", f("finance", "rate", "interest_rate", "ea", "daily"), "", 3),
        ("relay/ecb.europa.eu/policy_rate", "the ECB", f("finance", "rate", "interest_rate", "ea", "on_change"), "", 3),
        (
            "relay/bankofengland.co.uk/bank_rate",
            "the Bank of England",
            f("finance", "rate", "interest_rate", "gb", "daily"),
            "",
            3,
        ),
        ("relay/treasury.gov/par_yield", "the US Treasury", f("finance", "yield", "yield", "us", "daily"), "", 20),
    ];
    let mut lines = Vec::new();
    for (n, _, _, _, _) in &relays {
        lines.push(fn_line(&host_name(n)));
    }
    let rain = host_name("earth/monthly_rainfall");
    lines.push(fn_line(&rain));
    for (n, _, _, _, cells) in &relays {
        lines.extend(relay_cells(&host_name(n), *cells));
    }
    lines.extend(own_cells(&rain, 3));
    for (n, source, facets, _, _) in &relays {
        lines.push(entry(&host_name(n), "function", facets.clone(), Some(source)));
    }
    lines.push(entry(&rain, "function", f("earth", "region", "rainfall", "world", "monthly"), Some("CHIRPS")));
    publish(&lite, "functions.jsonl", &lines, NOW).unwrap_or_else(|e| panic!("{e}"));
    // the classes: scripture's two, and the register's
    let (verse, edition, dataset) =
        (host_name("scripture/verse"), host_name("scripture/edition"), host_name("register/dataset"));
    publish(&lite, "classes.jsonl", &[class_lines(&edition), class_lines(&verse), class_lines(&dataset)], T1)
        .unwrap_or_else(|e| panic!("classes: {e}"));
    let mut cls = class_cells(&verse, 50);
    cls.extend(class_cells(&edition, 5));
    cls.extend(class_cells(&dataset, 2));
    cls.push(entry(&verse, "class", f("scripture", "verse", "text", "world", "static"), None));
    cls.push(entry(&edition, "class", f("scripture", "edition", "provenance", "world", "static"), None));
    cls.push(entry(&dataset, "class", f("registry", "dataset", "licence", "world", "on_change"), None));
    publish(&lite, "cells.jsonl", &cls, T2).unwrap_or_else(|e| panic!("cells: {e}"));
    build(&lite, T3);
    let held = held_of(lite).await;
    assert_eq!(held, PRODUCTION);
}
