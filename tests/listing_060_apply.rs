//! http 0.6.0, the listing release (CONTRACT-http-listing.md L3 and L8, with the changes of 25 Nov): what a run writes, on
//! the store side alone (`datumesh_http::listing::apply` over a `Fetch` the test makes, the clock injected as the run's
//! instant; no network). Failing on the test commit: `apply` is `todo!()`.
//!
//! * withdrawal on each of the three causes after a successful fetch: the host's mesh no longer lists the name, the
//!   descriptor does not verify, the key document is invalid (and, the test writer's reading, a mesh document that fails);
//! * one failed run changes nothing: every rank and `source(k)` cell as before, no withdrawal written, the listing keeps its
//!   last-seen day;
//! * drop-out after seven days unreachable, counted from the first failed run after the last success; not at six days
//!   23 hours;
//! * a run with nothing changed writes no `listings` row, no catalogue row or cell (and so no embedding: the store's
//!   `pending_embeddings` is fed by catalogue rows, see `listing_060_pg.rs`); the host's last-seen day is written only when
//!   it moves to the next day, or when the host returns after being unreachable.

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/listing060.rs"]
mod listing060;
#[path = "common/r07world.rs"]
mod r07world;

use catjob::*;
use catworld::*;
use common::lite::*;
use common::readpath::Served;
use datumesh_http::listing::{Cut, Fetch, Outcome};
use datumesh_store::Store;
use listing060::*;
use r07world::*;

fn listed(lite: &Lite) -> Vec<String> {
    lite.store.listings_of(SRC).unwrap().into_iter().map(|l| l.name).collect()
}

/// The listing mesh with the source's bank rate listed at RUN1 and the catalogue built at T3, served.
async fn listed_once(label: &str) -> Served {
    let lite = listing_lite(label);
    let a = apply(&lite, &fetched(vec![name_of(&bank_rate(), "a", vec![gb()])]), RUN1);
    assert_eq!(a.outcome, Outcome::Ok);
    assert_eq!(a.added, [bank_rate()]);
    assert_eq!(a.listed, 1);
    assert_eq!(a.writes, 2, "the listing and the host's row");
    assert!(a.changed_any());
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    assert_eq!(sources_at(&m, GB).await, [bank_rate(), relay()], "listed: rank 1 is the source's own function");
    m
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l3_a_first_run_lists_the_name_with_its_host_row() {
    let m = listed_once("l060-first").await;
    let s = &m.lite.store;
    let l = s.listing(&bank_rate()).unwrap().expect("listed");
    assert_eq!(l.host, SRC);
    assert_eq!(l.subjects, [gb()]);
    assert_eq!(l.key_id, src_key_id());
    assert_eq!((l.fetched_at.as_str(), l.listed_at.as_str()), (RUN1, RUN1));
    assert_eq!((l.label.as_str(), l.description.as_str()), (LABEL, DESCRIPTION));
    let h = s.listed_host(SRC).unwrap().expect("the host's row");
    assert_eq!(h.last_seen.as_deref(), Some(DAY1));
    assert_eq!(h.unreachable_since, None);
}

// ---- withdrawal on each of the three causes -----------------------------------------------------------------------

/// Withdrawal, cause 1: a fetch succeeds and the host's mesh no longer lists the name. It drops out at once, and the
/// catalogue job withdraws the `source(k)` cell it had written for it (rank past the count of sources); rank 1 returns to
/// the relay.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l8_withdrawal_when_the_mesh_no_longer_lists_the_name() {
    let m = listed_once("l060-w1").await;
    let a = apply(&m.lite, &fetched(vec![]), RUN2);
    assert_eq!(a.outcome, Outcome::Ok);
    assert_eq!(a.dropped, [bank_rate()]);
    assert_eq!(a.listed, 0);
    assert!(listed(&m.lite).is_empty());
    assert_eq!(m.lite.store.listing_count().unwrap(), 0);
    assert!(build(&m.lite, T4).cells > 0, "the job writes the withdrawal");
    assert_eq!(sources_at(&m, GB).await, [relay()]);
    assert_withdrawn(&source_at(&m, GB, 2).await, PAST, "source(2)");
}

