//! Build 2 Part 2, R3: `GET /find` is hybrid. These tests need no vector index: an SQLite store,
//! the fixture published with `catalog` lines, and, where an embed service matters, the fake of
//! `common/fake_embed.rs`. The vector side is in `find_vector.rs` (pgvector).

mod common;

use common::fake_embed::{dead_url, FakeEmbed};
use common::readpath::*;
use serde_json::json;

fn s<'a>(v: &'a serde_json::Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_else(|| panic!("no string {k} in {v}"))
}

/// Exact beats text: the query `kestrel` is the label of `bird_a` (exact) and a word of the
/// description of `bird_b` (text). `match` says which, and the exact one is first.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exact_beats_text() {
    let m = serve_lite("hy-1", &lexicon_and_zoo(), None, |_| {}).await;
    let v = m.find("kestrel").await;
    assert!(pos(&v, BIRD_A) < pos(&v, BIRD_B), "{:?}", names(&v));
    assert_eq!(s(result(&v, BIRD_A), "match"), "exact");
    assert_eq!(s(result(&v, BIRD_B), "match"), "text");
    assert_eq!(results(&v)[0]["name"], BIRD_A);
    // Case-insensitive, and after NFC: the exact match of the label is the same for `KESTREL`.
    let v = m.find("KESTREL").await;
    assert_eq!(s(result(&v, BIRD_A), "match"), "exact");
}

/// An exact match is a name, label, alias or value equal to the query or to one of its tokens:
/// the alias `Gn` finds the function whose dictionary has it, as an exact match.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_alias_or_a_value_is_an_exact_match_of_its_function() {
    let m = serve_lite("hy-2", &lexicon_and_zoo(), None, |_| {}).await;
    for q in ["Gn", "2Chr", "genesis", "Code 07"] {
        let v = m.find(q).await;
        let hit = results(&v).into_iter().find(|r| r["match"] == "exact");
        assert!(hit.is_some(), "{q}: no exact result in {v}");
    }
    let v = m.find("Gn").await;
    assert!(names(&v).contains(&VERSE.to_string()), "{:?}", names(&v));
}

/// A text match means every query word appears in a name, label, description or argument line.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_text_match_needs_every_word_in_name_label_description_or_argument_line() {
    let m = serve_lite("hy-3", &lexicon_and_zoo(), None, |_| {}).await;
    // description
    assert_eq!(s(result(&m.find("weekly kestrel").await, BIRD_B), "match"), "text");
    // a word in the description and a word in the label: still one entry
    assert!(names(&m.find("synthetic verses").await).contains(&VERSE.to_string()));
    // an argument line (`pilgrimage` is only in the line of `verse`)
    let v = m.find("pilgrimage").await;
    assert_eq!(names(&v), [VERSE], "{v}");
    assert_eq!(s(results(&v)[0], "match"), "text");
    // one word missing: no text match
    assert!(!names(&m.find("weekly zebrafish").await).contains(&BIRD_B.to_string()));
}

/// Every result has `call` and `label`, a `match` of the three kinds, a numeric `score`, and keeps
/// `kind`, `name`, `description`; a function's `call` is the template with typed placeholders.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_result_has_call_and_label() {
    let m = serve_lite("hy-4", &lexicon_and_zoo(), None, |_| {}).await;
    for q in ["verse", "kestrel", "pilgrimage", "John 3:16", "coded", "bird"] {
        let v = m.find(q).await;
        assert!(v["query"] == q, "{v}");
        assert!(v["vector"].is_boolean(), "vector is a bool: {v}");
        for r in results(&v) {
            assert!(!s(r, "call").is_empty() && !s(r, "label").is_empty(), "{q}: {r}");
            assert!(["exact", "text", "vector"].contains(&s(r, "match")), "{q}: {r}");
            assert!(r["score"].is_number(), "{q}: {r}");
            for k in ["kind", "name", "description"] {
                assert!(r[k].is_string(), "{q}: {k} in {r}");
            }
        }
    }
    let v = m.find("pilgrimage").await;
    let r = result(&v, VERSE);
    assert_eq!(s(r, "call"), VERSE_TEMPLATE);
    assert_eq!(s(r, "label"), "Synthetic Verses");
    assert_eq!(s(r, "kind"), "function");
    // the members that keep their meaning
    for k in ["descriptor", "fn_hash", "since"] {
        assert!(r.get(k).is_some(), "{k} in {r}");
    }
}

