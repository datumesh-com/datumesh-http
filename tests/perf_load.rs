//! CONTRACT-perf L-T 5-8: batched loading (0.3.3) changes no receipt, version, count or error.
//! The golden files in `tests/golden/perf_load/` were recorded with the 0.3.2 code (one round trip
//! after another) and every later version must reproduce them byte for byte. To record them again
//! (only on purpose): `DATUMESH_BLESS=1 cargo test --test perf_load`, or, where the environment is not
//! passed on (`cargo-perf.sh`), with an empty file `tests/golden/perf_load/BLESS` present (remove it after).
//!
//! The fixture: one stored function with three arguments (`entity`, `metric`, a valid `date`) and
//! 2,600 cell lines (line 1 is the function line, cell `k` (0-based) is file line `k + 2`), so the
//! file crosses two chunks of 1,000 cells. SQLite store, fixed clock, fixed key.

mod common;

use std::path::PathBuf;

use common::lite::*;
use common::pub031::*;
use datumesh_core::{Arg, Args, Cell, Resolver, Scalar, Type};
use datumesh_store::Store;

const F: &str = "mesh-test.datumesh.com/scratch_perf/series";
const N: usize = 2_600;

/// Three arguments; `date` is the only valid one (or none, for the second binding of test 8).
fn sig(date_valid: bool) -> String {
    format!(
        r#"{{"args":{{"date":{{"type":"date","valid":{date_valid}}},"entity":{{"type":"string","valid":false}},"metric":{{"type":"string","valid":false}}}},"order":["entity","metric","date"],"returns":{{"type":"number","unit":"1"}}}}"#
    )
}

fn fn_hash(date_valid: bool) -> String {
    descriptor_hash(&stored_descriptor(&sig(date_valid)))
}

fn parts(k: usize) -> (String, String, String) {
    (format!("E{}", k % 130), format!("M{}", k / 130), format!("2026-03-{:02}", k % 28 + 1))
}

/// Cell `k` of `F`; `tx`, `asof` left out when `None`.
fn cell_line(k: usize, value: &str, tx: Option<&str>, asof: Option<&str>) -> String {
    use datumesh_core::json::Value;
    let (e, m, d) = parts(k);
    let args = parse(&format!(
        r#"{{"date":{{"type":"date","value":"{d}"}},"entity":{{"type":"string","value":"{e}"}},"metric":{{"type":"string","value":"{m}"}}}}"#
    ));
    let cell = Value::Object(vec![("args".into(), args), ("fn".into(), st(F))]);
    let mut members = vec![
        ("cell", cell),
        ("licence", st("CC0-1.0")),
        ("summary", st("Synthetic perf data.")),
        ("value", parse(&format!(r#"{{"type":"number","value":"{value}"}}"#))),
    ];
    for (key, v) in [("tx", tx), ("asof", asof)] {
        if let Some(v) = v {
            members.push((key, st(v)));
        }
    }
    jcs(members)
}

fn cell_id(k: usize, date_valid: bool) -> String {
    let (e, m, d) = parts(k);
    let mut args = Args::new();
    for (name, t, v) in [("date", Type::Date, d), ("entity", Type::String, e), ("metric", Type::String, m)] {
        args.insert(name.to_string(), Arg::Value(Scalar::from_lexical(t, &v).unwrap()));
    }
    let (ns, f) = F.rsplit_once('/').unwrap();
    Cell { ns: ns.to_string(), fn_name: f.to_string(), fn_hash: fn_hash(date_valid), args }.id()
}

fn value(k: usize) -> String {
    format!("{}.{}", k / 100 + 1, (k * 37) % 9_999 * 10 + 1)
}

/// Cell `k` of the plain file: some lines with an explicit `tx`/`asof`, the rest take the clock.
fn plain_cell(k: usize, explicit: bool) -> String {
    match (explicit, k) {
        (true, k) if k % 7 == 0 => cell_line(k, &value(k), Some(NOW), Some(NOW)),
        (true, k) if k % 11 == 0 => cell_line(k, &value(k), Some(T1), Some(T2)),
        (true, k) if k % 13 == 0 => cell_line(k, &value(k), None, Some(T1)),
        _ => cell_line(k, &value(k), None, None),
    }
}

fn plain_lines(explicit: bool) -> Vec<String> {
    let mut lines = vec![function_line(F, &sig(true), None)];
    lines.extend((0..N).map(|k| plain_cell(k, explicit)));
    lines
}

// ---------------------------------------------------------------------------------------------
// Golden files

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/perf_load").join(name)
}

fn check_golden(name: &str, got: &str) {
    let path = golden_path(name);
    let marker = path.with_file_name("BLESS");
    if std::env::var("DATUMESH_BLESS").is_ok_and(|v| v == "1") || marker.exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, got).unwrap();
        return;
    }
    let want =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e} (DATUMESH_BLESS=1 records)", path.display()));
    if want != got {
        let first = want.lines().zip(got.lines()).position(|(a, b)| a != b);
        panic!(
            "{name} differs from the golden (first differing line: {first:?}; golden {} lines, got {})",
            want.lines().count(),
            got.lines().count()
        );
    }
}

