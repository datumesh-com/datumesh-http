//! http 0.4.9, CONTRACT-0.4.9 H7: the rule for dictionary values.
//!
//! 1. Only a key of type `string` can have dictionary values: date, number and boolean keys never do.
//! 2. A publisher's own `dictionary` is stored as given.
//! 3. A string key gets a value automatically from published cells only while the function has at most
//!    `DATUMESH_DICTIONARY_MAX` (default 1,000) distinct values for it.
//! 4. A publish whose new values would take a key past the limit adds none of them, and marks the key open
//!    (store `mark_key_open`); an open key never gets automatic values again; existing values stay.
//! 5. A catalogue entry's `"open_keys": [...]` makes those keys open from the first cell.
//! 7. The lookup per value is exact (`has_key_value`).
//! `GET /metrics` gains `datumesh_dictionary_open_keys_total`.
//!
//! These tests use the new store 0.5.2 methods `key_value_count`, `has_key_value` and `key_is_open`, and FAIL
//! until the rule is in.
//!
//! The limit is the process environment variable `DATUMESH_DICTIONARY_MAX`, set to 5 for the whole of this
//! test binary before anything runs (the library's `publish_stored_with` has no configuration: it reads the
//! variable when it needs the limit; `Config::from_vars` reads it as well).

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use std::sync::Once;

use common::lite::*;
use common::pub031::{function_line, run, write, T1, T2};
use common::readpath::*;
use datumesh_store::Store;
use publish4::get;
use serde_json::{json, Value};

const MAX: usize = 5;

fn limit_is_five() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| std::env::set_var("DATUMESH_DICTIONARY_MAX", MAX.to_string()));
}

const D: &str = "mesh-test.datumesh.com/h049/dict/d";
const E: &str = "mesh-test.datumesh.com/h049/dict/e";
const G: &str = "mesh-test.datumesh.com/h049/dict/g";
const H: &str = "mesh-test.datumesh.com/h049/dict/h";

/// `f(site: string, day: date, n: number)`.
fn sig() -> String {
    r#"{"args":{"day":{"type":"date","valid":true},"n":{"type":"number","unit":"1","valid":false},"site":{"type":"string","valid":false}},"order":["site","day","n"],"returns":{"type":"number","unit":"1"}}"#.to_string()
}

fn cell(f: &str, site: &str, day: &str, n: &str) -> String {
    json!({"cell": {"fn": f, "args": {"site": {"type": "string", "value": site},
            "day": {"type": "date", "value": day}, "n": {"type": "number", "value": n}}},
        "licence": "CC0-1.0", "summary": "Synthetic test data.", "value": {"type": "number", "value": "1"}})
    .to_string()
}

fn sites(f: &str, names: &[&str]) -> Vec<String> {
    names.iter().enumerate().map(|(i, s)| cell(f, s, &format!("2026-03-{:02}", i + 1), &i.to_string())).collect()
}

fn publish(lite: &Lite, file: &str, lines: &[String], at: &'static str) {
    let f = write(lite, file, lines);
    run(lite, &f, at).unwrap_or_else(|e| panic!("{file}: {e}"));
}

fn count(lite: &Lite, f: &str, key: &str) -> u64 {
    lite.store.key_value_count(f, key).unwrap()
}

fn has(lite: &Lite, f: &str, key: &str, v: &str) -> bool {
    lite.store.has_key_value(f, key, v).unwrap()
}

fn open(lite: &Lite, f: &str, key: &str) -> bool {
    lite.store.key_is_open(f, key).unwrap()
}

#[test]
fn only_a_string_key_has_dictionary_values() {
    limit_is_five();
    let lite = Lite::new("h7-types");
    publish(&lite, "a.jsonl", &[vec![function_line(D, &sig(), None)], sites(D, &["s1", "s2", "s3"])].concat(), NOW);
    assert_eq!(count(&lite, D, "site"), 3);
    assert!(has(&lite, D, "site", "s2"));
    // dates and numbers are typed values: never dictionary values, whatever the limit
    assert_eq!(count(&lite, D, "day"), 0);
    assert_eq!(count(&lite, D, "n"), 0);
    assert!(!has(&lite, D, "day", "2026-03-01"));
    assert!(!has(&lite, D, "n", "1"));
    assert!(!open(&lite, D, "site"));
    assert!(!open(&lite, D, "day") && !open(&lite, D, "n"));
}

#[test]
fn a_key_takes_values_up_to_the_limit_then_none_and_is_open() {
    limit_is_five();
    let lite = Lite::new("h7-limit");
    publish(&lite, "a.jsonl", &[vec![function_line(D, &sig(), None)], sites(D, &["s1", "s2", "s3"])].concat(), NOW);
    // two more: five in all, the limit itself is allowed
    publish(&lite, "b.jsonl", &sites(D, &["s4", "s5"]), T1);
    assert_eq!(count(&lite, D, "site"), 5);
    assert!(!open(&lite, D, "site"), "at the limit, not past it");
    // one more would make six: none is added, and the key is open
    publish(&lite, "c.jsonl", &sites(D, &["s6"]), T2);
    assert_eq!(count(&lite, D, "site"), 5, "existing values stay, none is added");
    assert!(!has(&lite, D, "site", "s6"));
    assert!(open(&lite, D, "site"));
    // later publishes add none, even one value, even one that would fit
    publish(&lite, "d.jsonl", &sites(D, &["s7"]), "2026-11-15T12:00:03.000000Z");
    assert_eq!(count(&lite, D, "site"), 5);
    assert!(!has(&lite, D, "site", "s7"));
    // the values it had are all still there
    for s in ["s1", "s2", "s3", "s4", "s5"] {
        assert!(has(&lite, D, "site", s), "{s}");
    }
    // the cells themselves are published all the same
    assert!(lite.store.resolve_binding(D, END).unwrap().is_some());
}

