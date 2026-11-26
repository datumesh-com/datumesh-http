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

// ---- Item 4: the keys for a past read's receipt -----------------------------------------------

const OTHER: &str = "gauges.example.org";
const MESH: &str = "mesh-test.datumesh.com";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn verify_finds_the_keys_of_a_past_reads_members() {
    let n = Node::start(MESH, |c| {
        c.namespaces = vec![MESH.to_string(), OTHER.to_string()];
        c.rate.enabled = false;
    })
    .await;
    let okey = key_for(OTHER);
    bl(|| n.store.put_namespace(OTHER, &key_doc(OTHER, &okey), BIND_TX).unwrap());
    let gauge = format!("{OTHER}/gauge");
    let date_sig =
        r#"{"args":{"d":{"type":"date","valid":false}},"order":["d"],"returns":{"type":"number","unit":"1"}}"#;
    n.stored(&gauge, date_sig);
    for (i, v) in [("2016-11-20", "5"), ("2016-11-21", "6")] {
        n.put_value_signed(&okey, &gauge, &args(&[("d", d(i))]), common::n(v), T1);
    }
    let empty = r#"{"args":{},"order":[],"returns":{"type":"number","unit":"1"}}"#;
    n.formula(&format!("{MESH}/gauges_total"), &format!("sum({gauge}(d: *))"), empty);
    // A past read (the node's clock is the system's): no member-set row.
    let r = n.get(&format!("/cell/{MESH}/gauges_total?asof=2026-06-01T00:00:00.000000Z")).await;
    assert_eq!(r.outcome(), "ok", "{}", r.text());
    let receipt = r.json().get("receipt").unwrap().to_jcs();
    let v = n.post("/verify", receipt).await;
    let j = v.json();
    assert_eq!(j.get("ok"), Some(&datumesh_core::json::Value::Bool(true)), "{}", v.text());
    assert_eq!(j.get("level").and_then(datumesh_core::json::Value::as_str), Some("recomputed"), "{}", v.text());
}

// ---- Item 5: the time budget applies to a rebuild ---------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rebuild_over_the_time_budget_is_e_eval_budget() {
    let n = Node::start(MESH, |c| {
        c.budget = Duration::from_secs(60);
        c.rate.enabled = false;
    })
    .await;
    let site =
        r#"{"args":{"site":{"type":"string","valid":false}},"order":["site"],"returns":{"type":"number","unit":"1"}}"#;
    let reading = format!("{MESH}/reading");
    n.stored(&reading, site);
    for k in 0..1000 {
        n.put_value(&reading, &args(&[("site", s(&format!("s{k:04}")))]), common::n("1"), T1);
    }
    let empty = r#"{"args":{},"order":[],"returns":{"type":"number","unit":"1"}}"#;
    n.formula(&format!("{MESH}/all"), &format!("sum({reading}(site: *))"), empty);
    let r = n.get(&format!("/cell/{MESH}/all?asof={T2}")).await;
    assert_eq!(r.outcome(), "ok", "{}", r.text());
    let rec = datumesh_core::Receipt::from_json(&r.json().get("receipt").unwrap().to_jcs()).unwrap();
    let parts: Vec<&str> = rec.inputs()[0].split(':').collect();
    let path = format!("/manifest/{}/{}?asof={}", parts[1], parts[2], rec.asof());
    assert_eq!(n.get(&path).await.status, 200, "with time: served");
    // The same database, a mesh with a budget of 1 ms: the rebuild of 1,000 members is refused.
    let mut setup = Setup::new(MESH);
    setup.db = Some(n.db.clone());
    let none = Node::start_with(
        setup,
        |c| {
            c.budget = Duration::from_millis(1);
            c.rate.enabled = false;
        },
        |b| b,
    )
    .await;
    let m = none.get(&path).await;
    assert_eq!(m.status, 503, "{}", m.text());
    assert!(m.text().contains("E_EVAL_BUDGET"), "{}", m.text());
}

