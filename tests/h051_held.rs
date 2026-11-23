//! http 0.5.1, CONTRACT-0.5.1 H4: the held sentence. Written first; FAIL until then.
//!
//! * A computed entry (a formula function that reads a relay; not a relay itself) that declares `source_name` is named
//!   "computed from <name>": production's rainfall entry reads "monthly rainfall computed from CHIRPS". A relay still
//!   reads "from <name>" (the ECB, the Bank of England, the US Treasury in the same sentence).
//! * Within a domain, measures are ordered by their cells, descending (the sum, over the measure's rank-1 entries, of the
//!   entry's stored cells, `cell_counts`), then by name; at most three are shown. So scripture, whose text entry holds
//!   the cells, reads "texts, provenance".
//!
//! Production's names, with the earth domain as production has it: the rainfall entry is a formula over the raw relay
//! `relay/chirps.ucsb.edu/rainfall_raw` and carries `source_name` "CHIRPS" (the raw relay has no catalogue entry).
//! The fixture's scripture entries hold cells (verse 3, edition 1) so that the order by cells is the contract's.

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
use crworld::*;
use serde_json::{json, Value};

const TAIL: &str = "Ask find in plain words for what you need.";

/// The example of the contract, word for word.
const EXAMPLE: &str = "This mesh holds finance (interest rates from the ECB and the Bank of England, exchange rates from the ECB, yields from the US Treasury), scripture (texts, provenance), earth (monthly rainfall computed from CHIRPS) and registry (licences). Ask find in plain words for what you need.";

struct Ent {
    name: String,
    facets: Value,
    source_name: Option<&'static str>,
    /// Stored cells of an own-data entry (a relay has its one relayed cell).
    cells: usize,
    /// The formula of a computed entry.
    formula: Option<String>,
}

fn ent(name: &str, facets: Value, source_name: Option<&'static str>) -> Ent {
    Ent { name: host_name(name), facets, source_name, cells: 0, formula: None }
}

fn with_cells(mut e: Ent, n: usize) -> Ent {
    e.cells = n;
    e
}

fn f(d: &str, t: &str, m: &[&str], r: &str, q: &str) -> Value {
    fac(&[d], &[t], m, &[r], &[q])
}

const RAW: &str = "relay/chirps.ucsb.edu/rainfall_raw";

fn lines_of(entries: &[Ent]) -> Vec<String> {
    let mut v = Vec::new();
    for e in entries {
        match &e.formula {
            Some(src) => v.push(formula_line(&e.name, src, &date_sig())),
            None => v.push(fn_line(&e.name)),
        }
    }
    for e in entries.iter().filter(|e| e.name.contains("/relay/")) {
        v.push(relay_cell(&e.name));
    }
    for e in entries {
        for i in 0..e.cells {
            v.push(cell_line(&e.name, &[("date", "date", &format!("2026-10-{:02}", i + 1))], ("number", "1")));
        }
    }
    for e in entries {
        let mut extra = json!({"facets": e.facets});
        if let Some(s) = e.source_name {
            extra["source_name"] = json!(s);
        }
        v.push(catalog_line_of(&e.name, "function", extra));
    }
    v
}