/// Withdrawal, cause 2: a fetch succeeds and the descriptor does not verify. A fetched name whose descriptor bytes do not
/// hash to its function hash is not written and drops out; so does a name the fetch reached and refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l8_withdrawal_when_the_descriptor_does_not_verify() {
    let m = listed_once("l060-w2").await;
    let mut bad = name_of(&bank_rate(), "a", vec![gb()]);
    bad.descriptor = descriptor("tampered").0;
    let a = apply(&m.lite, &fetched(vec![bad]), RUN2);
    assert_eq!(a.dropped, [bank_rate()], "{a:?}");
    assert!(listed(&m.lite).is_empty());
    build(&m.lite, T4);
    assert_eq!(sources_at(&m, GB).await, [relay()]);
    assert_withdrawn(&source_at(&m, GB, 2).await, PAST, "source(2)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l8_withdrawal_when_the_fetch_refused_the_name() {
    let m = listed_once("l060-w2b").await;
    let f = Fetch::Fetched { key_id: src_key_id(), names: vec![], refused: vec![bank_rate()], cut: None };
    let a = apply(&m.lite, &f, RUN2);
    assert_eq!(a.dropped, [bank_rate()]);
    assert!(listed(&m.lite).is_empty());
}

/// Withdrawal, cause 3: the key document is invalid (an answer that fails the SPEC 9 checks, not a failure to reach it):
/// every listing of the host drops out at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l8_withdrawal_when_the_key_document_is_invalid() {
    let lite = listing_lite("l060-w3");
    let two = fetched(vec![name_of(&bank_rate(), "a", vec![gb()]), name_of(&format!("{SRC}/other"), "b", vec![gb()])]);
    apply(&lite, &two, RUN1);
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    assert_eq!(sources_at(&m, GB).await.len(), 3);
    let a = apply(&m.lite, &Fetch::KeyDocFailed { detail: "the document names another host".into() }, RUN2);
    assert_eq!(a.outcome, Outcome::KeyDocFailed);
    assert_eq!(a.dropped, [bank_rate(), format!("{SRC}/other")]);
    assert_eq!(a.listed, 0);
    assert!(listed(&m.lite).is_empty());
    let h = m.lite.store.listed_host(SRC).unwrap().unwrap();
    assert_eq!(h.last_seen.as_deref(), Some(DAY1), "a failed key document is not a sighting");
    build(&m.lite, T4);
    assert_eq!(sources_at(&m, GB).await, [relay()]);
    assert_withdrawn(&source_at(&m, GB, 2).await, PAST, "source(2)");
    assert_withdrawn(&source_at(&m, GB, 3).await, PAST, "source(3)");
}

/// The test writer's reading: a mesh document that answers and fails (it disagrees with the key document) is also an
/// answer that fails the SPEC 9 checks (9.6), so the host's listings drop out at once, as for cause 3.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l8_withdrawal_when_the_mesh_document_fails() {
    let m = listed_once("l060-w3b").await;
    let a = apply(&m.lite, &Fetch::MeshDocFailed { detail: "host disagrees".into() }, RUN2);
    assert_eq!(a.outcome, Outcome::MeshDocFailed);
    assert_eq!(a.dropped, [bank_rate()]);
    assert!(listed(&m.lite).is_empty());
}

// ---- an unreachable mesh is not a withdrawal ----------------------------------------------------------------------

/// [25 Nov] One failed run changes nothing: after a run where the host is unreachable, every rank and `source(k)` cell is
/// as before, and no withdrawal is written (the next catalogue run writes no cell at all); the listing carries its
/// last-seen day. The run writes the host's time first unreachable, once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l8_one_failed_run_changes_nothing() {
    let m = listed_once("l060-down").await;
    let before = (listed(&m.lite), m.lite.store.listing(&bank_rate()).unwrap(), catalogue_stamps(&m.lite));
    let a = apply(&m.lite, &unreachable(), RUN2);
    assert_eq!(a.outcome, Outcome::Unreachable);
    assert!(a.dropped.is_empty() && a.added.is_empty() && a.changed.is_empty(), "{a:?}");
    assert_eq!(a.listed, 1);
    assert_eq!(a.writes, 1, "the host's time first unreachable, and nothing else");
    assert_eq!((listed(&m.lite), m.lite.store.listing(&bank_rate()).unwrap(), catalogue_stamps(&m.lite)), before);
    let h = m.lite.store.listed_host(SRC).unwrap().unwrap();
    assert_eq!(h.last_seen.as_deref(), Some(DAY1), "the host keeps the day it was last seen");
    assert_eq!(h.unreachable_since.as_deref(), Some(RUN2));
    assert_eq!(build(&m.lite, T4).cells, 0, "no withdrawal, no cell: the catalogue is as before");
    assert_eq!(sources_at(&m, GB).await, [bank_rate(), relay()]);
    // a second failed run writes nothing: the time first unreachable is written once
    let again = apply(&m.lite, &unreachable(), RUN3);
    assert_eq!(again.writes, 0);
    assert_eq!(m.lite.store.listed_host(SRC).unwrap().unwrap().unreachable_since.as_deref(), Some(RUN2));
}

