//! CONTRACT-perf L-T9, L-T10 (H5) and R-T3 (H6): `publish_stored_lanes` (and the CLI) writes cells on
//! parallel lanes
//! (`DATUMESH_PUBLISH_LANES`) without changing any receipt, version or row. Its own test binary,
//! because it sets that variable; the two tests here take a lock around every run, so none reads
//! the variable while the other sets it. PostgreSQL, through the `Node` harness.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use common::lite::{FixedClock, NOW};
use common::pub031::{descriptor_hash, function_line, jcs, parse, st, stored_descriptor};
use common::*;
use datumesh_core::{Arg, Args, Cell, Scalar, Type};
use datumesh_http::publish::{publish_stored_lanes, publish_stored_with, PublishOptions, Published};

const A: &str = "mesh-a.example";
const F: &str = "mesh-a.example/scratch_perf/lanes";
const CELLS: usize = 40;
const LINES: usize = 3_000;

static ENV: Mutex<()> = Mutex::new(());

const SIG: &str = r#"{"args":{"entity":{"type":"string","valid":false},"metric":{"type":"string","valid":false}},"order":["entity","metric"],"returns":{"type":"number","unit":"1"}}"#;

/// The (entity, metric) of cell `i`: 8 x 5 = 40 distinct cells.
fn parts(i: usize) -> (String, String) {
    (format!("E{}", i % 8), format!("M{}", i / 8))
}

fn cell_id(i: usize) -> String {
    let (e, m) = parts(i);
    let mut args = Args::new();
    for (name, v) in [("entity", e), ("metric", m)] {
        args.insert(name.to_string(), Arg::Value(Scalar::from_lexical(Type::String, &v).unwrap()));
    }
    let (ns, f) = F.rsplit_once('/').unwrap();
    Cell { ns: ns.to_string(), fn_name: f.to_string(), fn_hash: descriptor_hash(&stored_descriptor(SIG)), args }.id()
}

/// The `tx` of version `n` (0-based) of every cell: later than the binding (the clock, `NOW`).
fn tx_of(n: usize) -> String {
    plus(NOW, (n as i64 + 1) * 1_000_000)
}