/// The default `limit` is 10 and the most 50.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn limit_defaults_to_10_and_stops_at_50() {
    let mut lines = lexicon_lines();
    lines.extend(zoo_lines(60));
    let m = serve_lite("hy-5", &lines, None, |_| {}).await;
    assert_eq!(results(&m.find("flock").await).len(), 10);
    assert_eq!(results(&m.get("/find?q=flock&limit=500").await).len(), 50);
    assert_eq!(results(&m.get("/find?q=flock&limit=3").await).len(), 3);
}

/// The embed service is down (nothing listens): `vector` is false, the answer is a 200 with the
/// text results, and there is no error member.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn embed_service_down_gives_vector_false_and_no_error() {
    let m = serve_lite("hy-6", &lexicon_and_zoo(), Some(config_with_embed(&dead_url())), |_| {}).await;
    let (status, body) = m.raw("/find?q=kestrel").await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["vector"], json!(false), "{v}");
    assert!(v.get("error").is_none(), "{v}");
    assert_eq!(s(results(&v)[0], "match"), "exact");
    assert!(names(&v).contains(&BIRD_B.to_string()));
}

/// An embed service that answers 503, or that is slower than the 2 s timeout, is the same.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn embed_service_failing_or_slow_gives_vector_false() {
    let fake = FakeEmbed::start();
    let m = serve_lite("hy-7", &lexicon_and_zoo(), Some(config_with_embed(&fake.url())), |_| {}).await;
    fake.set_fail(true);
    let v = m.find("kestrel").await;
    assert_eq!(v["vector"], json!(false), "{v}");
    assert_eq!(s(results(&v)[0], "match"), "exact");
    fake.set_fail(false);
    fake.set_delay(6_000);
    let t = std::time::Instant::now();
    let v = m.find("kestrel").await;
    assert!(t.elapsed() < std::time::Duration::from_millis(4_500), "the client waited {:?}", t.elapsed());
    assert_eq!(v["vector"], json!(false), "{v}");
    assert_eq!(s(results(&v)[0], "match"), "exact");
}

/// No `DATUMESH_EMBED_URL`: vector `find` is off, `vector` is false.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_an_embed_url_vector_is_off() {
    let m = serve_lite("hy-8", &lexicon_and_zoo(), None, |_| {}).await;
    assert_eq!(m.find("kestrel").await["vector"], json!(false));
}

/// `DATUMESH_EMBED_URL` is read by `Config::from_vars` (the env route of `from_env`).
#[test]
fn embed_url_comes_from_the_environment() {
    let c = config_with_embed("http://embed.example:8080");
    let d = format!("{c:?}");
    assert!(d.contains("http://embed.example:8080"), "Config's Debug does not show the embed URL: {d}");
}

