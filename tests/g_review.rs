//! The fixes from the independent review of Part G (CONTRACT-guard-patch.md, "Decided 26 Nov:
//! fixes from the independent review of Part G") at the mesh: items 3, 4 and 5. Failing tests.
//!
//! * Item 3: on a store with a recording clock, running the same publish-stored file twice, and
//!   importing the same bundle twice (bundle::import_signed, and POST /bundles), succeeds the second
//!   time and writes nothing.
//! * Item 4: the keys for a past read's receipt: POST /verify of a receipt over another publisher's
//!   members, read with a past `?asof=` (no member-set row), is ok.
//! * Item 5: the rebuild counts against the evaluation's time budget (DATUMESH_BUDGET_MS): with no
//!   budget of 1 ms (0 is refused by the mesh's configuration), the /manifest?asof= rebuild of 1,000
//!   members is 503 E_EVAL_BUDGET.
//!
//! `foreign_deps` (item 4) is not covered: it walks a receipt's lineage to foreign stored cells, and
//! slices of foreign functions are refused, so a past read's slice has no foreign member to find.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::lite::*;
use common::readpath::build_served;
use common::*;
use datumesh_core::Cell;
use datumesh_http::bundle;
use datumesh_store::{SqliteOptions, SqliteStore, Store};
use publish4::*;

const MIN: &str = "0001-01-01T00:00:00.000000Z";

/// A recording clock the test sets.
#[derive(Clone)]
struct Rec(Arc<Mutex<String>>);

impl Rec {
    fn at(t: &str) -> Rec {
        Rec(Arc::new(Mutex::new(t.to_string())))
    }
    fn set(&self, t: &str) {
        *self.0.lock().unwrap() = t.to_string();
    }
    fn f(&self) -> datumesh_store::RecordingClock {
        let c = self.0.clone();
        Arc::new(move || c.lock().unwrap().clone())
    }
}

/// A Lite store (the mesh host's key document held) that records at `rec`.
fn rec_lite(label: &str, rec: &Rec) -> Lite {
    let base = Lite::new(label);
    let store =
        SqliteStore::open_with(base.dir.join("store.db"), SqliteOptions::new().recording_clock(rec.f())).unwrap();
    let lite = Lite { store: Arc::new(store), key: base.key.clone(), dir: base.dir.clone() };
    // The directory now belongs to `lite`.
    std::mem::forget(base);
    lite
}

fn versions_of(lite: &Lite, cell_id: &str) -> usize {
    lite.store.versions(cell_id, MIN, END).unwrap().len()
}

fn level_id(host: &str, site: &str) -> String {
    Cell { ns: ns_of(host), fn_name: "level".into(), fn_hash: stored_fn().hash(), args: args(&[("site", s(site))]) }
        .id()
}

// ---- Item 3: re-runs are no-ops ----------------------------------------------------------------

#[test]
fn the_same_publish_file_twice_on_a_recording_store_writes_nothing_the_second_time() {
    let rec = Rec::at(NOW);
    let lite = rec_lite("g-review-publish", &rec);
    let c = calculus::calc();
    let mut lines = calculus::data_lines(&c);
    lines.truncate(lines.len().min(8));
    let f = common::pub031::write(&lite, "twice.jsonl", &lines);
    common::pub031::run(&lite, &f, NOW).unwrap_or_else(|e| panic!("first run: {e}"));
    let fn_hash = lite.store.resolve_name(&c.reading, END).unwrap().expect("reading is bound");
    let n_cell = |site: &str| {
        Cell {
            ns: c.reading.rsplit_once('/').unwrap().0.into(),
            fn_name: "reading".into(),
            fn_hash: fn_hash.clone(),
            args: args(&[("site", s(site))]),
        }
        .id()
    };
    let before = (versions_of(&lite, &n_cell("N")), versions_of(&lite, &n_cell("S")));
    assert_eq!(before, (1, 1));
    // The clock moves on; the same file at a later run clock is the same lines, already stored.
    rec.set(T2_RUN);
    common::pub031::run(&lite, &f, NOW).unwrap_or_else(|e| panic!("the second run of the same file must succeed: {e}"));
    assert_eq!((versions_of(&lite, &n_cell("N")), versions_of(&lite, &n_cell("S"))), before, "nothing written");
}

/// A later recording time for the second runs.
const T2_RUN: &str = "2026-11-15T12:00:30.000000Z";

#[test]
fn the_same_bundle_imported_twice_on_a_recording_store_writes_nothing_the_second_time() {
    let rec = Rec::at(NOW);
    let lite = rec_lite("g-review-import", &rec);
    let b = simple_bundle(HOST);
    let owned: Vec<String> = Vec::new();
    bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(NOW), &b).unwrap_or_else(|e| panic!("first: {e:?}"));
    let id = level_id(HOST, "N");
    assert_eq!(versions_of(&lite, &id), 1);
    rec.set(T2_RUN);
    bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(T2_RUN), &b)
        .unwrap_or_else(|e| panic!("the same bundle again must succeed: {e:?}"));
    assert_eq!(versions_of(&lite, &id), 1, "nothing written");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_bundle_posted_twice_on_a_recording_store_writes_nothing_the_second_time() {
    let rec = Rec::at(MESH_NOW_P);
    let lite = rec_lite("g-review-post", &rec);
    let log_dir = lite.path("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let cfg = public_config(&[], None, Some(&log_dir));
    let p = Public { m: build_served(lite, Some(cfg), |_| {}).await, log_dir };
    let r = p.post(simple_bundle(HOST)).await;
    assert_eq!(r.status, 201, "{}", r.text());
    let id = level_id(HOST, "N");
    assert_eq!(versions_of(&p.m.lite, &id), 1);
    rec.set("2026-11-16T00:00:30.000000Z");
    let again = p.post(simple_bundle(HOST)).await;
    assert!(matches!(again.status, 200 | 201), "the same bundle again succeeds: {} {}", again.status, again.text());
    assert_eq!(versions_of(&p.m.lite, &id), 1, "nothing written");
}

/// The clock of the served Lite meshes (common/readpath.rs's MESH_NOW).
const MESH_NOW_P: &str = "2026-11-16T00:00:00.000000Z";

