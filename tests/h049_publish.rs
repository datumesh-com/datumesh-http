//! http 0.4.9, CONTRACT-0.4.9 H6: a publish does work in proportion to what it writes, not to what the
//! function already holds. Found on mesh-test: 14 cells and 14 tests onto a function with 8,120 tests took
//! 833 s. These tests are written first and FAIL (by time, or by the old rows being rerun) until then.
//!
//! The function holds 2,000 tests (written into the store, as run on 2026-01-01); a file writes 14 new cells
//! and 14 new tests. It must finish quickly, and the old rows must not be run again: their `last_run_at`
//! stays. (Uses the store 0.5.2 `name_function_tests`.)

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/h049.rs"]
mod h049;
#[path = "common/publish4.rs"]
mod publish4;

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use common::lite::*;
use common::pub031::{function_line, run, write, T1};
use datumesh_store::Store;
use h049::*;

const F: &str = "mesh-test.datumesh.com/h049/top_up/level";
const OLD: usize = 2000;

#[test]
fn fourteen_cells_and_tests_onto_two_thousand_tests_is_fast_and_reruns_nothing_old() {
    let lite = Lite::new("h6-topup");
    // the function and its 2,000 old cells (no tests in this file)
    let mut first = vec![function_line(F, &sig(), None)];
    for i in 0..OLD {
        first.push(cell(F, &format!("o{i:04}"), "1"));
    }
    let f1 = write(&lite, "first.jsonl", &first);
    run(&lite, &f1, NOW).unwrap_or_else(|e| panic!("{e}"));
    let hash = hash_in(&lite, F);
    // 2,000 old tests, as recorded by a run on 2026-01-01
    let mut old_cases = BTreeSet::new();
    for i in 0..OLD {
        old_cases.insert(put_raw(&lite, &hash, &tline(F, &format!("o{i:04}"), "1"), Some((OLD_RUN, "pass"))));
    }
    let names: Vec<(String, String)> = old_cases.iter().map(|c| (c.clone(), F.to_string())).collect();
    lite.store.name_function_tests(&hash, &names).unwrap();
    assert_eq!(lite.store.function_tests(&hash).unwrap().len(), OLD);

    // the top-up: 14 new cells, 14 new tests
    let mut second = Vec::new();
    for i in 0..14 {
        second.push(cell(F, &format!("n{i:02}"), "2"));
    }
    for i in 0..14 {
        second.push(tline(F, &format!("n{i:02}"), "2"));
    }
    let f2 = write(&lite, "second.jsonl", &second);
    let t = Instant::now();
    let p = run(&lite, &f2, T1).unwrap_or_else(|e| panic!("{e}"));
    let took = t.elapsed();
    assert!(took < Duration::from_secs(20), "the top-up took {took:?} (on 0.4.8 it is proportional to the old tests)");
    assert_eq!(p.versions, 14, "{p:?}");

    let rows = lite.store.function_tests(&hash).unwrap();
    assert_eq!(rows.len(), OLD + 14, "the old and the new tests");
    let (mut old_seen, mut new_seen) = (0, 0);
    for r in &rows {
        if old_cases.contains(&r.case) {
            old_seen += 1;
            assert_eq!(r.last_run_at.as_deref(), Some(OLD_RUN), "an old test was run again: {}", r.case);
            assert_eq!(r.last_result.as_deref(), Some("pass"));
        } else {
            new_seen += 1;
            let at = r.last_run_at.as_deref().unwrap_or_else(|| panic!("a new test never ran: {}", r.case));
            assert_ne!(at, OLD_RUN);
            assert_eq!(r.last_result.as_deref(), Some("pass"), "{}", r.case);
        }
    }
    assert_eq!((old_seen, new_seen), (OLD, 14));
    // the new tests were named when stored
    let summary = lite.store.function_test_summary(&hash, Some(F)).unwrap();
    assert_eq!(summary.count as usize, OLD + 14, "{summary:?}");
}

/// The same file twice: the second run changes no row and runs nothing old either (cheap, idempotent).
#[test]
fn a_second_run_of_the_top_up_changes_nothing() {
    let lite = Lite::new("h6-twice");
    let mut lines = vec![function_line(F, &sig(), None)];
    for i in 0..14 {
        lines.push(cell(F, &format!("n{i:02}"), "2"));
    }
    for i in 0..14 {
        lines.push(tline(F, &format!("n{i:02}"), "2"));
    }
    let f = write(&lite, "f.jsonl", &lines);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    let hash = hash_in(&lite, F);
    let before = lite.store.function_tests(&hash).unwrap();
    let again = run(&lite, &f, T1).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(again.versions, 0, "{again:?}");
    let after = lite.store.function_tests(&hash).unwrap();
    assert_eq!(before.len(), after.len());
    for (b, a) in before.iter().zip(after.iter()) {
        assert_eq!((&b.case, &b.last_result), (&a.case, &a.last_result));
    }
}