#[test]
fn a_publish_that_would_exceed_adds_none_of_its_values() {
    limit_is_five();
    let lite = Lite::new("h7-batch");
    // eight new values in one file: more than the limit, so none, and the key is open from the start
    let names = ["a1", "a2", "a3", "a4", "a5", "a6", "a7", "a8"];
    publish(&lite, "a.jsonl", &[vec![function_line(E, &sig(), None)], sites(E, &names)].concat(), NOW);
    assert_eq!(count(&lite, E, "site"), 0);
    assert!(open(&lite, E, "site"));
    // three then four: the second would take it to seven, so none of the four
    let lite2 = Lite::new("h7-batch2");
    publish(&lite2, "a.jsonl", &[vec![function_line(G, &sig(), None)], sites(G, &["x1", "x2", "x3"])].concat(), NOW);
    publish(&lite2, "b.jsonl", &sites(G, &["x4", "x5", "x6", "x7"]), T1);
    assert_eq!(count(&lite2, G, "site"), 3, "none of the four, not the first two of them");
    assert!(open(&lite2, G, "site"));
}

#[test]
fn a_value_already_known_is_not_new() {
    limit_is_five();
    let lite = Lite::new("h7-known");
    publish(&lite, "a.jsonl", &[vec![function_line(D, &sig(), None)], sites(D, &["s1", "s2", "s3", "s4", "s5"])].concat(), NOW);
    // the same values again, with other dates: nothing new, nothing past the limit, the key stays closed
    let again: Vec<String> =
        ["s1", "s2", "s3"].iter().map(|s| cell(D, s, "2027-01-01", "9")).collect();
    publish(&lite, "b.jsonl", &again, T1);
    assert_eq!(count(&lite, D, "site"), 5);
    assert!(!open(&lite, D, "site"));
}

#[test]
fn a_catalogue_entry_may_name_the_open_keys() {
    limit_is_five();
    let lite = Lite::new("h7-open-keys");
    let entry = json!({"catalog": H, "kind": "function", "label": "H", "description": "An open-key function.",
        "args": {"site": "the site"}, "examples": [], "dictionary": {}, "open_keys": ["site"]})
    .to_string();
    // the entry first, then the cells: the key is open from the first cell
    publish(&lite, "a.jsonl", &[vec![function_line(H, &sig(), None), entry], sites(H, &["p1", "p2"])].concat(), NOW);
    assert!(open(&lite, H, "site"));
    assert_eq!(count(&lite, H, "site"), 0, "no automatic value");
    publish(&lite, "b.jsonl", &sites(H, &["p3"]), T1);
    assert_eq!(count(&lite, H, "site"), 0);
}

#[test]
fn a_publishers_own_dictionary_is_stored_as_given() {
    limit_is_five();
    let lite = Lite::new("h7-own");
    let dict: Vec<Value> = (0..8).map(|i| json!({"value": format!("w{i}"), "label": format!("Site {i}")})).collect();
    let entry = catalog_line(G, "function", "G", "A function with a dictionary.", json!({"site": "the site"}), json!({"site": dict}));
    publish(&lite, "a.jsonl", &[function_line(G, &sig(), None), entry], NOW);
    assert_eq!(count(&lite, G, "site"), 8, "more than the limit, as given");
    // cells with values the dictionary has change nothing; a value it has not is not added (past the limit)
    publish(&lite, "b.jsonl", &sites(G, &["w1", "w2", "zz"]), T1);
    assert_eq!(count(&lite, G, "site"), 8);
    assert!(!has(&lite, G, "site", "zz"));
    assert!(has(&lite, G, "site", "w7"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_metrics_count_the_open_keys() {
    limit_is_five();
    let lite = Lite::new("h7-metrics");
    // two open keys: one by the limit, one by the entry
    let entry = json!({"catalog": H, "kind": "function", "label": "H", "description": "An open-key function.",
        "args": {"site": "the site"}, "examples": [], "dictionary": {}, "open_keys": ["site"]})
    .to_string();
    publish(
        &lite,
        "a.jsonl",
        &[
            vec![function_line(E, &sig(), None), function_line(H, &sig(), None), entry],
            sites(E, &["a1", "a2", "a3", "a4", "a5", "a6"]),
            sites(H, &["p1"]),
        ]
        .concat(),
        NOW,
    );
    assert!(open(&lite, E, "site") && open(&lite, H, "site"));
    let cfg = datumesh_http::Config::from_vars(|k| match k {
        "DATUMESH_HOST" => Some(HOST.to_string()),
        "DATUMESH_DICTIONARY_MAX" => Some(MAX.to_string()),
        _ => None,
    })
    .unwrap();
    let m = build_served(lite, Some(cfg), |_| {}).await;
    let t = get(&m, "/metrics", &[]).await.text();
    assert!(t.contains("# TYPE datumesh_dictionary_open_keys_total"), "{t}");
    let n: u64 = t
        .lines()
        .find_map(|l| l.strip_prefix("datumesh_dictionary_open_keys_total").map(|r| r.trim().parse().unwrap()))
        .unwrap_or_else(|| panic!("no sample in: {t}"));
    assert_eq!(n, 2, "the number of open keys the store holds: {t}");
}