/// Line `k` (0-based): cell `k % 40`, its version `k / 40`, explicit `tx` = `asof`.
fn cell_line(k: usize) -> String {
    let (e, m) = parts(k % CELLS);
    let args =
        parse(&format!(r#"{{"entity":{{"type":"string","value":"{e}"}},"metric":{{"type":"string","value":"{m}"}}}}"#));
    let cell = datumesh_core::json::Value::Object(vec![("args".into(), args), ("fn".into(), st(F))]);
    let tx = tx_of(k / CELLS);
    jcs(vec![
        ("asof", st(&tx)),
        ("cell", cell),
        ("licence", st("CC0-1.0")),
        ("summary", st("Synthetic lanes data.")),
        ("tx", st(&tx)),
        ("value", parse(&format!(r#"{{"type":"number","value":"{}.{}"}}"#, k / CELLS + 1, (k * 7 % 1000) * 10 + 1))),
    ])
}

fn file(name: &str) -> std::path::PathBuf {
    let mut lines = vec![function_line(F, SIG, None)];
    lines.extend((0..LINES).map(cell_line));
    let p = out_dir().join(format!("lanes-{}-{name}.jsonl", std::process::id()));
    std::fs::write(&p, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    p
}

fn publish(node: &Node, path: &Path, lanes: &str) -> Published {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("DATUMESH_PUBLISH_LANES", lanes);
    let opts = PublishOptions { clock: Arc::new(FixedClock(NOW)), backfill: None };
    let r = bl(|| publish_stored_lanes(&*node.store, &node.key, A, &[A.to_string()], path, &opts)).unwrap();
    std::env::remove_var("DATUMESH_PUBLISH_LANES");
    r
}

/// `publish_stored_with` (always one lane) with the lanes variable set to `lanes`.
fn publish_one_lane(node: &Node, path: &Path, lanes: &str) -> Published {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("DATUMESH_PUBLISH_LANES", lanes);
    let opts = PublishOptions { clock: Arc::new(FixedClock(NOW)), backfill: None };
    let r = bl(|| publish_stored_with(&*node.store, &node.key, A, &[A.to_string()], path, &opts)).unwrap();
    std::env::remove_var("DATUMESH_PUBLISH_LANES");
    r
}

/// Runs `f` on a connection to the node's database (opened and dropped off the runtime).
fn sql<T>(node: &Node, f: impl FnOnce(&mut datumesh_store::pg_unstable::postgres::Client) -> T) -> T {
    use datumesh_store::pg_unstable::postgres::{Config, NoTls};
    let cfg: Config = node.db.url().parse().unwrap();
    bl(|| f(&mut cfg.connect(NoTls).unwrap()))
}

/// Row counts of receipts, versions, cells, cell_args.
fn counts(node: &Node) -> [i64; 4] {
    sql(node, |c| {
        ["receipts", "versions", "cells", "cell_args"]
            .map(|t| c.query_one(&format!("SELECT count(*) FROM {t}"), &[]).unwrap().get(0))
    })
}

/// Everything observable about the cells, per id: latest, version_asof at three instants, every
/// version's receipt bytes.
/// A version as the lanes must agree on it: every field but `seq` (store 0.6.5), the insertion
/// sequence, which follows the order the lanes' writes interleave in. `recorded` is compared: these
/// nodes' stores have no recording clock, so it is the signed tx on every lane.
fn ver(v: &datumesh_store::Version) -> String {
    format!(
        "cell {} tx {} receipt {} asof {} withdrawn {} recorded {}",
        v.cell_id, v.tx, v.receipt_hash, v.asof, v.withdrawn, v.recorded
    )
}

fn state(node: &Node) -> BTreeMap<String, String> {
    bl(|| {
        (0..CELLS)
            .map(|i| {
                let id = cell_id(i);
                let mut out = String::new();
                let latest = node.store.latest(&id).unwrap().unwrap_or_else(|| panic!("no version of {id}"));
                out.push_str(&format!("latest {}\n", ver(&latest)));
                for n in [0, LINES / CELLS / 2, LINES / CELLS - 1] {
                    // Mid-way between version n and n+1 (after the last: the same as latest).
                    let at = plus(&tx_of(n), 500_000);
                    out.push_str(&format!(
                        "asof {at} {:?}\n",
                        node.store.version_asof(&id, &at).unwrap().as_ref().map(ver)
                    ));
                }
                for n in 0..LINES / CELLS {
                    let tx = tx_of(n);
                    let v = node.store.version_asof(&id, &tx).unwrap().unwrap();
                    assert_eq!(v.tx, tx, "{id}");
                    let bytes = node.store.receipt(&v.receipt_hash).unwrap();
                    out.push_str(&format!("receipt {n} {}\n", String::from_utf8_lossy(&bytes)));
                }
                (id, out)
            })
            .collect()
    })
}

/// The cell ids of the versions in physical (insertion) order.
fn insertion_order(node: &Node) -> Vec<String> {
    sql(node, |c| {
        c.query("SELECT cell_id FROM versions ORDER BY ctid", &[]).unwrap().iter().map(|r| r.get(0)).collect()
    })
}

async fn node() -> Node {
    let node = Node::start(A, |_| {}).await;
    let _ = &node;
    node
}

/// L-T9: one lane and four lanes give the same latest, version_asof, receipts and row counts; on
/// one lane the versions are inserted in file order, on four each cell's versions stay in file order.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lanes_one_and_four_store_the_same() {
    let (one, four) = (node().await, node().await);
    let (f1, f4) = (file("one"), file("four"));
    let (p1, p4) = (publish(&one, &f1, "1"), publish(&four, &f4, "4"));
    for p in [&p1, &p4] {
        assert_eq!((p.functions, p.cells, p.receipts, p.versions), (1, LINES, LINES, LINES), "{p:?}");
    }
    assert_eq!(p1, p4);
    let want = [LINES as i64, LINES as i64, CELLS as i64, 2 * CELLS as i64];
    assert_eq!(counts(&one), want);
    assert_eq!(counts(&four), want);
    let (s1, s4) = (state(&one), state(&four));
    assert_eq!(s1.len(), CELLS);
    assert!(s1 == s4, "the states differ between 1 and 4 lanes");

    // Insertion order: file order on one lane (H2); on four lanes every cell still has all its versions.
    let expected: Vec<String> = (0..LINES).map(|k| cell_id(k % CELLS)).collect();
    assert!(insertion_order(&one) == expected, "one lane: versions not inserted in file order");
    let mut per: BTreeMap<String, usize> = BTreeMap::new();
    for id in insertion_order(&four) {
        *per.entry(id).or_insert(0) += 1;
    }
    assert_eq!(per.len(), CELLS);
    assert!(per.values().all(|&n| n == LINES / CELLS));
    let _ = std::fs::remove_file(f1);
    let _ = std::fs::remove_file(f4);
}

/// L-T10: the same file again, on four lanes, writes 0 cells.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn republishing_on_four_lanes_writes_nothing() {
    let n = node().await;
    let f = file("rerun");
    let first = publish(&n, &f, "4");
    assert_eq!((first.cells, first.versions), (LINES, LINES), "{first:?}");
    let before = (counts(&n), state(&n));
    let again = publish(&n, &f, "4");
    // `receipts` counts receipts "new or already there"; nothing new is written.
    assert_eq!((again.functions, again.versions), (0, 0), "{again:?}");
    assert_eq!((counts(&n), state(&n)), before);
    let _ = std::fs::remove_file(f);
}

/// R-T3 (H6): `publish_stored_with` writes on one lane whatever the variable says, so the
/// physical order of `versions` is file order and the result equals that of
/// `publish_stored_lanes` on the same file.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn publish_stored_with_writes_in_file_order_and_equals_the_lanes() {
    let (with, lanes) = (node().await, node().await);
    let (fw, fl) = (file("with"), file("lanes"));
    let (pw, pl) = (publish_one_lane(&with, &fw, "4"), publish(&lanes, &fl, "4"));
    assert_eq!((pw.functions, pw.cells, pw.receipts, pw.versions), (1, LINES, LINES, LINES), "{pw:?}");
    assert_eq!(pw, pl);
    let want = [LINES as i64, LINES as i64, CELLS as i64, 2 * CELLS as i64];
    assert_eq!(counts(&with), want);
    assert_eq!(counts(&lanes), want);
    assert!(state(&with) == state(&lanes), "publish_stored_with and publish_stored_lanes store different data");
    let expected: Vec<String> = (0..LINES).map(|k| cell_id(k % CELLS)).collect();
    assert!(
        insertion_order(&with) == expected,
        "publish_stored_with: versions not inserted in file order (DATUMESH_PUBLISH_LANES=4 must not apply)"
    );
    let _ = std::fs::remove_file(fw);
    let _ = std::fs::remove_file(fl);
}