// ---- Second review (27 Nov) -----------------------------------------------------------------------

#[test]
fn a_file_with_two_versions_of_one_cell_run_twice_on_a_recording_store_writes_nothing_the_second_time() {
    // B5: publish-stored skips a line when its receipt is any version of the cell at its signed tx. A
    // recording store with a fixed clock: both versions are recorded at the same instant's ordering.
    let rec = Rec::at(NOW);
    let lite = rec_lite("g-review-two-versions", &rec);
    let c = calculus::calc();
    let cell = |value: &str, tx: &str| {
        serde_json::json!({
            "cell": {"fn": c.reading, "args": {"site": {"type": "string", "value": "N"}}},
            "licence": "CC0-1.0", "summary": "Synthetic test data.",
            "value": {"type": "number", "value": value}, "tx": tx, "asof": tx,
        })
        .to_string()
    };
    let fns = common::pub031::write(
        &lite,
        "fns.jsonl",
        &[common::pub031::function_line(&c.reading, &calculus::reading_sig(), None)],
    );
    common::pub031::run(&lite, &fns, BEFORE_RUN).unwrap_or_else(|e| panic!("the function: {e}"));
    let lines = vec![cell("10", "2026-11-15T11:30:00.000000Z"), cell("11", "2026-11-15T11:45:00.000000Z")];
    let f = common::pub031::write(&lite, "two.jsonl", &lines);
    common::pub031::run(&lite, &f, NOW).unwrap_or_else(|e| panic!("first run: {e}"));
    let fn_hash = lite.store.resolve_name(&c.reading, END).unwrap().expect("bound");
    let id = Cell {
        ns: c.reading.rsplit_once('/').unwrap().0.into(),
        fn_name: "reading".into(),
        fn_hash,
        args: args(&[("site", s("N"))]),
    }
    .id();
    assert_eq!(versions_of(&lite, &id), 2);
    common::pub031::run(&lite, &f, NOW).unwrap_or_else(|e| panic!("the second run must succeed: {e}"));
    assert_eq!(versions_of(&lite, &id), 2, "nothing written");
}

/// Before the run clock: when the function of the two-version file is bound.
const BEFORE_RUN: &str = "2026-11-15T11:00:00.000000Z";

#[test]
fn a_bundle_correction_signed_earlier_is_accepted_when_recorded_later_and_again_is_a_no_op() {
    // C: bundle import follows publish-stored's rule: a correction signed earlier than the cell's latest
    // is accepted when it is recorded later (recording_time), and the same receipt again is a no-op.
    let rec = Rec::at(NOW);
    let lite = rec_lite("g-review-bundle-backfill", &rec);
    let owned: Vec<String> = Vec::new();
    bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(NOW), &simple_bundle(HOST))
        .unwrap_or_else(|e| panic!("first: {e:?}"));
    let id = level_id(HOST, "N");
    // A correction of N signed at 10:00, before its version of 11:00 (TX), recorded later.
    rec.set(T2_RUN);
    let correction = simple_bundle_with(
        HOST,
        &seed_for(HOST),
        vec![level_receipt(HOST, "N", "43", "2026-11-15T10:00:00.000000Z")],
        None,
    );
    bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(T2_RUN), &correction)
        .unwrap_or_else(|e| panic!("a correction signed earlier, recorded later, is accepted: {e:?}"));
    let latest = lite.store.latest(&id).unwrap().unwrap();
    assert_eq!((latest.tx.as_str(), latest.recorded.as_str()), ("2026-11-15T10:00:00.000000Z", T2_RUN));
    assert_eq!(versions_of(&lite, &id), 2);
    rec.set("2026-11-15T12:01:00.000000Z");
    bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock("2026-11-15T12:01:00.000000Z"), &correction)
        .unwrap_or_else(|e| panic!("the same bundle again is a no-op: {e:?}"));
    assert_eq!(versions_of(&lite, &id), 2, "nothing written");
}