/// The operator's catalogue file (`Config::catalog`, `DATUMESH_CATALOG_FILE`) is still read: it
/// describes a function that has no publisher entry (here `bird_a`, whose publisher entry is
/// removed from this fixture), and `/find` shows the operator's description.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_operator_file_describes_what_the_publisher_did_not() {
    let lines: Vec<String> = lexicon_and_zoo()
        .into_iter()
        .filter(|l| !l.contains(r#""catalog":"mesh-test.datumesh.com/scratch_b2/zoo/bird_a""#))
        .collect();
    assert!(lines.len() < lexicon_and_zoo().len());
    let m = serve_lite("hy-9", &lines, None, |c| {
        c.catalog.insert(BIRD_A.to_string(), "Operator words about ospreys".to_string());
    })
    .await;
    let v = m.find("ospreys").await;
    assert_eq!(s(result(&v, BIRD_A), "description"), "Operator words about ospreys");
}

/// The operator file never overrides a publisher entry for the same name: the publisher's
/// description is the one shown and searched.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_operator_file_does_not_override_a_publisher_entry() {
    let m = serve_lite("hy-10", &lexicon_and_zoo(), None, |c| {
        c.catalog.insert(BIRD_A.to_string(), "Operator words about ospreys".to_string());
        c.catalog.insert(VERSE.to_string(), "Operator verse words, nightingale".to_string());
    })
    .await;
    let v = m.find("birds site").await;
    assert_eq!(s(result(&v, BIRD_A), "description"), "Counts of birds at a site");
    assert_eq!(s(result(&v, BIRD_A), "label"), "Kestrel");
    assert!(m.find("ospreys").await["results"].as_array().unwrap().is_empty());
    assert!(m.find("nightingale").await["results"].as_array().unwrap().is_empty());
    let v = m.find("pilgrimage").await;
    assert_eq!(s(result(&v, VERSE), "description"), VERSE_DESC);
}

/// `GET /find?summary=1`: the catalogue summary, a JSON object `{"classes": [...], "functions":
/// [...]}`; each entry `{"name", "label", "description", "cells", "keys": {<key>: {"distinct",
/// "values": [{"value", "label"}]}}}` with at most 50 values per key. No `results`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn summary_lists_classes_and_functions_with_labels_counts_and_small_dictionaries() {
    let m = serve_lite("hy-11", &lexicon_and_zoo(), None, |_| {}).await;
    let v = m.get("/find?summary=1").await;
    assert!(v.get("results").is_none(), "{v}");
    assert!(v["classes"].is_array() && v["functions"].is_array(), "{v}");
    let f = v["functions"].as_array().unwrap();
    let verse = f.iter().find(|e| e["name"] == VERSE).unwrap_or_else(|| panic!("no verse in {v}"));
    assert_eq!(verse["label"], "Synthetic Verses");
    assert_eq!(verse["description"], VERSE_DESC);
    assert_eq!(verse["cells"], json!(3));
    let book = &verse["keys"]["book"];
    let values = book["values"].as_array().unwrap();
    for (value, label) in [("John", "John"), ("Gen", "Genesis"), ("2Chr", "2 Chronicles")] {
        assert!(values.iter().any(|x| x["value"] == value && x["label"] == label), "{value} in {book}");
    }
    // a dictionary of 60 values: at most 50 are listed, `distinct` says how many there are
    let code = f.iter().find(|e| e["name"] == CODE).unwrap();
    assert!(code["keys"]["code"]["values"].as_array().unwrap().len() <= 50, "{code}");
    assert_eq!(code["keys"]["code"]["distinct"], json!(60));
    assert_eq!(code["cells"], json!(0));
    // other forms of `summary` do not make a summary
    assert!(m.get("/find?q=kestrel").await.get("functions").is_none());
}

/// `src/find_stopwords.txt` starts with the brief's generic words, in English.
#[test]
fn the_stopword_list_has_the_generic_words() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/find_stopwords.txt");
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let words: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    for w in ["bible", "verse", "verses", "chapter", "text", "rate", "rates", "price", "the", "of", "in"] {
        assert!(words.contains(&w), "{w} is not in find_stopwords.txt");
    }
    // Arabic, Hebrew and Greek words are there too
    let has = |lo: char, hi: char| words.iter().any(|w| w.chars().any(|c| (lo..=hi).contains(&c)));
    assert!(has('\u{0600}', '\u{06FF}'), "no Arabic word");
    assert!(has('\u{0590}', '\u{05FF}'), "no Hebrew word");
    assert!(has('\u{0370}', '\u{03FF}'), "no Greek word");
}
