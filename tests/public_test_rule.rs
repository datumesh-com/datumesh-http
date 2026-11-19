//! Build 2 Part 3, E5: tests for public functions, the rule datumesh-http enforces (D8).
//!
//! The surface these tests specify:
//!
//! * `publish-stored` files take `test` lines: `{"test": "<function name>", "call": "<call>",
//!   "asof"?: "<instant>", "inputs"?: {"<call of an input cell>": <typed scalar>}, "expect":
//!   <typed scalar> | {"fault": "E_FORMULA_..."}}`;
//! * `datumesh_http::publish::publish_stored_checked(store, key, host, owned, path, opts)` is
//!   `publish_stored_with` plus the public rule (the binary's `publish-stored` calls it). The
//!   existing `publish_stored*` functions keep their behaviour: tests are recorded, never required
//!   (the local mesh, and every test written before Part 3);
//! * the rule refuses a file that defines a **computed** (formula) function unless the tests held
//!   for it (the file's, and any already held for the same function hash) cover every derived edge
//!   case and exercise every branch, and every test passes:
//!   * `E_TESTS_MISSING` at the line of the formula: `PublishErrorKind::TestsMissing`; the detail
//!     names the code and the function and lists the derived cases not covered, as
//!     `zero(<arg>)` (a `number` argument that is a divisor in the body), `absent(<function>)` (an
//!     input cell read), `negative(<arg>)` (each `number` argument), `extent_lo(<arg>)` and
//!     `extent_hi(<arg>)` (a `date`/`instant` argument with a declared extent), `empty(<function>)`
//!     (a slice input), after the words `uncovered cases`, then the branches not exercised after
//!     the words `unexercised branches` (each section only when it is not empty);
//!     a derived case is covered by a test that exercises it, which the evaluator records
//!     (`datumesh_formula::coverage`): a test whose `expect` is a value or a fault;
//!   * `E_TESTS_FAILING` at the line of the failing `test`: `PublishErrorKind::TestsFailing`; the
//!     detail names the code and the case id (the first 16 hex digits of the SHA-256 of the test
//!     line's JCS) of each failing test, with what was expected and what came out;
//! * either refusal writes nothing (the store file is byte-identical);
//! * stored functions are exempt.

#[path = "common/calculus.rs"]
mod calculus;
mod common;

use calculus::*;
use common::lite::*;
use common::pub031::*;

fn lines(c: &Calc, rest: Vec<String>) -> Vec<String> {
    let mut v = data_lines(c);
    v.extend(rest);
    v
}

/// The data (6 lines), then the function: its line is the 7th.
const FN_LINE: usize = 7;

fn with_tests(c: &Calc, f: String, tests: Vec<String>) -> Vec<String> {
    let mut v = vec![f];
    v.extend(tests);
    lines(c, v)
}

// ---- accepted ---------------------------------------------------------------------------------

/// Every derived case and both arms: accepted; and a second run changes nothing.
#[test]
fn full_tests_are_accepted() {
    let c = calc();
    let (lite, r) = checked("ptr-full", &with_tests(&c, per_unit_line(&c, None), per_unit_tests(&c)));
    let p = r.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(p.functions, 3, "{p:?}");
    assert!(lite.bound_tx(&c.per_unit).is_some());
    let before = snapshot(&lite);
    let again = run_checked(&lite, &lite.path("f.jsonl"), T1).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((again.functions, again.versions), (0, 0), "{again:?}");
    assert_eq!(snapshot(&lite), before, "a second run wrote something");
}

#[test]
fn each_kind_of_function_with_its_tests_is_accepted() {
    let c = calc();
    // a slice input and a number argument: positive, negative, the empty slice
    let (_, r) = checked("ptr-avg", &with_tests(&c, avg_log_line(&c), avg_log_tests(&c)));
    r.unwrap_or_else(|e| panic!("avg_log: {e}"));
    // a date argument with a declared range: both ends
    let (_, r) = checked("ptr-quarter", &with_tests(&c, quarter_line(&c), quarter_tests(&c)));
    r.unwrap_or_else(|e| panic!("quarter: {e}"));
    // three arms over two ifs, all taken, one negative
    let (_, r) = checked("ptr-band", &with_tests(&c, band_line(&c), band_tests(&c)));
    r.unwrap_or_else(|e| panic!("band: {e}"));
}

/// Stored functions are exempt: their proof is the source.
#[test]
fn stored_functions_are_exempt() {
    let c = calc();
    let (lite, r) = checked("ptr-stored", &data_lines(&c));
    r.unwrap_or_else(|e| panic!("{e}"));
    assert!(lite.bound_tx(&c.reading).is_some());
}

