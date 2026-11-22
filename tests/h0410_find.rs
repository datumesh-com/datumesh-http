//! http 0.4.10, CONTRACT-round-0.7-P HP6: `/find` does bounded work, and the summary's counts come from the
//! counters. Written first; FAIL until then (the counters, and the bounded `text_find`, are store 0.5.3).
//!
//! Counted by the probe of `common/h0410.rs`: rows of `catalog_entries` (and of the cell tables) the mesh
//! visits. The catalogue has 20 entries, a table small enough that the planner reads it whole, so a call of
//! `text_find` visits 20 rows and the count of rows is 20 times the count of calls.
//!
//! * A 30-word query is cut to its first 8 words and spans of at most 4 (at most 26 spans, with the 8 words
//!   and the whole query: 35 calls at the most). 0.4.9 made about 150 calls for it. The test also compares it
//!   with the query of those first 8 words alone: the long query may not cost more than one pass over the
//!   table beyond it.
//! * The find thread's first build (the summary) reads no cell for its counts: the counts are counters.
//!   Coordinator decision: `refresh_value_counts` is not part of that build; it runs on its own schedule, first
//!   `DATUMESH_VALUE_COUNTS_SECS` after the start. The test sets that knob to an hour to keep it out of the
//!   window (0.4.9 ignores the variable).

mod common;
#[path = "common/h0410.rs"]
mod h0410;

use common::lite::*;
use common::pub031::function_line;
use h0410::*;
use serde_json::json;

const CATALOG: usize = 20;

fn sig() -> String {
    r#"{"args":{"site":{"type":"string","valid":false}},"order":["site"],"returns":{"type":"number","unit":"1"}}"#
        .to_string()
}

fn name(i: usize) -> String {
    format!("mesh-test.datumesh.com/h0410/fnd/f{i:02}")
}

fn cell(f: &str, site: &str) -> String {
    json!({"cell": {"fn": f, "args": {"site": {"type": "string", "value": site}}}, "licence": "CC0-1.0",
        "summary": "Synthetic test data.", "value": {"type": "number", "value": "1"}})
    .to_string()
}

/// Twenty functions, each catalogued with words that every one of them shares, and `cells` cells in each of the
/// first `with_cells`.
fn catalogue(fx: &Fx, with_cells: usize, cells: usize) {
    let mut lines = Vec::new();
    for i in 0..CATALOG {
        lines.push(function_line(&name(i), &sig(), None));
    }
    for i in 0..CATALOG {
        lines.push(common::readpath::catalog_line(
            &name(i),
            "function",
            &format!("The level of the site {i}"),
            "The rate for the site, a value for the euro reference level.",
            json!({"site": "the site"}),
            json!({}),
        ));
    }
    for i in 0..with_cells {
        for c in 0..cells {
            lines.push(cell(&name(i), &format!("s{c:05}")));
        }
    }
    fx.publish(&lines, NOW);
}

fn words(n: usize) -> String {
    let vocab = ["the", "rate", "site", "euro", "level", "for", "reference", "value"];
    (0..n).map(|i| vocab[i % vocab.len()]).collect::<Vec<_>>().join("%20")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_query_of_thirty_words_does_the_work_of_eight() {
    let fx = Fx::pg("hp6-words");
    catalogue(&fx, 0, 0);
    fx.probe().arm();
    // the find thread held: a query is answered without a first build
    let w = fx.serve(config(&held(&[]), &[])).await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let p = w.fx.probe();
    let b0 = p.reads(Group::Catalog);
    let r = w.get(&format!("/find?q={}", words(8))).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let b1 = p.reads(Group::Catalog);
    let r = w.get(&format!("/find?q={}", words(30))).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let b2 = p.reads(Group::Catalog);
    let (eight, thirty) = (b1 - b0, b2 - b1);
    assert!(eight > 0, "the 8-word query read no catalogue row: the probe is not counting");
    assert!(
        thirty <= eight + CATALOG as u64,
        "the 30-word query visited {thirty} catalogue rows, the 8-word one {eight}: only 8 words are used"
    );
    assert!(
        thirty <= (50 * CATALOG) as u64,
        "the 30-word query visited {thirty} rows ({} calls of 20 rows); at most 35 calls are made",
        thirty / CATALOG as u64
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_summary_counts_cells_from_the_counters_not_by_reading_them() {
    let fx = Fx::pg("hp6-summary");
    // two functions with 1,500 cells each: counting them by reading visits 3,000 rows
    catalogue(&fx, 2, 1_500);
    fx.probe().arm();
    // the find thread builds the summary; the others are held
    let mut vars = held(&["find"]);
    vars.push(("DATUMESH_VALUE_COUNTS_SECS", "3600"));
    let w = fx.serve(config(&vars, &[])).await;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let r = loop {
        let r = w.get("/find?summary=1").await;
        if r.status != 503 || std::time::Instant::now() > end {
            break r;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    assert_eq!(r.status, 200, "{}", r.text());
    let v = r.json();
    let f = v["functions"].as_array().unwrap();
    let cells = |n: &str| f.iter().find(|e| e["name"] == n).unwrap_or_else(|| panic!("no {n} in {v}"))["cells"].clone();
    assert_eq!(cells(&name(0)), 1500, "{v}");
    assert_eq!(cells(&name(1)), 1500, "{v}");
    let read = w.fx.probe().reads(Group::Cells);
    assert!(read < 400, "the first build visited {read} rows of cells; the counters make it a handful");
}
