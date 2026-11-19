//! Build 2 Part 2, R3: value resolution. The fixture is synthetic and scripture-shaped
//! (`common/readpath.rs`): `lexicon/verse(book, chapter, verse)` with a publisher dictionary
//! `John`, `Gen` ("Genesis", alias `Gn`), `2Chr` ("2 Chronicles"), `Jer` ("Jérémie") and `Prim`
//! ("Bible Primer"); the text of its cells is invented. SQLite store, no embed service.
//!
//! `resolution` is `{"status": "exact", "call", "label"}`, `{"status": "ambiguous", "candidates":
//! [...], "keys"?: [{"key", "candidates": [{"value", "label", "score"}]}]}`, `{"status": "range",
//! "error": "E_RANGE_REFUSED"}` or `{"status": "none"}` (the member may be absent for a query that
//! is not a request).

mod common;

use common::readpath::*;
use serde_json::Value;

async fn served(label: &str) -> Served {
    serve_lite(label, &lexicon_and_zoo(), None, |_| {}).await
}

fn resolved_call(v: &Value) -> String {
    assert_eq!(status(v), "exact", "{v}");
    norm(v["resolution"]["call"].as_str().unwrap())
}

fn filled(book: &str, chapter: u32, verse: u32) -> String {
    norm(&format!("{VERSE}(book: \"{book}\", chapter: {chapter}, verse: {verse})"))
}

/// `2Chr 1:1`: a digit-led book code is a value, exactly as written; the numbers fill the number
/// keys in signature order. The label is human (the dictionary's label of the book).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_digit_led_code_is_a_value() {
    let m = served("rs-1").await;
    let v = m.find("2Chr 1:1").await;
    assert_eq!(resolved_call(&v), filled("2Chr", 1, 1));
    let label = v["resolution"]["label"].as_str().unwrap();
    assert!(label.contains("2 Chronicles") && label.contains("1:1"), "{label}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn john_3_16_and_gen_1_1_resolve_exactly() {
    let m = served("rs-2").await;
    let v = m.find("John 3:16").await;
    assert_eq!(resolved_call(&v), filled("John", 3, 16));
    assert!(v["resolution"]["label"].as_str().unwrap().contains("John 3:16"), "{v}");
    let v = m.find("Gen 1:1").await;
    assert_eq!(resolved_call(&v), filled("Gen", 1, 1));
    // case-insensitive
    assert_eq!(resolved_call(&m.find("john 3:16").await), filled("John", 3, 16));
}

/// "Genesis 1:1": the label "Genesis" of `Gen` is a labelled dictionary match; so is a publisher alias.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_label_or_an_alias_fills_the_value() {
    let m = served("rs-3").await;
    let v = m.find("Genesis 1:1").await;
    assert_eq!(resolved_call(&v), filled("Gen", 1, 1));
    assert!(v["resolution"]["label"].as_str().unwrap().contains("Genesis 1:1"), "{v}");
    assert_eq!(resolved_call(&m.find("Gn 1:1").await), filled("Gen", 1, 1));
    assert_eq!(resolved_call(&m.find("2 Chronicles 1:1").await), filled("2Chr", 1, 1), "a two-word label");
}

/// Compared after NFC: a label typed with combining marks is the label.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn matching_is_after_nfc() {
    let m = served("rs-4").await;
    let nfd = "Je\u{301}re\u{301}mie 5:1";
    assert_eq!(resolved_call(&m.find(nfd).await), filled("Jer", 5, 1));
    assert_eq!(resolved_call(&m.find("J\u{e9}r\u{e9}mie 5:1").await), filled("Jer", 5, 1));
}

/// Generic words stay out of value matching: `bible` is in the description of the function and
/// in the fixed list, so "bible verse" never fills the book with `Prim` ("Bible Primer").
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bible_verse_never_fills_a_book() {
    let m = served("rs-5").await;
    for q in ["bible verse", "bible", "verse", "the verse of the bible", "bible verses"] {
        let v = m.find(q).await;
        assert_ne!(status(&v), "exact", "{q}: {v}");
        for r in results(&v) {
            assert!(!r["call"].as_str().unwrap().contains("\"Prim\""), "{q}: a filled book in {r}");
        }
        if let Some(res) = v.get("resolution") {
            let rs = res.to_string();
            let rs = norm(&rs.replace("\\\"", "\""));
            assert!(!rs.contains("book:\"Prim\""), "{q}: {rs}");
        }
    }
    // The generic words do not stop the real ones: still resolves, with the book from `John`.
    let v = m.find("bible verse John 3:16").await;
    assert_eq!(resolved_call(&v), filled("John", 3, 16));
    // And `Prim` itself, spelled as a value, is an exact match.
    assert_eq!(resolved_call(&m.find("Prim 1:2").await), filled("Prim", 1, 2));
}

/// "verse 3:16" has the numbers but no book: not exact, nothing filled for `book`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_missing_book_is_not_guessed() {
    let m = served("rs-6").await;
    let v = m.find("verse 3:16").await;
    assert_ne!(status(&v), "exact", "{v}");
    if let Some(cands) = v["resolution"]["candidates"].as_array() {
        for c in cands {
            if let Some(call) = c["call"].as_str() {
                assert!(call.contains("book: <string>") || call.contains("book:<string>"), "book filled: {c}");
            }
            if let Some(missing) = c["missing_keys"].as_array() {
                assert!(missing.iter().any(|k| k == "book"), "{c}");
            }
        }
    }
}

/// A range is refused, never shortened: `3:16-18`, `16-18` with an en dash, `1-5`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_range_is_refused() {
    let m = served("rs-7").await;
    for q in ["John 3:16-18", "John 3 16\u{2013}18", "Gen 1-5", "Genesis 1:1-5", "John 3:16\u{2013}18"] {
        let v = m.find(q).await;
        assert_eq!(status(&v), "range", "{q}: {v}");
        assert_eq!(v["resolution"]["error"], "E_RANGE_REFUSED", "{q}: {v}");
        assert!(v["resolution"].get("call").is_none(), "{q}: a call was given for a range: {v}");
        for r in results(&v) {
            assert!(!r["call"].as_str().unwrap().contains("verse: 16"), "{q}: shortened range in {r}");
        }
    }
    // a single verse next to it is fine (the dash of a word is not a range)
    assert_eq!(status(&m.find("John 3:16").await), "exact");
}

/// The `/find` answer of a resolved request lists the filled call among the results, as a result
/// of the function with `call` filled.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_resolved_call_is_a_result_with_its_label() {
    let m = served("rs-8").await;
    let v = m.find("John 3:16").await;
    let r = result(&v, VERSE);
    assert_eq!(norm(r["call"].as_str().unwrap()), filled("John", 3, 16), "{r}");
    assert!(r["label"].as_str().unwrap().contains("John 3:16"), "{r}");
    assert_eq!(r["match"], "exact");
}

/// The resolved call is a call the mesh answers (the cell exists in the fixture, invented text).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_resolved_call_reads_the_cell() {
    let m = served("rs-9").await;
    let v = m.find("Genesis 1:1").await;
    let call = v["resolution"]["call"].as_str().unwrap().to_string();
    let r = m.get(&format!("/cell/{}", common::world::call_path(&call))).await;
    assert_eq!(r["outcome"], "ok", "{r}");
    assert!(r.to_string().contains("delta epsilon zeta"), "{r}");
}

/// A query that names no function and no value has no resolution of status exact.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_to_resolve() {
    let m = served("rs-10").await;
    let v = m.find("zebrafish").await;
    assert!(matches!(status(&v).as_str(), "none"), "{v}");
}