/// [25 Nov] Drop-out after seven days unreachable, counted from the first failed run after the last success: kept at six
/// days 23 hours, dropped at seven days; the catalogue then withdraws it and rank 1 returns to the relay.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l8_drop_out_after_seven_days_unreachable_and_not_before() {
    const FIRST_FAIL: &str = "2026-11-17T00:00:00.000000Z";
    const SIX_DAYS_23H: &str = "2026-11-23T23:00:00.000000Z";
    const SEVEN_DAYS_MINUS_1US: &str = "2026-11-23T23:59:59.999999Z";
    const SEVEN_DAYS: &str = "2026-11-24T00:00:00.000000Z";
    let m = listed_once("l060-seven").await;
    apply(&m.lite, &unreachable(), FIRST_FAIL);
    for at in ["2026-11-20T00:00:00.000000Z", SIX_DAYS_23H, SEVEN_DAYS_MINUS_1US] {
        let a = apply(&m.lite, &unreachable(), at);
        assert!(a.dropped.is_empty(), "kept at {at}: {a:?}");
        assert_eq!(listed(&m.lite), [bank_rate()], "at {at}");
    }
    assert_eq!(build(&m.lite, T4).cells, 0);
    let a = apply(&m.lite, &unreachable(), SEVEN_DAYS);
    assert_eq!(a.dropped, [bank_rate()], "dropped at seven days");
    assert!(listed(&m.lite).is_empty());
    build(&m.lite, T5);
    assert_eq!(sources_at(&m, GB).await, [relay()]);
    assert_withdrawn(&source_at(&m, GB, 2).await, PAST, "source(2)");
}

/// The seven days count from the first failed run after the LAST success: a success in between starts them again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l8_the_seven_days_count_from_the_first_failure_after_the_last_success() {
    let lite = listing_lite("l060-seven-reset");
    let f = fetched(vec![name_of(&bank_rate(), "a", vec![gb()])]);
    apply(&lite, &f, RUN1);
    apply(&lite, &unreachable(), "2026-11-17T00:00:00.000000Z");
    // back on day 5: the host returns (its row is written: last seen, no longer unreachable), nothing else
    let back = apply(&lite, &f, "2026-11-21T00:00:00.000000Z");
    assert_eq!(back.writes, 1, "the host's row only: {back:?}");
    let h = lite.store.listed_host(SRC).unwrap().unwrap();
    assert_eq!((h.last_seen.as_deref(), h.unreachable_since.as_deref()), (Some("2026-11-21"), None));
    apply(&lite, &unreachable(), "2026-11-22T00:00:00.000000Z");
    // eight days after the first failure, six after the second: kept
    let a = apply(&lite, &unreachable(), "2026-11-25T00:00:00.000000Z");
    assert!(a.dropped.is_empty(), "{a:?}");
    let a = apply(&lite, &unreachable(), "2026-11-29T00:00:00.000000Z");
    assert_eq!(a.dropped, [bank_rate()]);
}

// ---- a quiet run writes nothing --------------------------------------------------------------------------------------

/// [25 Nov] A run that finds nothing changed writes nothing: no `listings` row, no timestamp, no catalogue row (so no new
/// embedding), and the catalogue job after it writes no cell. Row counts before and after.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l3_a_run_with_nothing_changed_writes_nothing() {
    let m = listed_once("l060-quiet").await;
    let f = fetched(vec![name_of(&bank_rate(), "a", vec![gb()])]);
    let counts = |lite: &Lite| {
        (
            rows(lite, "SELECT count(*) FROM listings"),
            rows(lite, "SELECT count(*) FROM listed_hosts"),
            rows(lite, "SELECT count(*) FROM catalog_entries"),
            rows(lite, "SELECT count(*) FROM descriptors"),
            untouched(lite),
            catalogue_stamps(lite),
            lite.store.listing(&bank_rate()).unwrap(),
            lite.store.listed_host(SRC).unwrap(),
        )
    };
    let before = counts(&m.lite);
    let a = apply(&m.lite, &f, RUN1B);
    assert_eq!(a.outcome, Outcome::Ok);
    assert!(!a.changed_any(), "{a:?}");
    assert_eq!(a.writes, 0, "{a:?}");
    assert_eq!(a.listed, 1);
    assert_eq!(counts(&m.lite), before, "nothing written");
    assert_eq!(build(&m.lite, T4).cells, 0, "no catalogue rebuild has anything to write");
}

