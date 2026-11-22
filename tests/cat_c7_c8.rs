//! The catalogue, phase 1 (CONTRACT-classes-round.md), C7 and C8: `describe` of the catalogue root and the `held` text of
//! `find?summary=1`, both read from the catalogue the job built (the cells of `catalog/subject`, which is also what C9's pages
//! read), over the scratch mesh of C6 (`catworld::scratch`).
//!
//! C7. `describe/<host>/catalog` answers in one call with `{"kind": "catalog", "host", "domains": [{"domain", "label",
//! "subjects", "names", "top": [{"subject": <key encoding>, "label", "source": <full name>}, ... up to 5]}], "rule": "<url of
//! /catalog/rule>"}`; domains by their number of subjects descending, then by name.
//!
//! C8. `find?summary=1` gains `held`, generated from the catalogue: domains by size, each with up to three measures and their
//! rank-1 sources, in at most 60 words; with no facets it is as today (no `held`, the client's own text).
//!
//! Choices where the contract is silent (each is in the report):
//! * `names` is the number of catalogue names (entries) that have the domain among their facets.
//! * `top` is the domain's subjects by number of sources descending, then by key encoding ascending (the encoding's names are
//!   ascending, so by measure, then region, then thing); the rank-1 source of a subject is its `source`.
//! * A domain's `label` is its value with `_` as a space.
//! * `held` is a string of whitespace-separated words; a domain appears as its label, a measure as its label, a source as its
//!   full name (one word). The measures of a domain: by number of subjects descending, then by name. A measure shows the
//!   rank-1 source of its subjects (at least one; never a source of another rank). The domains that do not fit in 60 words are
//!   left out, the smaller first. Without facets there is no `held` member and the rest of the summary is unchanged.
//! * The catalogue is the job's cells: before the job has run there is nothing to generate from (not tested).

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;

use std::collections::BTreeSet;

use catjob::*;
use catworld::*;
use common::lite::*;
use common::readpath::*;
use serde_json::Value;

/// World S, the job run, served.
async fn served_s(label: &str) -> Served {
    let lite = scratch(label);
    build(&lite, T3);
    serve(lite, &[]).await
}

/// The expected domains of world S: `(domain, subjects, names)` in the order C7 says.
fn s_domains() -> Vec<(&'static str, usize, usize)> {
    vec![("finance", 6, 7), ("scripture", 2, 2), ("earth", 1, 1), ("registry", 1, 1), ("tools", 1, 2)]
}

/// The subjects of a domain of world S in the order of `top`: by number of sources descending, then by key encoding.
fn expected_top(domain: &str) -> Vec<(String, String)> {
    let mut subs: Vec<Subject> = s_subjects().into_iter().filter(|s| s.domain == domain).collect();
    subs.sort_by(|a, b| {
        b.sources
            .len()
            .cmp(&a.sources.len())
            .then_with(|| sk(a.domain, a.thing, a.measure, a.region).cmp(&sk(b.domain, b.thing, b.measure, b.region)))
    });
    subs.into_iter().take(5).map(|s| (sk(s.domain, s.thing, s.measure, s.region), s.sources[0].clone())).collect()
}

fn members(v: &Value) -> Vec<&str> {
    let mut k: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    k.sort();
    k
}

// ---- C7 ---------------------------------------------------------------------------------------------------------------

/// The shape: `kind`, `host`, `domains`, `rule`, and nothing else; each domain's members and each `top` entry's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c7_describe_of_the_catalogue_root_has_the_shape() {
    let m = served_s("c7-shape").await;
    let a = describe(&m, &format!("{HOST}/catalog")).await;
    assert_eq!((a.status, a.kind()), (200, Some("catalog")), "{}", a.text);
    assert_eq!(members(&a.v), ["domains", "host", "kind", "rule"], "{}", a.text);
    assert_eq!(a.v["host"], HOST);
    assert_eq!(a.v["rule"], format!("https://{HOST}/catalog/rule"), "{}", a.text);
    let domains = a.v["domains"].as_array().unwrap();
    assert!(!domains.is_empty());
    for d in domains {
        assert_eq!(members(d), ["domain", "label", "names", "subjects", "top"], "{d}");
        assert!(d["subjects"].is_u64() && d["names"].is_u64(), "counts are numbers: {d}");
        for t in d["top"].as_array().unwrap() {
            assert_eq!(members(t), ["label", "source", "subject"], "{t}");
        }
    }
}

/// The domains are ordered by their number of subjects, descending, then by name; the counts are those of the world, and the
/// labels are the domain with `_` as a space.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c7_the_domains_are_by_subject_count_then_name() {
    let m = served_s("c7-order").await;
    let a = describe(&m, &format!("{HOST}/catalog")).await;
    let got: Vec<(String, u64, u64)> = a.v["domains"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (d["domain"].as_str().unwrap().to_string(), d["subjects"].as_u64().unwrap(), d["names"].as_u64().unwrap())
        })
        .collect();
    let want: Vec<(String, u64, u64)> =
        s_domains().into_iter().map(|(d, s, n)| (d.to_string(), s as u64, n as u64)).collect();
    assert_eq!(got, want, "{}", a.text);
    for d in a.v["domains"].as_array().unwrap() {
        assert_eq!(d["label"], d["domain"].as_str().unwrap().replace('_', " "), "{d}");
    }
}

