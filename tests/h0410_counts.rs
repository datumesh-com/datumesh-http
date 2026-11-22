//! http 0.4.10, CONTRACT-round-0.7-P HP1, HP2 and HP3, counted: what the index reads, that a class describe
//! reads nothing, and that a write or a computed cell does not make the index read everything again.
//! Written first; FAIL until then.
//!
//! These run on PostgreSQL (`DATABASE_URL`) with the read probe of `common/h0410.rs`: the number of rows of
//! `cells`, `cell_args`, `versions` and `computed_versions` the mesh visits (the store is sealed, so there is
//! no counting wrapper; see that file). The three background builds other than the one under test are held
//! back an hour (`DATUMESH_*_FIRST_BUILD_DELAY_SECS`), so that only the index and the request read.
//!
//! The thresholds lie between what the contract's reads cost and what 0.4.9's cost, for the sizes used (measured
//! on the implementation: about 8 rows per extent cell, so 2,400 for 300 objects; reading every method of the
//! 3,300 cells is about 26,000; a full rebuild is therefore at least 2,400 rows).

mod common;
#[path = "common/h0410.rs"]
mod h0410;

use std::time::Duration;

use common::lite::NOW;
use common::pub031::{function_line, T2};
use h0410::*;

const K: &str = "mesh-test.datumesh.com/h0410/cnt/alpha";
const BASE: &str = "mesh-test.datumesh.com/h0410/cnt/base_level";
const EXTENT: usize = 300;
/// The other method holds ten times as many cells as the list.
const RATE: usize = 3_000;

/// The class `K`: `members` (the extent, 300 objects E00000..), `rate` (3,000 objects R00000.. of its own) and
/// `twice`, a formula over the stored function `BASE` (which holds a cell for the object C00001).
fn class(label: &str) -> (Fx, Cls) {
    let fx = Fx::pg(label);
    fx.publish(&[function_line(BASE, &sig_of(Kind::Level), None)], NOW);
    fx.publish(
        &[formula_of(&format!("{K}/twice"), &format!("{BASE}(base: base, quote: quote) * 2"), &sig_of(Kind::Rate))],
        NOW,
    );
    let cls = define_class_with(&fx, K, &[("members", Kind::Extent), ("rate", Kind::Rate)], &[("twice", &format!("{K}/twice"))]);
    let mut lines = cells_of(&cls, "members", Kind::Extent, "E", 0..EXTENT);
    lines.extend(cells_of(&cls, "rate", Kind::Rate, "R", 0..RATE));
    fx.publish(&lines, T2);
    fx.publish(&[method_cell(BASE, Kind::Level, "C00001", "Q")], T2);
    (fx, cls)
}

fn name(i: usize, p: &str) -> String {
    format!("{p}{i:05}")
}

async fn serve_index(fx: Fx) -> W {
    fx.probe().arm();
    let vars = held(&["index"]);
    fx.serve(config(&vars, &[("a", &format!("{K}/members"))])).await
}

/// The mesh with the first build done and a second to settle.
async fn built(fx: Fx, cls: &Cls) -> W {
    let w = serve_index(fx).await;
    let r = w.describe_until_found(&cls.id(&name(0, "E"), "Q")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    tokio::time::sleep(Duration::from_secs(2)).await;
    w
}

// ---- HP2: the build reads the declared list only ----------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_first_build_reads_the_extent_keys_cells_only() {
    let (fx, cls) = class("hp2-reads");
    let w = built(fx, &cls).await;
    let reads = w.fx.probe().reads(Group::Cells);
    assert!(reads > 0, "the build read nothing: the probe is not counting");
    // Measured: the extent-only build visits about 8 rows per extent cell (cells, cell_args, versions): 2,400.
    // Reading every method would visit that and 8 per cell of `rate`: about 26,000. The bound is 12 per object.
    assert!(
        reads <= (12 * EXTENT) as u64,
        "the index of {EXTENT} extent objects (and {RATE} cells of another method) read {reads} rows; the extent \
         alone is about {} and every method about {}",
        8 * EXTENT,
        8 * (EXTENT + RATE)
    );
    // and it holds the extent: the last object is in
    let r = w.get(&format!("/describe/{}", cls.id(&name(EXTENT - 1, "E"), "Q"))).await;
    assert_eq!(r.status, 200, "{}", r.text());
}

// ---- HP1: describe of a class reads no cell ---------------------------------------------------------------

/// The index has built (nothing: its only alias names a class that does not exist) and has not heard of `K`.
/// 0.4.9 indexed `K` inside the describe request: every cell of every method, over 20,000 rows here. The request now
/// reads the descriptor only. (The index thread may hear of `K` and read its extent a moment later, at most
/// 2,400 rows: the threshold is set between that and the old 20,000 and more.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describing_a_class_reads_no_cell() {
    let (fx, cls) = class("hp1-zero");
    fx.probe().arm();
    let vars = held(&["index"]);
    let w = fx.serve(config(&vars, &[("a", "mesh-test.datumesh.com/h0410/cnt/none/members")])).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let before = w.fx.probe().reads(Group::Cells);
    let r = w.get(&format!("/describe/{K}")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(r.text().contains(&cls.hash), "{}", r.text());
    let after = w.fx.probe().reads(Group::Cells);
    assert!(after - before < 4000, "a describe of a class with 3,300 cells visited {} rows of cells", after - before);
    // an object by its key encoding: answered from the class descriptor, no index, no cell
    let before = w.fx.probe().reads(Group::Cells);
    let r = w.get(&format!("/describe/{}", common::world::call_path(&cls.encoding(&name(5, "R"), "Q")))).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(w.fx.probe().reads(Group::Cells) - before < 4000);
}

// ---- HP3: no rebuild per write ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stored_version_is_added_to_the_index_without_a_rebuild() {
    let (fx, cls) = class("hp3-stored");
    let w = built(fx, &cls).await;
    let before = w.fx.probe().reads(Group::Cells);
    // a new object of the list, written straight to the store, then announced to the mesh
    let id = put_extent_version(&w.fx, &cls, "members", &name(99_999, "E"), "2026-11-15T12:00:05.000000Z");
    let m = w.mesh.clone();
    common::on_thread(move || m.stored_version_added(&id));
    // known at once (index_cell_now) ...
    let (s, body) = w.describe_now(&cls.id(&name(99_999, "E"), "Q"));
    assert_eq!(s, 200, "{body}");
    // ... and nothing was read again: six ticks of the index thread later a rebuild would have read >= 2,400 rows
    tokio::time::sleep(Duration::from_secs(3)).await;
    let after = w.fx.probe().reads(Group::Cells);
    assert!(after - before < 100, "a stored write made the index read {} rows", after - before);
    let (s, body) = w.describe_now(&cls.id(&name(99_999, "E"), "Q"));
    assert_eq!(s, 200, "still known: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_computed_cell_is_added_to_the_index_without_a_rebuild() {
    let (fx, cls) = class("hp3-computed");
    let w = built(fx, &cls).await;
    let before = w.fx.probe().reads(Group::Cells);
    let r = w.get(&format!("/cell/{K}/twice?base=C00001&date=2026-03-02&quote=Q")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    // the object of the computed cell is known at once, though it is in no list
    let (s, body) = w.describe_now(&cls.id("C00001", "Q"));
    assert_eq!(s, 200, "{body}");
    tokio::time::sleep(Duration::from_secs(3)).await;
    let after = w.fx.probe().reads(Group::Cells);
    assert!(after - before < 100, "a computed cell made the index read {} rows", after - before);
}