/// The host's last-seen day is written only when it moves from one day to the next: a run the next day writes the host's
/// row (one write) and no listing row.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l3_the_last_seen_day_is_written_once_a_day() {
    let m = listed_once("l060-day").await;
    let f = fetched(vec![name_of(&bank_rate(), "a", vec![gb()])]);
    let stamps = catalogue_stamps(&m.lite);
    let a = apply(&m.lite, &f, RUN2);
    assert_eq!(a.writes, 1, "{a:?}");
    assert!(!a.changed_any());
    assert_eq!(m.lite.store.listed_host(SRC).unwrap().unwrap().last_seen.as_deref(), Some(DAY2));
    let l = m.lite.store.listing(&bank_rate()).unwrap().unwrap();
    assert_eq!(l.fetched_at, RUN1, "the listing row is not rewritten");
    assert_eq!(catalogue_stamps(&m.lite), stamps);
    assert_eq!(apply(&m.lite, &f, RUN3).writes, 0, "the same day again: nothing");
}

/// A changed descriptor (another hash) is a change: the listing row is replaced (one write) and its first listed time kept.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l3_a_changed_descriptor_is_written() {
    let m = listed_once("l060-changed").await;
    let new = name_of(&bank_rate(), "b", vec![gb()]);
    let a = apply(&m.lite, &fetched(vec![new.clone()]), RUN1B);
    assert_eq!(a.changed, [bank_rate()]);
    assert_eq!(a.writes, 1);
    let l = m.lite.store.listing(&bank_rate()).unwrap().unwrap();
    assert_eq!(l.fn_hash, new.fn_hash);
    assert_eq!((l.fetched_at.as_str(), l.listed_at.as_str()), (RUN1B, RUN1));
    assert_eq!(build(&m.lite, T4).cells, 0, "the same rank: the catalogue's cells do not change");
}

/// A cut run keeps what it has verified and records the cut; the names past the cut were not checked and stay listed. A
/// name it reached and refused drops out.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l2_a_cut_run_keeps_the_names_past_the_cut() {
    let lite = listing_lite("l060-cut");
    let names: Vec<String> = ["a_rate", "b_rate", "c_rate"].iter().map(|n| format!("{SRC}/{n}")).collect();
    let all: Vec<_> = names.iter().map(|n| name_of(n, n, vec![gb()])).collect();
    apply(&lite, &fetched(all.clone()), RUN1);
    let cut = Fetch::Fetched { key_id: src_key_id(), names: vec![all[0].clone()], refused: vec![], cut: Some(Cut::Names) };
    let a = apply(&lite, &cut, RUN1B);
    assert_eq!(a.outcome, Outcome::Cut);
    assert_eq!(a.cut, Some(Cut::Names));
    assert!(a.dropped.is_empty(), "{a:?}");
    assert_eq!(listed(&lite), names);
    let cut = Fetch::Fetched {
        key_id: src_key_id(),
        names: vec![all[0].clone()],
        refused: vec![names[1].clone()],
        cut: Some(Cut::Time),
    };
    let a = apply(&lite, &cut, RUN1B);
    assert_eq!(a.cut, Some(Cut::Time));
    assert_eq!(a.dropped, [names[1].clone()]);
    assert_eq!(listed(&lite), [names[0].clone(), names[2].clone()]);
}

/// Only names under `<host>/` are taken: a fetched name under another host is not written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l2_only_names_under_the_host_are_listed() {
    let lite = listing_lite("l060-under");
    let f = fetched(vec![name_of(&bank_rate(), "a", vec![gb()]), name_of(&format!("{THIRD}/rates/bank_rate"), "t", vec![gb()])]);
    let a = apply(&lite, &f, RUN1);
    assert_eq!(a.added, [bank_rate()]);
    assert_eq!(lite.store.listing(&format!("{THIRD}/rates/bank_rate")).unwrap(), None);
    assert_eq!(lite.store.listing_count().unwrap(), 1);
}