/// The counts and the `top` entries agree with the cells: the number of subjects of a domain is the number of its `exists()`
/// cells that are true, each `top` entry's label is the cell `label()` and its source the cell `source(1)`, the entries are at
/// most 5, the best first (more sources first, then the key encoding), and the answer needed no other call.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c7_the_counts_and_the_top_equal_the_cells() {
    let m = served_s("c7-cells").await;
    let a = describe(&m, &format!("{HOST}/catalog")).await;
    for d in a.v["domains"].as_array().unwrap() {
        let domain = d["domain"].as_str().unwrap();
        // the subjects of the domain, read from the cells
        let mut live = 0;
        for s in s_subjects().into_iter().filter(|s| s.domain == domain) {
            let e = cell(&m, &sc(s.domain, s.thing, s.measure, s.region, "exists()")).await;
            if truth(&e) == Some(true) {
                live += 1;
            }
        }
        assert_eq!(d["subjects"], live, "{domain}: subjects equal the cells that exist");
        let top = d["top"].as_array().unwrap();
        let want = expected_top(domain);
        assert!(top.len() <= 5, "{domain}: at most 5");
        assert_eq!(top.len(), live.min(5), "{domain}: all of a small domain, 5 of a large one");
        let got: Vec<(String, String)> = top
            .iter()
            .map(|t| (t["subject"].as_str().unwrap().to_string(), t["source"].as_str().unwrap().to_string()))
            .collect();
        assert_eq!(got, want, "{domain}: the top by sources, then by key encoding");
        for t in top {
            // the label and source of an entry are those of its cells
            let enc = t["subject"].as_str().unwrap();
            let s = s_subjects()
                .into_iter()
                .find(|s| sk(s.domain, s.thing, s.measure, s.region) == enc)
                .unwrap_or_else(|| panic!("{enc} is no subject of the world"));
            let l = cell(&m, &sc(s.domain, s.thing, s.measure, s.region, "label()")).await;
            assert_eq!(t["label"].as_str(), l.value(), "{enc}");
            let src = cell(&m, &sc(s.domain, s.thing, s.measure, s.region, "source(1)")).await;
            assert_eq!(t["source"].as_str(), src.value(), "{enc}");
        }
        // no duplicate subject
        let uniq: BTreeSet<&str> = top.iter().map(|t| t["subject"].as_str().unwrap()).collect();
        assert_eq!(uniq.len(), top.len(), "{domain}");
    }
}

/// The root is not a class or a function name: it answers for the host's catalogue only, and a name under it is unknown.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c7_other_names_under_the_root_are_not_the_catalogue() {
    let m = served_s("c7-other").await;
    let a = describe(&m, &format!("{HOST}/catalog/nothing")).await;
    assert_eq!(a.status, 404, "{}", a.text);
}

// ---- C8 ---------------------------------------------------------------------------------------------------------------

fn tokens(held: &str) -> Vec<String> {
    held.split_whitespace().map(|w| w.trim_matches(|c: char| ",;:.()".contains(c)).to_string()).collect()
}

async fn held_of(m: &Served) -> String {
    let r = until_built(m, "/find?summary=1").await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = r.json();
    v["held"].as_str().unwrap_or_else(|| panic!("no held in {v}")).to_string()
}

/// `held`: at most 60 words, the domains by size (finance 6, scripture 2, then earth, registry, tools by name), and the rest of
/// the summary as it was.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c8_held_is_at_most_sixty_words_with_the_domains_by_size() {
    let m = served_s("c8-order").await;
    let held = held_of(&m).await;
    assert!(held.split_whitespace().count() <= 60, "{} words: {held}", held.split_whitespace().count());
    let t = tokens(&held);
    let at = |d: &str| t.iter().position(|x| x == d).unwrap_or_else(|| panic!("{d} is not in held: {held}"));
    let order: Vec<usize> = ["finance", "scripture", "earth", "registry", "tools"].iter().map(|d| at(d)).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "domains by size, then name: {held}");
    // the summary's other members are there as before
    let v = m.get("/find?summary=1").await;
    assert!(v["classes"].is_array() && v["functions"].is_array(), "{v}");
}

/// With no facets there is no `held`: the summary is what it was (the client's text is built from `classes` and `functions`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c8_without_facets_the_summary_is_as_today() {
    let m = serve_lite("c8-plain", &lexicon_and_zoo(), None, |_| {}).await;
    let r = until_built(&m, "/find?summary=1").await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = r.json();
    assert!(v.get("held").is_none(), "no facets, no held: {v}");
    assert!(v["classes"].is_array() && v["functions"].is_array(), "{v}");
    assert!(v["functions"].as_array().unwrap().iter().any(|e| e["name"] == VERSE), "{v}");
}
