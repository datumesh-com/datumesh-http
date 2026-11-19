//! Build 2 Part 3, E5: the tests in `bundle 1`. A bundle may carry a `tests` member (an array of
//! the test objects of `test` lines, `{"test", "call", "asof"?, "inputs"?, "expect"}`, sorted by
//! `test` then by case id), built by `bundle::build_with_tests(namespace, descriptors, names,
//! receipts, catalog, tests, seed)` (`build` and `build_with_catalog` are unchanged). The manifest
//! signature covers it, like `catalog`: a changed, added, removed or dropped test is refused
//! (`BundleError::Manifest`, `E_SIGNATURE`) and nothing is written.
//!
//! `import_signed` is a write path of datumesh-http, so it enforces the public rule: a bundle that
//! binds a computed function without covering tests is `E_TESTS_MISSING`, one with a failing test
//! `E_TESTS_FAILING`: `BundleError::Tests(String)` whose text names the code and lists as
//! `public_test_rule.rs` says. A bundle of stored functions only is as before.

mod common;
#[path = "common/calculus.rs"]
mod calculus;

use calculus::*;
use common::lite::*;
use common::seed_for;
use datumesh_core::Resolver;
use common::pub031::{run, write};
use common::readpath::*;
use datumesh_core::json::{self, Value};
use datumesh_core::FunctionDescriptor;
use datumesh_http::bundle::{self, BundleError};
use datumesh_store::Store;
use serde_json::json;

const BNS: &str = "mesh-test.datumesh.com/scratch_b3b";

fn c() -> Calc {
    Calc::new(BNS)
}

/// The tests of `per_unit` that need no data (supplied inputs, an absent site): the bundle has no receipts.
fn tests(c: &Calc) -> Vec<String> {
    let f = &c.per_unit;
    let reading = |v: &str| Some(json!({ format!("{}(site: \"N\")", c.reading): num(v) }));
    vec![
        test_line(f, &pu_call(c, "N", "2"), reading("10"), None, num("5")),
        test_line(f, &pu_call(c, "N", "0"), reading("10"), None, num("0")),
        test_line(f, &pu_call(c, "N", "-3"), reading("10"), None, num("0")),
        test_line(f, &pu_call(c, "ZZ", "2"), None, None, fault("E_FORMULA_ABSENT")),
    ]
}

/// The descriptors and names of `reading` (stored) and `per_unit` (a formula), taken from a store
/// the fixture was published on, as a publisher's tooling would.
fn parts(c: &Calc) -> (Vec<Vec<u8>>, Vec<(String, String)>) {
    let lite = Lite::new("bt-src");
    let f = write(&lite, "f.jsonl", &[
        common::pub031::function_line(&c.reading, &reading_sig(), None),
        per_unit_line(c, None),
    ]);
    run(&lite, &f, NOW).unwrap();
    let mut descriptors = Vec::new();
    let mut names = Vec::new();
    for n in [&c.reading, &c.per_unit] {
        let h = lite.store.resolve_binding(n, END).unwrap().unwrap().fn_hash;
        let d = lite.store.descriptor(&h).unwrap().into_owned();
        descriptors.push(FunctionDescriptor::from_json(&d).unwrap().canonical());
        names.push((n.to_string(), h));
    }
    (descriptors, names)
}

fn make(c: &Calc, tests: Vec<String>) -> Vec<u8> {
    let (descriptors, names) = parts(c);
    bundle::build_with_tests(
        BNS,
        descriptors,
        names,
        vec![],
        vec![],
        tests.into_iter().map(String::into_bytes).collect(),
        &seed_for(HOST),
    )
    .unwrap()
}

fn import(lite: &Lite, bytes: &[u8]) -> Result<bundle::Imported, BundleError> {
    let owned: Vec<String> = Vec::new();
    bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(NOW), bytes)
}

fn nothing_written(lite: &Lite, c: &Calc) {
    assert!(lite.store.resolve_name(&c.per_unit, END).unwrap().is_none(), "{} is bound", c.per_unit);
    assert!(lite.store.resolve_name(&c.reading, END).unwrap().is_none(), "{} is bound", c.reading);
}

fn edit_tests(bytes: &[u8], f: impl FnOnce(&mut Vec<Value>)) -> Vec<u8> {
    let mut f = Some(f);
    let mut v = json::parse(bytes).unwrap();
    if let Value::Object(m) = &mut v {
        for (k, x) in m.iter_mut() {
            if k == "tests" {
                if let Value::Array(a) = x {
                    if let Some(f) = f.take() {
                        f(a);
                    }
                }
            }
        }
    }
    v.to_jcs()
}

#[test]
fn the_tests_are_a_member_of_the_bundle() {
    let c = c();
    let v = json::parse(&make(&c, tests(&c))).unwrap();
    let keys: Vec<&str> = v.as_object().unwrap().iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["bundle", "descriptors", "manifest", "names", "namespace", "receipts", "tests"]);
    let Some(Value::Array(a)) = v.get("tests") else { panic!("tests is not an array") };
    assert_eq!(a.len(), 4);
    // sorted by test name, then by case id (first 16 hex digits of the SHA-256 of the test's JCS)
    let ids: Vec<String> = a.iter().map(|t| datumesh_core::sha256_hex(&t.to_jcs())[..16].to_string()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
    // `build` is unchanged: no tests member
    let (d, n) = parts(&c);
    let old = bundle::build(BNS, d, n, vec![], &seed_for(HOST)).unwrap();
    assert!(json::parse(&old).unwrap().get("tests").is_none());
}