/// The store's state for the cells `ids` (in order): a SHA-256 over the ordered receipt bytes, the
/// first and last 3 receipts in full, and every version (cell id, tx, receipt hash).
fn state_of(lite: &Lite, ids: &[String]) -> String {
    let mut out = String::new();
    let mut all = Vec::new();
    let mut versions = String::new();
    let mut receipts = Vec::new();
    for id in ids {
        let v = lite.store.latest(id).unwrap().unwrap_or_else(|| panic!("no version of {id}"));
        let bytes =
            lite.store.receipt(&v.receipt_hash).unwrap_or_else(|| panic!("no receipt {}", v.receipt_hash)).into_owned();
        all.extend_from_slice(&bytes);
        all.push(b'\n');
        versions.push_str(&format!("version {} {} {}\n", v.cell_id, v.tx, v.receipt_hash));
        receipts.push(bytes);
    }
    out.push_str(&format!("receipts {} sha256 {}\n", receipts.len(), datumesh_core::sha256_hex(&all)));
    for (i, r) in receipts.iter().enumerate() {
        if i < 3 || i + 3 >= receipts.len() {
            out.push_str(&format!("receipt {i} {}\n", String::from_utf8_lossy(r)));
        }
    }
    out.push_str(&versions);
    out
}

fn published_line(p: &datumesh_http::publish::Published) -> String {
    format!("published {p:?}\n")
}

fn error_line(e: &datumesh_http::publish::PublishError) -> String {
    format!("error line {} kind {:?}\ntext {e}\n", e.line, e.kind)
}

// ---------------------------------------------------------------------------------------------
// L-T 5

/// 2,600 cells with some explicit `tx`/`asof`: every receipt, version and count as 0.3.2.
#[test]
fn t5_receipts_and_versions_identical() {
    let lite = Lite::new("perf-t5");
    let f = write(&lite, "f.jsonl", &plain_lines(true));
    let p = run(&lite, &f, NOW).unwrap();
    assert_eq!((p.functions, p.cells, p.receipts, p.versions), (1, N, N, N), "{p:?}");
    let ids: Vec<String> = (0..N).map(|k| cell_id(k, true)).collect();
    let got = published_line(&p) + &state_of(&lite, &ids);
    check_golden("t5_plain.golden", &got);
}

// ---------------------------------------------------------------------------------------------
// L-T 6

/// The file without `tx` re-run at a later clock: skipped through the skip rule, nothing written.
#[test]
fn t6_rerun_at_a_later_clock_publishes_nothing() {
    let lite = Lite::new("perf-t6");
    let f = write(&lite, "f.jsonl", &plain_lines(false));
    run(&lite, &f, NOW).unwrap();
    let ids: Vec<String> = (0..N).map(|k| cell_id(k, true)).collect();
    let state = state_of(&lite, &ids);
    let before = snapshot(&lite);
    let again = run(&lite, &f, T2).unwrap();
    assert_eq!((again.functions, again.cells, again.receipts, again.versions), (0, 0, 0, 0), "{again:?}");
    assert_eq!(snapshot(&lite), before, "the re-run wrote something");
    assert_eq!(state_of(&lite, &ids), state);
}

