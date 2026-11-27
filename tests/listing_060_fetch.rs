//! http 0.6.0, the listing release (CONTRACT-http-listing.md L2, L3, L9): the fetch job against a second mesh in process,
//! reached through `DATUMESH_PEERS`-style origins (a recording proxy in front of it, which can replace, rewrite, delay or
//! fail its answers). Failing on the test commit: `fetch` and `run_host` are `todo!()`.
//!
//! * key document checks: valid; wrong host; expired key (no key valid at the run's time, the test writer's reading of
//!   "validity"); not found; and a mesh document disagreeing on `host` or `proto`;
//! * a descriptor whose bytes do not hash to the function hash: the name is refused (and drops out when it was listed);
//! * limits: names (the first N by name), bytes, time;
//! * the other mesh's catalogue cells: a forged one is refused, and a valid one leaves no row in `cells`, `receipts` or
//!   `versions`; only names under `<host>/` are taken (the source's catalogue also lists a third publisher's name);
//! * never a cell, slice or receipt of a listed function: no request for one.

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/listing060.rs"]
mod listing060;

use std::time::{Duration, Instant};

use common::lite::*;
use common::*;
use datumesh_core::json::{self, Value};
use datumesh_http::listing::{Cut, Fetch, Limits, Outcome};
use datumesh_store::Store;
use listing060::*;

/// The requests for anything of a listed function but its description (`/describe/<name>`, `/fn/<name>?format=json`):
/// cells, slices, series, versions, explanations of it.
fn reads_of_the_name(log: &[String], name: &str) -> Vec<String> {
    log.iter()
        .filter(|p| p.contains(name.split('/').nth(1).unwrap()))
        .filter(|p| !p.starts_with("/describe/") && !p.starts_with("/fn/"))
        .cloned()
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l2_a_valid_host_is_fetched_from_its_own_catalogue() {
    let src = source("l060-f-valid", &[]).await;
    let f = fetch_at(&src.proxy.base, Limits::default(), RUN1).await;
    let Fetch::Fetched { key_id, names, refused, cut } = &f else { panic!("not fetched: {f:?}") };
    assert_eq!(key_id, &src_key_id());
    assert_eq!(cut, &None);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(names.len(), 1, "only names under source.example/: the third publisher's name is not followed: {f:?}");
    let n = &names[0];
    assert_eq!(n.name, bank_rate());
    assert_eq!(n.fn_hash, src_hash(&src, &bank_rate()));
    assert_eq!(datumesh_core::function_hash(&n.descriptor).unwrap(), n.fn_hash, "the descriptor's bytes hash to it");
    assert_eq!(n.subjects, [gb()]);
    assert_eq!((n.label.as_str(), n.description.as_str()), (LABEL, DESCRIPTION));
    let log = src.proxy.log();
    assert!(log.iter().any(|p| p == "/.well-known/datumesh.json"), "the key document: {log:?}");
    assert!(log.iter().any(|p| p == "/.well-known/datumesh-mesh.json"), "the mesh document: {log:?}");
    assert!(log.iter().any(|p| p.starts_with("/describe/") && p.contains("bank_rate")), "describe: {log:?}");
    assert!(log.iter().any(|p| *p == format!("/descriptor/{}", n.fn_hash)), "the descriptor by hash: {log:?}");
    assert!(reads_of_the_name(&log, &bank_rate()).is_empty(), "no cell, slice or receipt of the listed function: {log:?}");
    assert!(!log.iter().any(|p| p.contains(THIRD)), "a name listed from elsewhere is not followed: {log:?}");
}

/// [25 Nov] The other mesh's catalogue cells are read and verified, and not stored: after a run, no row in `cells`,
/// `receipts` or `versions`, no binding and no namespace of the host; the listing and its descriptor are all that is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l3_a_run_stores_no_cell_receipt_version_or_binding_of_the_other_mesh() {
    let src = source("l060-f-norows", &[]).await;
    let lite = listing_lite("l060-f-norows");
    let before = untouched(&lite);
    let a = run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await;
    assert_eq!(a.outcome, Outcome::Ok);
    assert_eq!(a.added, [bank_rate()]);
    assert_eq!(untouched(&lite), before, "cells, receipts, versions, name_bindings, class_bindings, namespaces");
    assert_eq!(hosted_rows(&lite, SRC), 0);
    let l = lite.store.listing(&bank_rate()).unwrap().expect("listed");
    assert_eq!(lite.store.descriptor(&l.fn_hash).as_deref(), Some(l.descriptor.as_slice()), "the descriptor is served by hash");
    assert_eq!(lite.store.resolve_name(&bank_rate(), END).unwrap(), None, "a listing is not a binding");
}