/// The raw relay at `T_RELAY`, the entries (and the formulas that read the raw relay) at `NOW`, the job at `T3`.
async fn held_of_lite(label: &str, entries: &[Ent]) -> String {
    let lite = Lite::new(label);
    let raw = host_name(RAW);
    publish(&lite, "raw.jsonl", &[fn_line(&raw), relay_cell(&raw)], T_RELAY).unwrap_or_else(|e| panic!("{e}"));
    publish(&lite, "e.jsonl", &lines_of(entries), NOW).unwrap_or_else(|e| panic!("{e}"));
    build(&lite, T3);
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

fn production() -> Vec<Ent> {
    let mut rain =
        ent("earth/monthly_rainfall", f("earth", "region", &["rainfall"], "world", "monthly"), Some("CHIRPS"));
    rain.formula = Some(format!("{}(date: date)", host_name(RAW)));
    vec![
        ent(
            "relay/ecb.europa.eu/fx_reference",
            f("finance", "currency", &["exchange_rate"], "ea", "daily"),
            Some("the ECB"),
        ),
        ent(
            "relay/ecb.europa.eu/policy_rate",
            f("finance", "rate", &["interest_rate"], "ea", "on_change"),
            Some("the ECB"),
        ),
        ent(
            "relay/bankofengland.co.uk/bank_rate",
            f("finance", "rate", &["interest_rate"], "gb", "daily"),
            Some("the Bank of England"),
        ),
        ent("relay/treasury.gov/par_yield", f("finance", "yield", &["yield"], "us", "daily"), Some("the US Treasury")),
        with_cells(ent("scripture/verse", f("scripture", "verse", &["text"], "world", "static"), None), 3),
        with_cells(
            ent("scripture/edition", f("scripture", "edition", &["provenance", "text"], "world", "static"), None),
            1,
        ),
        rain,
        ent("register/dataset", f("registry", "dataset", &["licence"], "world", "on_change"), None),
    ]
}

/// The contract's example, word for word: "monthly rainfall computed from CHIRPS", "texts, provenance", and the relays
/// still "from".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h4_the_production_example_sentence() {
    let held = held_of_lite("h4-example", &production()).await;
    assert_eq!(held, EXAMPLE);
    assert!(held.split_whitespace().count() <= 60, "{held}");
}

/// A computed entry is "computed from <name>", a relay "from <name>", in one measure's sources: the order of the sources
/// is the subjects' (here one subject per entry, by key text), each name once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h4_a_relay_still_reads_from_and_a_computed_entry_reads_computed_from() {
    let mut calc = ent("earth/rain_calc", f("earth", "region", &["rainfall"], "world", "monthly"), Some("CHIRPS"));
    calc.formula = Some(format!("{}(date: date)", host_name(RAW)));
    let held = held_of_lite("h4-sources", &[calc]).await;
    assert_eq!(held, format!("This mesh holds earth (monthly rainfall computed from CHIRPS). {TAIL}"));
    // the same measure given by a relay with the same source name: "from"
    let relay = ent(
        "relay/chirps.ucsb.edu/rainfall_raw2",
        f("earth", "region", &["rainfall"], "world", "monthly"),
        Some("CHIRPS"),
    );
    let held = held_of_lite("h4-relay", &[relay]).await;
    assert_eq!(held, format!("This mesh holds earth (monthly rainfall from CHIRPS). {TAIL}"));
}

/// Measures within a domain are ordered by cells, descending, then by name, and three are shown: `rainfall` has 5 cells in
/// one entry, `licence` and `text` 3 each (the tie goes by name), `yield` 2 in two entries (the most subjects, which was
/// the order, and the fewest cells; it is the one left out).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h4_measures_are_ordered_by_their_cells() {
    let e = vec![
        with_cells(ent("w/yield_a", f("dd", "t1", &["yield"], "world", "static"), None), 1),
        with_cells(ent("w/yield_b", f("dd", "t2", &["yield"], "world", "static"), None), 1),
        with_cells(ent("w/rain", f("dd", "t3", &["rainfall"], "world", "static"), None), 5),
        with_cells(ent("w/lic", f("dd", "t4", &["licence"], "world", "static"), None), 3),
        with_cells(ent("w/txt", f("dd", "t5", &["text"], "world", "static"), None), 3),
    ];
    let held = held_of_lite("h4-cells", &e).await;
    assert_eq!(held, format!("This mesh holds dd (rainfall, licences, texts). {TAIL}"));
}

/// With no cells at all the order is by name: the measures tie.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h4_measures_with_no_cells_are_ordered_by_name() {
    let e = vec![
        ent("w/a1", f("dd", "t1", &["yield"], "world", "static"), None),
        ent("w/a2", f("dd", "t2", &["yield"], "world", "static"), None),
        ent("w/b1", f("dd", "t3", &["licence"], "world", "static"), None),
    ];
    let held = held_of_lite("h4-nocells", &e).await;
    assert_eq!(held, format!("This mesh holds dd (licences, yields). {TAIL}"));
}
