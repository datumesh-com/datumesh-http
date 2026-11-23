//! The 0.7 round (CONTRACT-round-0.7.md), R8: the held sentence. Failing tests.
//!
//! `find?summary=1` carries `held`, plain words made from the facet labels and the source names, never a function name, at most
//! 60 words, ending with the instruction to ask `find` in plain words.
//!
//! The form: "This mesh holds D1, D2 ... and Dn. Ask find in plain words for what you need." The domains are in the order of the
//! subjects, descending (then by name). Each Dk is the domain's label and, in parentheses, its measures (at most three, by their
//! number of subjects descending, then by name) as plain phrases: the measure's label from the table (plural where the table says),
//! with the frequency in front when it is `monthly`, `weekly` or `annual`, then "from" and the source names of the measure's
//! rank-1 entries, "A, B and C". A source is named only for a relayed entry; the publisher's own data names none. Over 60 words
//! the last domains' measures are dropped, then the last domains, until it fits.
//!
//! `source_name` on a catalogue entry: plain words, 1 to 48 bytes, string rules, outside every hash. Without it the source domain
//! (the `<domain>` of `<host>/relay/<domain>/...`) is used.
//!
//! The fake catalogues here use relayed names (`<host>/relay/<domain>/...`, with a relayed cell, as the feeds publish them), so a
//! relayed entry is one by its name and by its cells. datumesh-local's pass-through of `held` is not tested here (another crate).

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
use common::pub031::{function_line, write};
use common::readpath::*;
use crworld::*;
use serde_json::{json, Value};

const TAIL: &str = "Ask find in plain words for what you need.";

/// An entry: a name, its facets, and its `source_name` if it has one.
fn ent(name: &str, facets: Value, source: Option<&str>) -> (String, Value, Option<String>) {
    (host_name(name), facets, source.map(str::to_string))
}

fn lines_of(entries: &[(String, Value, Option<String>)]) -> Vec<String> {
    let mut v: Vec<String> = entries.iter().map(|(n, _, _)| fn_line(n)).collect();
    v.extend(entries.iter().filter(|(n, _, _)| n.contains("/relay/")).map(|(n, _, _)| relay_cell(n)));
    v.extend(entries.iter().map(|(n, f, s)| {
        let mut extra = json!({"facets": f});
        if let Some(s) = s {
            extra["source_name"] = json!(s);
        }
        catalog_line_of(n, "function", extra)
    }));
    v
}

async fn held_of_lite(label: &str, entries: &[(String, Value, Option<String>)]) -> String {
    let lite = Lite::new(label);
    crworld::publish(&lite, "e.jsonl", &lines_of(entries), NOW).unwrap_or_else(|e| panic!("{e}"));
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    held_of(&m).await
}

async fn held_of(m: &Served) -> String {
    // The first catalogue build is on the thread (R11): the summary is 200 before it, without `held`; wait for it.
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let r = until_built(m, "/find?summary=1").await;
        assert_eq!(r.status, 200, "{}", r.text());
        let v = r.json();
        if let Some(h) = v["held"].as_str() {
            return h.to_string();
        }
        assert!(std::time::Instant::now() < end, "no held in {v}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn f(d: &str, t: &str, m: &[&str], r: &str, q: &str) -> Value {
    fac(&[d], &[t], m, &[r], &[q])
}

/// Production's names, as the contract's example has them.
fn production() -> Vec<(String, Value, Option<String>)> {
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
        ent("scripture/verse", f("scripture", "verse", &["text"], "world", "static"), None),
        ent("scripture/edition", f("scripture", "edition", &["provenance", "text"], "world", "static"), None),
        ent(
            "relay/chirps.ucsb.edu/monthly_rainfall",
            f("earth", "region", &["rainfall"], "world", "monthly"),
            Some("CHIRPS"),
        ),
        ent("register/dataset", f("registry", "dataset", &["licence"], "world", "on_change"), None),
    ]
}

const EXAMPLE: &str = "This mesh holds finance (interest rates from the ECB and the Bank of England, exchange rates from the ECB, yields from the US Treasury), scripture (texts, provenance), earth (monthly rainfall from CHIRPS) and registry (licences). Ask find in plain words for what you need.";

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

// ---- the form --------------------------------------------------------------------------------------------------------

/// The contract's example, word for word, from a fake catalogue of production's names.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r8_the_example_sentence_from_production_names() {
    let held = held_of_lite("r8-example", &production()).await;
    assert_eq!(held, EXAMPLE);
    assert!(word_count(&held) <= 60, "{} words", word_count(&held));
}