// ---- key document checks ---------------------------------------------------------------------------------------------

async fn key_doc_case(label: &str, setup: impl FnOnce(&Source)) -> (Fetch, Lite) {
    let src = source(label, &[]).await;
    setup(&src);
    let f = fetch_at(&src.proxy.base, Limits::default(), RUN1).await;
    let lite = listing_lite(label);
    let a = run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await;
    assert_eq!(a.listed, 0, "{label}: nothing listed this run");
    assert_eq!(lite.store.listing_count().unwrap(), 0);
    (f, lite)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l2_a_key_document_of_another_host_fails() {
    let (f, _) = key_doc_case("l060-kd-host", |s| {
        let other = "other.example";
        s.proxy.override_path("/.well-known/datumesh.json", 200, key_doc(other, &key_for(other)));
    })
    .await;
    assert!(matches!(f, Fetch::KeyDocFailed { .. }), "{f:?}");
}

/// Validity: a key document whose only key ended before the run's time (it expired) does not verify the host now.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l2_a_key_document_whose_key_expired_fails() {
    let (f, _) = key_doc_case("l060-kd-expired", |s| {
        let doc = json::parse(&key_doc(SRC, &key_for(SRC))).unwrap();
        let Value::Object(mut top) = doc else { panic!() };
        for (k, v) in top.iter_mut() {
            if k == "keys" {
                let Value::Array(keys) = v else { panic!() };
                for key in keys.iter_mut() {
                    let Value::Object(m) = key else { panic!() };
                    m.push(("valid_to".into(), Value::String("2026-11-15T13:00:00.000000Z".into())));
                }
            }
        }
        let expired = Value::Object(top).to_jcs();
        datumesh_core::key_document_from_response(SRC, 200, Some(&expired)).expect("a well-formed document, expired");
        s.proxy.override_path("/.well-known/datumesh.json", 200, expired);
    })
    .await;
    assert!(matches!(f, Fetch::KeyDocFailed { .. }), "{f:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l2_a_key_document_not_found_fails() {
    let (f, _) = key_doc_case("l060-kd-404", |s| {
        s.proxy.override_path("/.well-known/datumesh.json", 404, br#"{"error":"not found"}"#.to_vec());
    })
    .await;
    assert!(matches!(f, Fetch::KeyDocFailed { .. }), "a 4xx is an answer that fails, not an unreachable host: {f:?}");
}

async fn mesh_doc_with(src: &Source, member: &str, value: &str) {
    let (status, body) = src.mesh.raw("/.well-known/datumesh-mesh.json").await;
    assert_eq!(status, 200);
    let Value::Object(mut m) = json::parse(&body).unwrap() else { panic!() };
    for (k, v) in m.iter_mut() {
        if k == member {
            *v = Value::String(value.into());
        }
    }
    src.proxy.override_path("/.well-known/datumesh-mesh.json", 200, Value::Object(m).to_jcs());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l2_a_mesh_document_disagreeing_on_host_fails() {
    let src = source("l060-md-host", &[]).await;
    mesh_doc_with(&src, "host", "other.example").await;
    let f = fetch_at(&src.proxy.base, Limits::default(), RUN1).await;
    assert!(matches!(f, Fetch::MeshDocFailed { .. }), "{f:?}");
    let lite = listing_lite("l060-md-host");
    let a = run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await;
    assert_eq!(a.outcome, Outcome::MeshDocFailed);
    assert_eq!(a.listed, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l2_a_mesh_document_disagreeing_on_proto_fails() {
    let src = source("l060-md-proto", &[]).await;
    mesh_doc_with(&src, "proto", "datumesh/2").await;
    let f = fetch_at(&src.proxy.base, Limits::default(), RUN1).await;
    assert!(matches!(f, Fetch::MeshDocFailed { .. }), "{f:?}");
}

// ---- unreachable ----------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l8_a_connection_error_a_5xx_or_a_timeout_is_unreachable() {
    let f = fetch_at(&dead_origin(), Limits::default(), RUN1).await;
    assert!(matches!(f, Fetch::Unreachable { .. }), "connection refused: {f:?}");
    let src = source("l060-unreach", &[]).await;
    src.proxy.fail_all(Some(503));
    let f = fetch_at(&src.proxy.base, Limits::default(), RUN1).await;
    assert!(matches!(f, Fetch::Unreachable { .. }), "503: {f:?}");
    src.proxy.fail_all(None);
    src.proxy.delay("/.well-known/", 4_000);
    let t = Instant::now();
    let f = fetch_at(&src.proxy.base, Limits { time: Duration::from_millis(800), ..Limits::default() }, RUN1).await;
    assert!(matches!(f, Fetch::Unreachable { .. }), "a key document that does not answer in time: {f:?}");
    assert!(t.elapsed() < Duration::from_millis(2_500), "the run stops at its time limit: {:?}", t.elapsed());
}

// ---- the descriptor -----------------------------------------------------------------------------------------------------

/// A descriptor whose bytes do not hash to the function hash: the name is refused; listed before, it drops out.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l2_a_descriptor_hash_mismatch_refuses_the_name() {
    let src = source("l060-desc", &[]).await;
    let lite = listing_lite("l060-desc");
    assert_eq!(run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await.added, [bank_rate()]);
    let h = src_hash(&src, &bank_rate());
    src.proxy.override_path(&format!("/descriptor/{h}"), 200, descriptor("not the one").0);
    let f = fetch_at(&src.proxy.base, Limits::default(), RUN2).await;
    let Fetch::Fetched { names, refused, .. } = &f else { panic!("{f:?}") };
    assert!(names.is_empty(), "{f:?}");
    assert_eq!(refused, &[bank_rate()]);
    let a = run_at(&lite, &src.proxy.base, Limits::default(), RUN2).await;
    assert_eq!(a.dropped, [bank_rate()]);
    assert_eq!(lite.store.listing_count().unwrap(), 0);
}

// ---- the other mesh's catalogue cells ------------------------------------------------------------------------------------

/// [25 Nov] A forged catalogue cell is refused. The source publishes `forged_rate` in a subject of its own (region `zzq`);
/// the proxy rewrites `zzq` to `zzx` in every answer but the documents, describe and descriptors, so each catalogue cell of
/// that subject no longer verifies against the key document (an implementation that did not verify would list
/// `forged_rate` under region `zzx`). The valid cells still list the bank rate.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l2_a_forged_catalogue_cell_is_refused() {
    let zzq = datumesh_store::SubjectKeys::new("finance", "rate", "interest_rate", "zzq");
    let more = src_fn_lines(&format!("{SRC}/forged_rate"), "Forged rate", "A rate in a subject of its own.", subject_facets(&zzq));
    let src = source("l060-forged", &more).await;
    // control: unforged, both are listed
    let f = fetch_at(&src.proxy.base, Limits::default(), RUN1).await;
    assert_eq!(fetched_names(&f), [bank_rate(), format!("{SRC}/forged_rate")]);
    src.proxy.rewrite("zzq", "zzx");
    let lite = listing_lite("l060-forged");
    let before = untouched(&lite);
    let a = run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await;
    assert_eq!(a.added, [bank_rate()], "the forged subject's name is not listed: {a:?}");
    assert_eq!(lite.store.listing(&format!("{SRC}/forged_rate")).unwrap(), None);
    for l in lite.store.listings_of(SRC).unwrap() {
        assert!(l.subjects.iter().all(|s| s.region != "zzx"), "no subject from a forged cell: {l:?}");
    }
    assert_eq!(untouched(&lite), before, "neither the forged nor the valid cells are stored");
}

// ---- limits ----------------------------------------------------------------------------------------------------------------

fn five() -> Vec<String> {
    ["e_rate", "a_rate", "d_rate", "b_rate", "c_rate"]
        .iter()
        .flat_map(|n| src_fn_lines(&format!("{SRC}/{n}"), "Rate", "A rate of the source.", gb_facets()))
        .collect()
}

fn all_six() -> Vec<String> {
    let mut v: Vec<String> = ["a_rate", "b_rate", "bank_rate", "c_rate", "d_rate", "e_rate"].iter().map(|n| format!("{SRC}/{n}")).collect();
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l2_the_names_limit_keeps_the_first_names_and_records_the_cut() {
    let src = source("l060-lim-names", &five()).await;
    let f = fetch_at(&src.proxy.base, Limits::default(), RUN1).await;
    assert_eq!(fetched_names(&f), all_six(), "control: all six");
    let f = fetch_at(&src.proxy.base, Limits { names: 3, ..Limits::default() }, RUN1).await;
    let Fetch::Fetched { names, cut, .. } = &f else { panic!("{f:?}") };
    assert_eq!(cut, &Some(Cut::Names));
    let got: Vec<String> = names.iter().map(|n| n.name.clone()).collect();
    assert_eq!(got, all_six()[..3].to_vec(), "the first three by name");
    let lite = listing_lite("l060-lim-names");
    let a = run_at(&lite, &src.proxy.base, Limits { names: 3, ..Limits::default() }, RUN1).await;
    assert_eq!((a.outcome, a.cut, a.listed), (Outcome::Cut, Some(Cut::Names), 3));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l2_the_bytes_limit_cuts_the_run() {
    let src = source("l060-lim-bytes", &five()).await;
    src.proxy.clear_log();
    let f = fetch_at(&src.proxy.base, Limits::default(), RUN1).await;
    assert_eq!(fetched_names(&f).len(), 6);
    let full = src.proxy.bytes() as u64;
    let f = fetch_at(&src.proxy.base, Limits { bytes: full / 2, ..Limits::default() }, RUN1).await;
    let Fetch::Fetched { names, cut, .. } = &f else { panic!("half the bytes of a full run: {f:?}") };
    assert_eq!(cut, &Some(Cut::Bytes));
    assert!(names.len() < 6, "{f:?}");
    for n in names {
        assert_eq!(datumesh_core::function_hash(&n.descriptor).unwrap(), n.fn_hash, "what is kept is verified");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l2_the_time_limit_cuts_the_run() {
    let src = source("l060-lim-time", &five()).await;
    src.proxy.delay("/describe/", 400);
    let limit = Duration::from_millis(1_500);
    let t = Instant::now();
    let f = fetch_at(&src.proxy.base, Limits { time: limit, ..Limits::default() }, RUN1).await;
    let took = t.elapsed();
    let Fetch::Fetched { names, cut, .. } = &f else { panic!("{f:?}") };
    assert_eq!(cut, &Some(Cut::Time));
    assert!(names.len() < 6, "{f:?}");
    assert!(took < limit + Duration::from_millis(1_000), "the run stops at its limit: {took:?}");
}

#[test]
fn l2_the_default_limits() {
    let l = Limits::default();
    assert_eq!((l.names, l.bytes, l.time), (2_000, 32 << 20, Duration::from_secs(120)));
    assert_eq!(datumesh_http::listing::UNREACHABLE_DROP_AFTER, Duration::from_secs(7 * 86_400));
}