// ---------------------------------------------------------------------------------------------
// L-T 7

/// Line 1,500 (cell 1,498, no `tx`) is refused: a version at T1, after its tx (the clock), that is
/// not this receipt, stored before the run. The same error as 0.3.2, nothing written.
#[test]
fn t7_line_1500_refused_for_a_version_that_is_not_its_receipt() {
    let lite = Lite::new("perf-t7a");
    let setup = write(
        &lite,
        "setup.jsonl",
        &[function_line(F, &sig(true), None), cell_line(1498, "999.5", Some(T1), Some(T1))],
    );
    run(&lite, &setup, NOW).unwrap();
    let f = write(&lite, "f.jsonl", &plain_lines(false));
    let before = snapshot(&lite);
    let e = run(&lite, &f, NOW).expect_err("the file must be refused");
    assert_eq!(e.line, 1500, "{e}");
    assert_eq!(snapshot(&lite), before, "a refused file wrote something");
    check_golden("t7_line_1500.golden", &error_line(&e));
}

/// Line 1,100 (cell 1,098) fails the version rule while line 1,200 (cell 1,198) fails in
/// preparation (tx after asof): the earlier line is reported.
#[test]
fn t7_earlier_version_rule_failure_beats_a_later_preparation_failure() {
    let lite = Lite::new("perf-t7b");
    let setup = write(
        &lite,
        "setup.jsonl",
        &[function_line(F, &sig(true), None), cell_line(1098, "999.5", Some(T1), Some(T1))],
    );
    run(&lite, &setup, NOW).unwrap();
    let mut lines = plain_lines(false);
    lines[1199] = cell_line(1198, &value(1198), Some(T2), Some(T1));
    let f = write(&lite, "f.jsonl", &lines);
    let before = snapshot(&lite);
    let e = run(&lite, &f, NOW).expect_err("the file must be refused");
    assert_eq!(e.line, 1100, "{e}");
    assert_eq!(snapshot(&lite), before, "a refused file wrote something");
    // Without line 1,100's problem the preparation failure at line 1,200 is the one reported.
    lines[1099] = cell_line(1098, "999.5", Some(T1), Some(T1));
    let g = write(&lite, "g.jsonl", &lines);
    let e2 = run(&lite, &g, NOW).expect_err("the file must be refused");
    assert_eq!(e2.line, 1200, "{e2}");
    check_golden("t7_line_1100_before_1200.golden", &(error_line(&e) + &error_line(&e2)));
}

// ---------------------------------------------------------------------------------------------
// L-T 8

/// A function line in the middle re-binds the name (bound at T1, other hash): the cells before
/// are of the first binding, the cells after (explicit `tx` T1) of the new one.
#[test]
fn t8_a_function_line_in_the_middle_rebinds_the_name() {
    let lite = Lite::new("perf-t8");
    let mut lines = vec![function_line(F, &sig(true), None)];
    lines.extend((0..1300).map(|k| plain_cell(k, false)));
    lines.push(function_line(F, &sig(false), Some(T1)));
    lines.extend((1300..N).map(|k| cell_line(k, &value(k), Some(T1), Some(T1))));
    let f = write(&lite, "f.jsonl", &lines);
    let p = run(&lite, &f, NOW).unwrap();
    assert_eq!((p.functions, p.cells, p.versions), (2, N, N), "{p:?}");
    let ids: Vec<String> = (0..N).map(|k| cell_id(k, k < 1300)).collect();
    let got = published_line(&p) + &state_of(&lite, &ids);
    check_golden("t8_rebind.golden", &got);
}