/// It is plain words: no full name, no function name (no slash, no underscore, no host), and it ends with the instruction.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r8_no_full_name_and_the_ending_is_the_find_instruction() {
    // names with no source_name: the source domain is used, still no full name
    let mut e = production();
    e.push(ent("relay/sec.gov/filing", f("finance", "filing", &["fundamentals"], "us", "annual"), None));
    e.push(ent("ratio/pe_ratio", f("tools", "none", &["ratio"], "none", "static"), None));
    let held = held_of_lite("r8-names", &e).await;
    for bad in ["/", "_", HOST, "mesh-test", "relay", "fx_reference", "policy_rate", "par_yield", "pe_ratio"] {
        assert!(!held.contains(bad), "{bad:?} is in held: {held}");
    }
    assert!(held.starts_with("This mesh holds "), "{held}");
    assert!(held.ends_with(TAIL), "{held}");
    assert!(word_count(&held) <= 60, "{held}");
}

/// The source domain stands in when an entry has no `source_name`: `from sec.gov`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r8_without_a_source_name_the_source_domain_is_used() {
    let e = vec![ent("relay/sec.gov/filing", f("finance", "filing", &["fundamentals"], "us", "annual"), None)];
    let held = held_of_lite("r8-domain", &e).await;
    assert_eq!(
        held,
        format!("This mesh holds finance (annual company fundamentals from sec.gov). {TAIL}"),
        "an annual frequency goes in front, the table says `company fundamentals`, the source domain is the name"
    );
}

/// The publisher's own data names no source, even when the entry carries a `source_name`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r8_the_publishers_own_data_names_no_source() {
    let e = vec![ent(
        "register/dataset",
        f("registry", "dataset", &["licence"], "world", "on_change"),
        Some("the Registry"),
    )];
    let held = held_of_lite("r8-own", &e).await;
    assert_eq!(held, format!("This mesh holds registry (licences). {TAIL}"));
}

/// Several rank-1 sources of a measure: "A, B and C", each name once, in the order of the subjects.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r8_the_source_names_are_joined_a_b_and_c() {
    let e = vec![
        ent("relay/ecb.europa.eu/estr", f("finance", "rate", &["interest_rate"], "ea", "daily"), Some("the ECB")),
        ent(
            "relay/bankofengland.co.uk/bank_rate",
            f("finance", "rate", &["interest_rate"], "gb", "daily"),
            Some("the Bank of England"),
        ),
        ent(
            "relay/treasury.gov/fed_funds",
            f("finance", "rate", &["interest_rate"], "us", "daily"),
            Some("the US Treasury"),
        ),
        // a second source of the same subject is rank 2 and is not named
        ent(
            "relay/ecb.europa.eu/policy_rate",
            f("finance", "rate", &["interest_rate"], "ea", "daily"),
            Some("the ECB again"),
        ),
    ];
    let held = held_of_lite("r8-join", &e).await;
    assert_eq!(
        held,
        format!(
            "This mesh holds finance (interest rates from the ECB, the Bank of England and the US Treasury). {TAIL}"
        )
    );
}

/// The label table, and the frequency in front: every row of the table, a measure outside it (its name with `_` as a space), and
/// `monthly`, `weekly` and `annual` in front, `daily`, `on_change` and `static` not.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r8_the_label_table_and_the_frequencies() {
    let rows: Vec<(&str, &str)> = vec![
        ("exchange_rate", "exchange rates"),
        ("interest_rate", "interest rates"),
        ("yield", "yields"),
        ("fundamentals", "company fundamentals"),
        ("text", "texts"),
        ("provenance", "provenance"),
        ("rainfall", "rainfall"),
        ("licence", "licences"),
        ("ratio", "ratios"),
        ("date", "dates"),
        ("odd_measure", "odd measure"),
    ];
    let e: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(i, (m, _))| ent(&format!("w/f{i:02}"), f(&format!("d{i:02}"), "x", &[m], "world", "static"), None))
        .collect();
    let held = held_of_lite("r8-table", &e).await;
    let parts: Vec<String> = rows.iter().enumerate().map(|(i, (_, l))| format!("d{i:02} ({l})")).collect();
    let (last, init) = parts.split_last().unwrap();
    assert_eq!(held, format!("This mesh holds {} and {last}. {TAIL}", init.join(", ")));

    let freq = [
        ("d1", "rainfall", "monthly"),
        ("d2", "rainfall", "weekly"),
        ("d3", "yield", "annual"),
        ("d4", "rainfall", "daily"),
        ("d5", "rainfall", "on_change"),
        ("d6", "text", "static"),
    ];
    let e: Vec<_> = freq.iter().map(|(d, m, q)| ent(&format!("w/{d}"), f(d, "x", &[m], "world", q), None)).collect();
    let held = held_of_lite("r8-freq", &e).await;
    assert_eq!(
        held,
        format!("This mesh holds d1 (monthly rainfall), d2 (weekly rainfall), d3 (annual yields), d4 (rainfall), d5 (rainfall) and d6 (texts). {TAIL}")
    );
}