/// The local, lax publish never requires tests and never refuses on them: a formula alone is
/// accepted, and so is one whose test fails (the failure is recorded: see tests_as_cells.rs).
#[test]
fn the_lax_publish_keeps_tests_optional() {
    let c = calc();
    let lite = Lite::new("ptr-lax");
    let f = write(&lite, "f.jsonl", &lines(&c, vec![per_unit_line(&c, None)]));
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    let lite = Lite::new("ptr-lax-fail");
    let mut t = per_unit_tests(&c);
    t[0] = test_line(&c.per_unit, &pu_call(&c, "N", "2"), None, None, num("6"));
    let f = write(&lite, "f.jsonl", &with_tests(&c, per_unit_line(&c, None), t));
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
}

// ---- E_TESTS_MISSING --------------------------------------------------------------------------

/// No tests: every derived case and the branches are listed.
#[test]
fn a_computed_function_without_tests_is_refused_with_the_list() {
    let c = calc();
    let e = refused(
        "ptr-none",
        &lines(&c, vec![per_unit_line(&c, None)]),
        FN_LINE,
        "E_TESTS_MISSING",
        &[
            c.per_unit.as_str(),
            "zero(units)",
            "negative(units)",
            &format!("absent({})", c.reading),
            "unexercised branches",
        ],
    );
    // the divisor case is for the divisor, the negative case for the number argument: nothing about `site`
    assert!(!e.detail.contains("negative(site)") && !e.detail.contains("zero(site)"), "{e}");
}

/// Tests that leave one derived case out: only that case is listed.
#[test]
fn a_missing_absent_case_is_listed_alone() {
    let c = calc();
    let mut t = per_unit_tests(&c);
    t.remove(3); // the absent input
    let e = refused(
        "ptr-no-absent",
        &with_tests(&c, per_unit_line(&c, None), t),
        FN_LINE,
        "E_TESTS_MISSING",
        &[&format!("absent({})", c.reading)],
    );
    assert!(!e.detail.contains("zero(units)") && !e.detail.contains("negative(units)"), "{e}");
    assert!(!e.detail.contains("unexercised branches"), "{e}");
}

#[test]
fn a_missing_zero_divisor_and_negative_are_each_listed() {
    let c = calc();
    // only the then arm, the absent input and the supplied inputs: no zero, no negative, one arm
    let t = per_unit_tests(&c);
    let only = vec![t[0].clone(), t[3].clone(), t[4].clone()];
    let e = refused(
        "ptr-no-zero",
        &with_tests(&c, per_unit_line(&c, None), only),
        FN_LINE,
        "E_TESTS_MISSING",
        &["zero(units)", "negative(units)", "unexercised branches"],
    );
    assert!(!e.detail.contains(&format!("absent({})", c.reading)), "{e}");
    // zero, negative and absent but no then arm: the zero case and the negative one are covered,
    // and the branch listed is the one not taken
    let only = vec![t[1].clone(), t[2].clone(), t[3].clone()];
    let e = refused(
        "ptr-no-then",
        &with_tests(&c, per_unit_line(&c, None), only),
        FN_LINE,
        "E_TESTS_MISSING",
        &["unexercised branches"],
    );
    assert!(!e.detail.contains("zero(units)") && !e.detail.contains("negative(units)"), "{e}");
}

/// Every derived case covered, one arm never taken: refused for the branch alone.
#[test]
fn an_unexercised_branch_is_refused_alone() {
    let c = calc();
    let t = band_tests(&c);
    let only = vec![t[0].clone(), t[1].clone()]; // -1 and 5: the arm `x > 10` is never taken
    let e = refused(
        "ptr-branch",
        &with_tests(&c, band_line(&c), only),
        FN_LINE,
        "E_TESTS_MISSING",
        &["unexercised branches"],
    );
    assert!(
        !e.detail.contains("negative(x)") && !e.detail.contains("uncovered cases"),
        "the negative case is covered: {e}"
    );
    // and with no negative test but every arm: the negative case alone
    let only = vec![t[1].clone(), t[2].clone(), test_line(&c.band, &format!("{}(x: 0)", c.band), None, None, num("0"))];
    let e = refused("ptr-negative", &with_tests(&c, band_line(&c), only), FN_LINE, "E_TESTS_MISSING", &["negative(x)"]);
    assert!(!e.detail.contains("unexercised branches"), "{e}");
}

