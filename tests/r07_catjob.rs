//! The 0.7 round (CONTRACT-round-0.7.md), R10 (the catalogue job under R1) and R7 (addresses). Failing tests.
//!
//! R10, with the withdrawal of R1 (a stored cell's version whose `withdrawn` member says why the cell has no value from its `asof`
//! on) in place of `false` and `""`:
//! * a subject that disappears has every method cell of it withdrawn with the reason "the subject is no longer in the catalogue",
//!   and its labels too (a request through a label answers `not_covered` with that `reason` and no receipt); a facet value likewise;
//! * `source(rank)` past the current count is withdrawn, "rank past the count of sources", when it was stored before;
//! * `exists()` is withdrawn, never `false`;
//! * a rerun with nothing changed writes nothing; a subject that comes back ends its withdrawal by a version with a value.
//!
//! A withdrawn cell read answers `not_covered` (404) with `receipt` (the withdrawal: a `withdrawn` member, no `value`), `trust`
//! (`primary`) and `reason` (its text); as of an instant before the run the history still answers `ok`.
//!
//! R7: the address rule (no facet value or rank in a cell address or a class name) is stated by the rule text; page paths may carry
//! a facet value (`/catalog/<domain>`).

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/r07world.rs"]
mod r07world;

use catjob::*;
use catworld::*;
use common::lite::*;
use common::pub031::{snapshot, write};
use common::readpath::*;
use crworld::*;
use r07world::*;
use serde_json::json;

const GB: (&str, &str, &str, &str) = ("finance", "rate", "interest_rate", "gb");

fn bank() -> String {
    host_name("relay/bankofengland.co.uk/bank_rate")
}

/// World V, the job run at `T3`, the bank rate's facets dropped at `T4`, the job run again at `T5`.
fn removed(label: &str) -> Lite {
    let lite = vocab(label);
    build(&lite, T3);
    let f = write(&lite, "drop.jsonl", &[catalog_line_of(&bank(), "function", json!({}))]);
    common::pub031::run(&lite, &f, T4).unwrap_or_else(|e| panic!("{e}"));
    let run = build(&lite, T5);
    assert!(cells(&run) > 0, "{run:?}");
    lite
}

fn labels_of_gb() -> [&'static str; 3] {
    ["interest rate · rate · united kingdom · finance", "interest rate united kingdom", "rate interest rate united kingdom"]
}

// ---- a removed subject -----------------------------------------------------------------------------------------------

/// Every method cell of the subject is withdrawn, with the reason; as of before the run each still has its value (the history
/// stays); the other subjects are untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r10_a_removed_subject_has_every_method_withdrawn() {
    let m = serve(removed("r10-subject"), &[]).await;
    let (d, t, me, r) = GB;
    for method in ["exists()", "label()", "sources()", "source(1)"] {
        let a = cell(&m, &sc(d, t, me, r, method)).await;
        assert_withdrawn(&a, GONE, method);
        let was = ask(&m, &path(&sc(d, t, me, r, method), Some(T4))).await;
        assert_ok(&was, &format!("{method} as of {T4}"));
    }
    let other = cell(&m, &sc("finance", "rate", "interest_rate", "ea", "exists()")).await;
    assert_eq!((other.outcome(), truth(&other)), ("ok", Some(true)), "{}", other.text);
}

/// `exists()` is withdrawn, never `false`: no subject of the world answers `ok` with `false`, and the removed one does not
/// answer `ok` at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r10_exists_is_withdrawn_never_false() {
    let m = serve(removed("r10-never-false"), &[]).await;
    for (d, t, me, r, _) in v_subjects() {
        let a = cell(&m, &sc(d, t, me, r, "exists()")).await;
        assert_ne!(truth(&a), Some(false), "{d} {t} {me} {r}: exists() is never false: {}", a.text);
    }
    let gone = cell(&m, &sc(GB.0, GB.1, GB.2, GB.3, "exists()")).await;
    assert_eq!(gone.outcome(), "not_covered", "{}", gone.text);
    for (f, v) in [("region", "gb")] {
        let a = cell(&m, &fc(f, v, "exists()")).await;
        assert_ne!(truth(&a), Some(false), "{f} {v}: {}", a.text);
    }
}