// ---- the bound -------------------------------------------------------------------------------------------------------

/// The text a domain contributes, between the commas and the "and": the domains of a sentence in order.
fn shown_domains(held: &str) -> Vec<(String, bool)> {
    let body = held.strip_prefix("This mesh holds ").unwrap().strip_suffix(&format!(". {TAIL}")).unwrap();
    // split on top-level separators: ", " and " and " outside parentheses
    let mut out = Vec::new();
    let (mut depth, mut cur) = (0i32, String::new());
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        let rest: String = chars[i..].iter().collect();
        if depth == 0 && rest.starts_with(", ") {
            out.push(std::mem::take(&mut cur));
            i += 2;
            continue;
        }
        if depth == 0 && rest.starts_with(" and ") && !rest[5..].contains(" and ") {
            out.push(std::mem::take(&mut cur));
            i += 5;
            continue;
        }
        cur.push(c);
        i += 1;
    }
    out.push(cur);
    out.into_iter()
        .map(|d| {
            let name = d.split(" (").next().unwrap().to_string();
            (name, d.contains('('))
        })
        .collect()
}

/// A catalogue of 14 domains, each with three measures and a relayed source with a name: it does not fit in 60 words, so the
/// last domains' measures are dropped (a domain without parentheses); the first domain keeps its measures, a domain with its
/// measures never follows one without, the domains shown are a prefix of the order by size, and the sentence is at most 60 words.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r8_over_sixty_words_the_last_domains_measures_are_dropped_first() {
    let mut e = Vec::new();
    for i in 0..14usize {
        let things: Vec<String> = (0..(8 - i / 2).max(1)).map(|k| format!("t{k:02}")).collect();
        let t: Vec<&str> = things.iter().map(String::as_str).collect();
        e.push((
            host_name(&format!("relay/s{i:02}.org/f{i:02}")),
            fac(&[&format!("d{i:02}")], &t, &["alpha", "beta", "gamma"], &["world"], &["daily"]),
            Some(format!("source {i:02}")),
        ));
    }
    let held = held_of_lite("r8-trim-measures", &e).await;
    assert!(word_count(&held) <= 60, "{} words: {held}", word_count(&held));
    assert!(
        held.starts_with("This mesh holds d00 (alpha from source 00"),
        "the largest domain keeps its measures: {held}"
    );
    assert!(held.ends_with(TAIL), "{held}");
    let shown = shown_domains(&held);
    let names: Vec<&str> = shown.iter().map(|(n, _)| n.as_str()).collect();
    let want: Vec<String> = (0..names.len()).map(|i| format!("d{i:02}")).collect();
    assert_eq!(names, want.iter().map(String::as_str).collect::<Vec<_>>(), "a prefix of the order by size: {held}");
    let mut bare = false;
    for (n, has) in &shown {
        if bare {
            assert!(!has, "{n} has measures after a domain that has none: {held}");
        }
        if !has {
            bare = true;
        }
    }
    assert!(bare, "the last domains' measures were dropped: {held}");
    assert!(shown.len() == 14, "bare domains fit, so no domain is left out yet: {held}");
}

/// 70 domains of one subject each: even bare they do not fit, so the last domains are dropped: a prefix of the order (by name,
/// the sizes being equal), no parentheses, at most 60 words, the instruction still at the end, `and` before the last one shown.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r8_then_the_last_domains_are_dropped() {
    let e: Vec<_> = (0..70usize)
        .map(|i| ent(&format!("w/f{i:02}"), f(&format!("d{i:02}"), "x", &["ratio"], "world", "static"), None))
        .collect();
    let held = held_of_lite("r8-trim-domains", &e).await;
    assert!(word_count(&held) <= 60, "{} words: {held}", word_count(&held));
    assert!(held.ends_with(TAIL), "{held}");
    let shown = shown_domains(&held);
    assert!(shown.len() >= 2 && shown.len() < 70, "{held}");
    let names: Vec<&str> = shown.iter().map(|(n, _)| n.as_str()).collect();
    let want: Vec<String> = (0..names.len()).map(|i| format!("d{i:02}")).collect();
    assert_eq!(names, want.iter().map(String::as_str).collect::<Vec<_>>(), "a prefix of the order: {held}");
    assert!(shown.iter().all(|(_, has)| !has), "no measures are left when domains are dropped: {held}");
    let last = names.last().unwrap();
    assert!(held.contains(&format!(" and {last}. ")), "{held}");
    // as long as it can be: "This mesh holds" (3) + n domains + "and" + the instruction (9) is n + 13, so 47 domains are 60 words
    assert_eq!((word_count(&held), shown.len()), (60, 47), "the longest sentence that fits: {held}");
}