/// A date with a declared range needs both ends; a slice input needs the empty slice.
#[test]
fn extents_and_empty_slices_are_derived_cases() {
    let c = calc();
    let q = quarter_tests(&c);
    refused(
        "ptr-lo",
        &with_tests(&c, quarter_line(&c), vec![q[0].clone()]),
        FN_LINE,
        "E_TESTS_MISSING",
        &["extent_hi(d)"],
    );
    refused(
        "ptr-hi",
        &with_tests(&c, quarter_line(&c), vec![q[1].clone()]),
        FN_LINE,
        "E_TESTS_MISSING",
        &["extent_lo(d)"],
    );
    let a = avg_log_tests(&c);
    let e = refused(
        "ptr-empty",
        &with_tests(&c, avg_log_line(&c), vec![a[0].clone(), a[1].clone()]),
        FN_LINE,
        "E_TESTS_MISSING",
        &[&format!("empty({})", c.log)],
    );
    assert!(!e.detail.contains("negative(scale)"), "{e}");
    // no negative scale: that case alone
    refused(
        "ptr-avg-neg",
        &with_tests(&c, avg_log_line(&c), vec![a[0].clone(), a[2].clone()]),
        FN_LINE,
        "E_TESTS_MISSING",
        &["negative(scale)"],
    );
}

/// A test that names a function the mesh does not know is refused at its line.
#[test]
fn a_test_for_an_unknown_function_is_refused() {
    let c = calc();
    let ghost = format!("{PFX}/ghost");
    let mut v = with_tests(&c, per_unit_line(&c, None), per_unit_tests(&c));
    v.push(test_line(&ghost, &format!("{ghost}(site: \"N\", units: 1)"), None, None, num("1")));
    let n = v.len();
    let lite = Lite::new("ptr-ghost");
    let f = write(&lite, "f.jsonl", &v);
    let before = snapshot(&lite);
    let e = run_checked(&lite, &f, NOW).unwrap_err();
    assert_eq!(e.line, n, "{e}");
    assert_eq!(snapshot(&lite), before);
}

// ---- E_TESTS_FAILING --------------------------------------------------------------------------

/// A wrong expectation is refused, with the case id and the numbers; a wrong fault likewise.
#[test]
fn a_failing_test_is_refused_with_its_case() {
    let c = calc();
    let mut t = per_unit_tests(&c);
    let wrong = test_line(&c.per_unit, &pu_call(&c, "N", "2"), None, None, num("6"));
    t[0] = wrong.clone();
    let e = refused(
        "ptr-fail",
        &with_tests(&c, per_unit_line(&c, None), t),
        FN_LINE + 1,
        "E_TESTS_FAILING",
        &[&case_id(&wrong), "5", "6"],
    );
    assert!(!e.detail.contains("E_TESTS_MISSING"), "{e}");

    // the fault expected is not the fault raised; and a value expected where a fault comes out
    let mut t = per_unit_tests(&c);
    let wrong = test_line(&c.per_unit, &pu_call(&c, "ZZ", "2"), None, None, fault("E_FORMULA_EMPTY"));
    t[3] = wrong.clone();
    refused(
        "ptr-fail-fault",
        &with_tests(&c, per_unit_line(&c, None), t),
        FN_LINE + 4,
        "E_TESTS_FAILING",
        &[&case_id(&wrong), "E_FORMULA_ABSENT"],
    );
    let mut t = per_unit_tests(&c);
    let wrong = test_line(&c.per_unit, &pu_call(&c, "ZZ", "2"), None, None, num("0"));
    t[3] = wrong.clone();
    refused(
        "ptr-fail-value",
        &with_tests(&c, per_unit_line(&c, None), t),
        FN_LINE + 4,
        "E_TESTS_FAILING",
        &[&case_id(&wrong)],
    );
}

/// A test with supplied inputs runs in a scope: its failure is a failure too, and the inputs
/// count (the supplied reading, not the stored one).
#[test]
fn tests_with_inputs_run_in_a_scope() {
    let c = calc();
    let mut t = per_unit_tests(&c);
    // the supplied reading is 40: 40 / 2 = 20; expecting the stored reading's 5 fails
    let wrong = test_line(
        &c.per_unit,
        &pu_call(&c, "N", "2"),
        Some(serde_json::json!({ format!("{}(site: \"N\")", c.reading): num("40") })),
        None,
        num("5"),
    );
    t[4] = wrong.clone();
    refused(
        "ptr-fail-inputs",
        &with_tests(&c, per_unit_line(&c, None), t),
        FN_LINE + 5,
        "E_TESTS_FAILING",
        &[&case_id(&wrong)],
    );
}