/// Full tests, signed: the bundle imports, the function is bound, and the tests are held (describe
/// counts them).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bundle_with_full_tests_is_accepted_and_served() {
    let c = c();
    let lite = Lite::new("bt-ok");
    import(&lite, &make(&c, tests(&c))).unwrap_or_else(|e| panic!("{e}"));
    assert!(lite.store.resolve_name(&c.per_unit, END).unwrap().is_some());
    let again = import(&lite, &make(&c, tests(&c))).unwrap();
    assert_eq!((again.functions_bound, again.versions), (0, 0));
    let m = build_served(lite, None, |_| {}).await;
    let d = m.get(&format!("/describe/{}", common::world::call_path(&c.per_unit))).await;
    assert_eq!(d["kind"], "function", "{d}");
    assert_eq!(d["tests"]["count"], 4, "{d}");
    assert_eq!((d["tests"]["branches"].as_u64(), d["tests"]["covered"].as_u64()), (Some(2), Some(2)), "{d}");
    assert_eq!(d["tests"]["counterexamples"], 0);
}

/// Without tests, with tests that do not cover, with failing ones: refused, nothing written.
#[test]
fn a_bundle_that_breaks_the_rule_is_refused() {
    let c = c();
    let t = tests(&c);
    // none
    let lite = Lite::new("bt-none");
    let e = import(&lite, &make(&c, vec![])).unwrap_err();
    assert!(matches!(e, BundleError::Tests(_)), "{e:?}");
    let text = e.to_string();
    for want in ["E_TESTS_MISSING", c.per_unit.as_str(), "zero(units)", "negative(units)", "unexercised branches"] {
        assert!(text.contains(want), "{want}: {text}");
    }
    nothing_written(&lite, &c);
    // the absent case left out
    let lite = Lite::new("bt-partial");
    let e = import(&lite, &make(&c, t[..3].to_vec())).unwrap_err();
    assert!(matches!(e, BundleError::Tests(_)) && e.to_string().contains(&format!("absent({})", c.reading)), "{e}");
    nothing_written(&lite, &c);
    // a wrong expectation
    let lite = Lite::new("bt-failing");
    let wrong = test_line(&c.per_unit, &pu_call(&c, "ZZ", "2"), None, None, num("1"));
    let mut bad = t.clone();
    bad[3] = wrong.clone();
    let e = import(&lite, &make(&c, bad)).unwrap_err();
    assert!(matches!(e, BundleError::Tests(_)), "{e:?}");
    assert!(e.to_string().contains("E_TESTS_FAILING") && e.to_string().contains(&case_id(&wrong)), "{e}");
    nothing_written(&lite, &c);
}

type Tamper = Box<dyn Fn(&[u8]) -> Vec<u8>>;

/// The manifest signature covers the tests.
#[test]
fn a_tampered_test_is_refused() {
    let c = c();
    let good = make(&c, tests(&c));
    let tampers: Vec<(&str, Tamper)> = vec![
        (
            "a test changed (an asof added)",
            Box::new(|b| {
                edit_tests(b, |a| {
                    if let Value::Object(m) = &mut a[0] {
                        m.push(("asof".to_string(), Value::String(NOW.to_string())));
                        m.sort_by(|x, y| x.0.cmp(&y.0));
                    }
                })
            }),
        ),
        ("a test removed", Box::new(|b| edit_tests(b, |a| { a.pop(); }))),
        ("all tests removed", Box::new(|b| edit_tests(b, |a| a.clear()))),
        (
            "a test added",
            Box::new(move |b| {
                let extra = test_line(&calc().per_unit, "x", None, None, num("1"));
                edit_tests(b, |a| a.push(json::parse(extra.as_bytes()).unwrap()))
            }),
        ),
    ];
    for (what, f) in tampers {
        let lite = Lite::new("bt-tamper");
        let e = import(&lite, &f(&good)).unwrap_err();
        assert!(matches!(e, BundleError::Manifest(_)), "{what}: {e:?}");
        assert!(e.to_string().to_lowercase().contains("signature"), "{what}: {e}");
        nothing_written(&lite, &c);
    }
    // the member dropped from a bundle that was signed with it
    let lite = Lite::new("bt-dropped");
    let mut v = json::parse(&good).unwrap();
    if let Value::Object(m) = &mut v {
        m.retain(|(k, _)| k != "tests");
    }
    let e = import(&lite, &v.to_jcs()).unwrap_err();
    assert!(matches!(e, BundleError::Manifest(_)), "{e:?}");
    nothing_written(&lite, &c);
}

/// A bundle of stored functions only needs no tests (and a stored function's tests are not asked).
#[test]
fn a_stored_bundle_is_exempt() {
    let c = c();
    let (d, n) = parts(&c);
    let (d, n) = (d[..1].to_vec(), n[..1].to_vec());
    let b = bundle::build_with_tests(BNS, d, n, vec![], vec![], vec![], &seed_for(HOST)).unwrap();
    let lite = Lite::new("bt-stored");
    import(&lite, &b).unwrap_or_else(|e| panic!("{e}"));
}