// ---- source_name on the entry ----------------------------------------------------------------------------------------

fn name_line_set(source: Value) -> Vec<String> {
    let n = host_name("relay/ecb.europa.eu/rate");
    vec![fn_line(&n), relay_cell(&n), catalog_line_of(&n, "function", json!({"source_name": source}))]
}

fn publish_lines(label: &str, lines: &[String]) -> Result<(), String> {
    let lite = Lite::new(label);
    let f = write(&lite, "s.jsonl", lines);
    common::pub031::run(&lite, &f, NOW).map(|_| ()).map_err(|e| format!("line {}: {e}", e.line))
}

/// `source_name` is plain words of 1 to 48 bytes: 1 and 48 bytes are accepted; 0 and 49 bytes, a number, a control character
/// are refused at the catalogue line (line 3).
#[test]
fn r8_source_name_is_one_to_forty_eight_bytes_of_plain_words() {
    for ok in ["x", "the Bank of England", &"a".repeat(48)] {
        publish_lines("r8-sn-ok", &name_line_set(json!(ok))).unwrap_or_else(|e| panic!("{ok:?}: {e}"));
    }
    // 48 bytes that are not 48 characters
    publish_lines("r8-sn-bytes", &name_line_set(json!("é".repeat(24)))).unwrap_or_else(|e| panic!("{e}"));
    for (what, v) in [
        ("empty", json!("")),
        ("49 bytes", json!("a".repeat(49))),
        ("49 bytes of two-byte characters and one more", json!(format!("{}a", "é".repeat(24)))),
        ("a number", json!(5)),
        ("a control character", json!("the\u{7}ECB")),
        ("a list", json!(["the ECB"])),
    ] {
        let e = publish_lines("r8-sn-bad", &name_line_set(v)).expect_err(what);
        assert!(e.starts_with("line 3:"), "{what}: refused at the catalogue line: {e}");
    }
}

/// `source_name` is outside every hash, as `facets` is: the same function with and without it has the same hash, and a second
/// publication of the same entry changes nothing.
#[test]
fn r8_source_name_is_outside_every_hash() {
    let n = host_name("relay/ecb.europa.eu/rate");
    let with = Lite::new("r8-hash-with");
    let without = Lite::new("r8-hash-without");
    crworld::publish(&with, "a.jsonl", &name_line_set(json!("the ECB")), NOW).unwrap_or_else(|e| panic!("{e}"));
    crworld::publish(&without, "a.jsonl", &[fn_line(&n), relay_cell(&n)], NOW).unwrap();
    assert_eq!(fn_hash(&with, &n), fn_hash(&without, &n));
    let before = common::pub031::snapshot(&with);
    crworld::publish(&with, "a.jsonl", &name_line_set(json!("the ECB")), NOW).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(common::pub031::snapshot(&with), before, "a republication of the same entry writes nothing");
    // a changed source name is a new entry version but the same function
    crworld::publish(&with, "b.jsonl", &name_line_set(json!("the European Central Bank")), T2)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(fn_hash(&with, &n), fn_hash(&without, &n));
}

/// The sentence follows a changed `source_name` after the next job and build.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r8_the_sentence_follows_the_source_name() {
    let mk = |s: &str| {
        vec![ent("relay/ecb.europa.eu/rate", f("finance", "rate", &["interest_rate"], "ea", "daily"), Some(s))]
    };
    let a = held_of_lite("r8-follow-a", &mk("the ECB")).await;
    let b = held_of_lite("r8-follow-b", &mk("the European Central Bank")).await;
    assert_eq!(a, format!("This mesh holds finance (interest rates from the ECB). {TAIL}"));
    assert_eq!(b, format!("This mesh holds finance (interest rates from the European Central Bank). {TAIL}"));
}

#[allow(dead_code)]
fn unused() -> String {
    function_line("a", "b", None)
}