/// The labels of the removed subject: the lookup cells are withdrawn (their read is `not_covered` with the withdrawal), and a
/// request through a label answers `not_covered` with the withdrawal's `reason` and no receipt, since no cell of the request was
/// resolved; as of before the run the label still resolves.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r10_the_labels_of_a_removed_subject_are_withdrawn() {
    let m = serve(removed("r10-labels"), &[]).await;
    for label in labels_of_gb() {
        let lookup = format!("/cell/{}/from_label?label={}", subject_class(), common::world::qenc(label));
        let a = ask(&m, &lookup).await;
        assert_withdrawn(&a, GONE, &format!("the lookup cell of {label}"));
        let b = ask(&m, &path(&format!("{}[\"{label}\"].label()", subject_class()), None)).await;
        assert_eq!((b.status, b.outcome()), (404, "not_covered"), "{label}: {}", b.text);
        assert_eq!(b.v["reason"], GONE, "{label}: {}", b.text);
        assert!(b.v.get("receipt").is_none_or(|r| r.is_null()), "{label}: no cell was resolved, so no receipt: {}", b.text);
        let was = ask(&m, &path(&format!("{}[\"{label}\"].label()", subject_class()), Some(T4))).await;
        assert_ok(&was, &format!("{label} as of {T4}"));
    }
}

/// A facet value that is gone: the object `region gb` has its methods withdrawn, and the counts of the values that remain moved.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r10_a_removed_facet_value_has_its_methods_withdrawn() {
    let m = serve(removed("r10-facet"), &[]).await;
    for method in ["exists()", "label()", "subjects()"] {
        let a = cell(&m, &fc("region", "gb", method)).await;
        assert_withdrawn(&a, GONE, &format!("region gb {method}"));
    }
    let n = cell(&m, &fc("domain", "finance", "subjects()")).await;
    assert_eq!((n.outcome(), n.value()), ("ok", Some("4")), "{}", n.text);
    let was = ask(&m, &path(&fc("region", "gb", "exists()"), Some(T4))).await;
    assert_ok(&was, "region gb as of before the run");
}

// ---- source(rank) ----------------------------------------------------------------------------------------------------

/// Two sources of a subject, then one: `sources()` is 1, `source(1)` is the one left, and `source(2)`, stored before, is withdrawn
/// "rank past the count of sources" (not removed, not `""`); as of before the run it still names the second source.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r10_a_rank_past_the_count_is_withdrawn() {
    let lite = Lite::new("r10-rank");
    let (a, b) = (host_name("w/a"), host_name("w/b"));
    let f = fac(&["dd"], &["tt"], &["mm"], &["world"], &["static"]);
    crworld::publish(&lite, "e.jsonl", &entries_lines(&[(a.clone(), f.clone()), (b.clone(), f)]), NOW)
        .unwrap_or_else(|e| panic!("{e}"));
    build(&lite, T3);
    let drop = write(&lite, "drop.jsonl", &[catalog_line_of(&b, "function", json!({}))]);
    common::pub031::run(&lite, &drop, T4).unwrap_or_else(|e| panic!("{e}"));
    let run = build(&lite, T5);
    assert!(cells(&run) > 0, "{run:?}");
    let m = serve(lite, &[]).await;
    let s = ("dd", "tt", "mm", "world");
    let n = cell(&m, &sc(s.0, s.1, s.2, s.3, "sources()")).await;
    assert_eq!((n.outcome(), n.value()), ("ok", Some("1")), "{}", n.text);
    let one = cell(&m, &sc(s.0, s.1, s.2, s.3, "source(1)")).await;
    assert_eq!((one.outcome(), one.value()), ("ok", Some(a.as_str())), "{}", one.text);
    let two = cell(&m, &sc(s.0, s.1, s.2, s.3, "source(2)")).await;
    assert_withdrawn(&two, PAST, "source(2)");
    let was = ask(&m, &path(&sc(s.0, s.1, s.2, s.3, "source(2)"), Some(T4))).await;
    assert_eq!((was.outcome(), was.value()), ("ok", Some(b.as_str())), "as of before the run: {}", was.text);
    // a rank that was never stored has no cell at all: not a withdrawal
    let three = cell(&m, &sc(s.0, s.1, s.2, s.3, "source(3)")).await;
    assert_eq!(three.outcome(), "not_covered", "{}", three.text);
    assert!(three.v.get("receipt").is_none_or(|r| r.is_null()), "never stored, never withdrawn: {}", three.text);
}

