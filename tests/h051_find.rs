//! http 0.5.1, CONTRACT-0.5.1 H3: `find` drops one-letter words from a query before matching. A query that has only
//! one-letter words is answered as the empty query. Written first; FAIL until then.
//!
//! * a query of the 26 one-letter words `a b c ... z` does the work of the empty query: the same listing, and the same
//!   reads of the catalogue (the empty query is answered from the find thread's last build, so none);
//! * `a euro rate` gives what `euro rate` gives (the `query` member echoes what was asked, so it is left out of the
//!   comparison); so does a query that opens with nine one-letter words (cut at 8 words today, the words that matter are
//!   lost);
//! * a one-letter word between others (`euro a rate`) is dropped as well.
//!
//! One letter is one character (the test uses ASCII). The fixture is `h0410_find`'s: twenty functions whose texts share the
//! words "a value for the euro reference level".

mod common;
#[path = "common/h0410.rs"]
mod h0410;

use common::lite::*;
use common::pub031::function_line;
use h0410::*;
use serde_json::{json, Value};

const CATALOG: usize = 20;

fn sig() -> String {
    r#"{"args":{"site":{"type":"string","valid":false}},"order":["site"],"returns":{"type":"number","unit":"1"}}"#
        .to_string()
}

fn name(i: usize) -> String {
    format!("mesh-test.datumesh.com/h051/fnd/f{i:02}")
}

fn catalogue(fx: &Fx) {
    let mut lines = Vec::new();
    for i in 0..CATALOG {
        lines.push(function_line(&name(i), &sig(), None));
    }
    for i in 0..CATALOG {
        lines.push(common::readpath::catalog_line(
            &name(i),
            "function",
            &format!("The euro rate of the site {i}"),
            "The rate for the site, a value for the euro reference level.",
            json!({"site": "the site"}),
            json!({}),
        ));
    }
    fx.publish(&lines, NOW);
}

async fn find(w: &W, q: &str) -> Value {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let r = w.get(&format!("/find?q={q}")).await;
        if r.status != 503 || std::time::Instant::now() > end {
            assert_eq!(r.status, 200, "{q}: {}", r.text());
            return r.json();
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// An answer without the echo of the query.
fn without_query(mut v: Value) -> Value {
    v.as_object_mut().unwrap().remove("query");
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h3_a_query_of_one_letter_words_is_the_empty_query() {
    let fx = Fx::pg("h051-letters");
    catalogue(&fx);
    fx.probe().arm();
    // the find thread builds the listing the empty query is answered from; the others are held
    let w = fx.serve(config(&held(&["find"]), &[])).await;
    let empty = find(&w, "").await;
    assert!(!empty["results"].as_array().unwrap().is_empty(), "the empty query lists the entries: {empty}");
    let p = w.fx.probe();
    let (c0, k0) = (p.reads(Group::Catalog), p.reads(Group::Cells));
    let again = find(&w, "").await;
    let (c1, k1) = (p.reads(Group::Catalog), p.reads(Group::Cells));
    assert_eq!(again["results"], empty["results"]);
    let letters: Vec<String> = ('a'..='z').map(|c| c.to_string()).collect();
    let q = letters.join("%20");
    let v = find(&w, &q).await;
    let (c2, k2) = (p.reads(Group::Catalog), p.reads(Group::Cells));
    assert_eq!(v["results"], empty["results"], "26 one-letter words are the empty query's listing");
    assert_eq!(v["vector"], empty["vector"]);
    assert_eq!(
        (c2 - c1, k2 - k1),
        (c1 - c0, k1 - k0),
        "and cost what the empty query costs, as to catalogue and cell rows"
    );
    // a long run of letters is the same
    let v = find(&w, &format!("{q}%20{q}")).await;
    assert_eq!(v["results"], empty["results"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h3_a_one_letter_word_changes_nothing() {
    let fx = Fx::pg("h051-article");
    catalogue(&fx);
    let w = fx.serve(config(&held(&["find"]), &[])).await;
    let plain = find(&w, "euro%20rate").await;
    assert!(!plain["results"].as_array().unwrap().is_empty(), "euro rate finds the entries: {plain}");
    for (what, q) in [
        ("a euro rate", "a%20euro%20rate"),
        ("euro a rate", "euro%20a%20rate"),
        ("euro rate a", "euro%20rate%20a"),
        ("a i o euro u rate", "a%20i%20o%20euro%20u%20rate"),
        ("nine letters, then the words (cut at 8 words today)", "a%20b%20c%20d%20e%20f%20g%20h%20i%20euro%20rate"),
    ] {
        let v = find(&w, q).await;
        assert_eq!(without_query(v), without_query(plain.clone()), "{what}");
    }
}
