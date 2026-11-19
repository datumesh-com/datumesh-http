//! Build 2 Part 3, E5 "Rerun on rebind": when a name is rebound, every function that reads that
//! name, transitively through the bindings the engine records, reruns its tests; the failures are
//! recorded and shown on the function (`describe` `tests.counterexamples`), never hidden, and they
//! never block the rebind.
//!
//! The world: `per_unit` (reads the stored `reading`), `scaled` = `per_unit(site, 2) + 1` with the
//! tests N = 6 and an absent site, and `outer` = `scaled * 10` with N = 60 and an absent site. The
//! tests have no `asof`, so they are about the cell as of now (a pinned past answer never changes).
//! Run 1 publishes it all; run 2, later, rebinds `per_unit` to three times the reading (so
//! `scaled(N)` is 16 and `outer(N)` 160); run 3 rebinds it back. The runs are `publish-stored`
//! runs (the rerun is done by every write path that rebinds a name, the local one included, and is
//! done before the publish returns).

mod common;
#[path = "common/calculus.rs"]
mod calculus;

use calculus::*;
use common::lite::*;
use common::pub031::T1;
use common::pub031::T2;
use common::readpath::*;
use common::world::call_path;

fn run1(c: &Calc) -> Vec<String> {
    let mut v = data_lines(c);
    v.push(per_unit_line(c, None));
    v.push(scaled_line(c));
    v.extend(scaled_tests(c));
    v.push(outer_line(c));
    v.extend(outer_tests(c));
    v
}

fn describe_path(name: &str) -> String {
    format!("/describe/{}", call_path(name))
}

async fn counterexamples(m: &Served, name: &str) -> (u64, u64) {
    let d = m.get(&describe_path(name)).await;
    (d["tests"]["count"].as_u64().unwrap(), d["tests"]["counterexamples"].as_u64().unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rebind_reruns_the_tests_of_its_dependants_and_shows_the_failures() {
    let c = calc();
    // run 1: all well; run 2: the rebind
    let m = serve_runs(
        "rr-1",
        &[(NOW, run1(&c)), (T1, vec![per_unit_v2_line(&c, None)])],
        Some(test_cells_config()),
    )
    .await;

    // the rebind took effect: it is not blocked by the dependants' failures
    let v = m.get(&format!("/cell/{}?site=N&units=2", c.per_unit)).await;
    assert_eq!(v["receipt"]["value"], "15", "{v}");

    // `scaled` reads per_unit directly, `outer` through scaled: both rerun and fail on N
    assert_eq!(counterexamples(&m, &c.scaled).await, (2, 1));
    assert_eq!(counterexamples(&m, &c.outer).await, (2, 1));

    // and the cells say which: N is false, the absent site is still true
    for (f, tests) in [(&c.scaled, scaled_tests(&c)), (&c.outer, outer_tests(&c))] {
        let hash = fn_hash(&m, f);
        let n_case = m.get(&format!("/cell/{}", call_path(&test_case_call(&hash, &case_id(&tests[0]))))).await;
        let absent = m.get(&format!("/cell/{}", call_path(&test_case_call(&hash, &case_id(&tests[1]))))).await;
        assert_eq!(n_case["receipt"]["value"], "false", "{f}: {n_case}");
        assert_eq!(absent["receipt"]["value"], "true", "{f}: {absent}");
    }

    // run 3 rebinds per_unit back to the first body (the same function hash as run 1): the
    // dependants rerun and the counterexamples go away
    let f = common::pub031::write(&m.lite, "run3.jsonl", &[per_unit_line(&c, None)]);
    common::pub031::run(&m.lite, &f, T2).unwrap_or_else(|e| panic!("{e}"));
    let v = m.get(&format!("/cell/{}?site=N&units=2", c.per_unit)).await;
    assert_eq!(v["receipt"]["value"], "5", "{v}");
    assert_eq!(counterexamples(&m, &c.scaled).await, (2, 0));
    assert_eq!(counterexamples(&m, &c.outer).await, (2, 0));
}

/// Nothing to rerun when no dependant has tests, and a function that does not read the rebound
/// name is not rerun (its counterexamples stay 0 although its sibling's grew).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_the_dependants_rerun() {
    let c = calc();
    let mut first = run1(&c);
    first.push(band_line(&c));
    first.extend(band_tests(&c));
    let m = serve_runs("rr-2", &[(NOW, first), (T1, vec![per_unit_v2_line(&c, None)])], None).await;
    assert_eq!(counterexamples(&m, &c.band).await, (3, 0));
    assert_eq!(counterexamples(&m, &c.scaled).await, (2, 1));
}