// ---- reruns and returns ----------------------------------------------------------------------------------------------

/// A rerun with nothing changed writes nothing, after the withdrawals as before them: no cell, not a byte of the store, at the
/// same instant and at a later one.
#[test]
fn r10_a_rerun_after_the_withdrawals_writes_nothing() {
    let lite = removed("r10-rerun");
    let before = snapshot(&lite);
    for clock in [T5, T6, T7] {
        let again = build(&lite, clock);
        assert_eq!(cells(&again), 0, "a rerun at {clock}: {again:?}");
        assert_eq!(snapshot(&lite), before, "a rerun at {clock} wrote something");
    }
}

/// A subject that comes back ends its withdrawal: a later version with a value for each method and each label, and the facet
/// object too; then a rerun writes nothing again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r10_a_returning_subject_ends_its_withdrawal() {
    let lite = removed("r10-return");
    let f = write(
        &lite,
        "back.jsonl",
        &[cat_line(&bank(), fac(&["finance"], &["rate"], &["interest_rate"], &["gb"], &["daily"]))],
    );
    common::pub031::run(&lite, &f, T6).unwrap_or_else(|e| panic!("{e}"));
    let run = build(&lite, T7);
    assert!(cells(&run) > 0, "{run:?}");
    let after = snapshot(&lite);
    assert_eq!(cells(&build(&lite, T8)), 0);
    assert_eq!(snapshot(&lite), after);
    let m = serve(lite, &[]).await;
    let (d, t, me, r) = GB;
    let e = cell(&m, &sc(d, t, me, r, "exists()")).await;
    assert_eq!((e.outcome(), truth(&e)), ("ok", Some(true)), "{}", e.text);
    assert_eq!(source_of(&m, d, t, me, r, 1).await.as_deref(), Some(bank().as_str()));
    for label in labels_of_gb() {
        let b = ask(&m, &path(&format!("{}[\"{label}\"].label()", subject_class()), None)).await;
        assert_ok(&b, label);
    }
    let g = cell(&m, &fc("region", "gb", "exists()")).await;
    assert_eq!((g.outcome(), truth(&g)), ("ok", Some(true)), "{}", g.text);
    // and the withdrawal is still in the history, between the two values
    let during = ask(&m, &path(&sc(d, t, me, r, "exists()"), Some(T5))).await;
    assert_eq!(during.outcome(), "not_covered", "as of {T5}: {}", during.text);
}

// ---- R7: addresses ---------------------------------------------------------------------------------------------------

/// The rule text states the address rule, on the page and in the cell: no facet value or rank in a cell address or a class name,
/// and that a page path may carry a facet value.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r7_the_rule_text_states_the_address_rule() {
    let lite = vocab("r7-rule");
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    let r = until_built(&m, "/catalog/rule").await;
    assert_eq!(r.status, 200, "{}", r.text());
    let text = plain(&r.text()).to_lowercase();
    for w in ["facet value", "cell address", "class name", "page path"] {
        assert!(text.contains(w), "the rule page does not say {w:?}: {text}");
    }
}

/// A page path may carry a facet value: `/catalog/<domain>` stays, for every domain of the world, and the root links to each.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r7_a_page_path_carries_a_domain() {
    let lite = vocab("r7-pages");
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    let root = until_built(&m, "/catalog").await;
    assert_eq!(root.status, 200, "{}", root.text());
    let links: Vec<String> = hrefs(&root.text()).into_iter().map(|h| local(&h)).collect();
    for d in ["finance", "earth", "tools"] {
        assert!(links.contains(&format!("/catalog/{d}")), "{d}: {links:?}");
        assert_eq!(page(&m, &format!("/catalog/{d}")).await.status, 200, "{d}");
    }
}
