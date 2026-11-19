//! Build 2 Part 4, P1: `POST /bundles` with the tests of Part 3 (E5) in the bundle
//! (`bundle::build_with_tests`, which Part 3 adds): a computed function with full tests is accepted
//! and its function page is served; a failing test is E_TESTS_FAILING, with the list; nothing is
//! written on a refusal. See `post_bundles.rs` for the rest of the endpoint.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use calculus::*;
use common::lite::*;
use common::seed_for;
use datumesh_http::bundle;
use datumesh_store::Store;
use publish4::*;

fn make(host: &str, tests: Vec<String>) -> Vec<u8> {
    let c = computed_calc(host);
    let (d, nm) = computed_parts(&c, host);
    bundle::build_with_tests(
        &format!("{host}/b4c"),
        d,
        nm,
        vec![],
        vec![],
        tests.into_iter().map(String::into_bytes).collect(),
        &seed_for(host),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_computed_function_with_full_tests_is_accepted() {
    let stub = Stub::start(PUB).await;
    let p = public("pbt-ok", &[stub.peer()], Some(PUB)).await;
    let c = computed_calc(PUB);
    let r = p.post(make(PUB, per_unit_bundle_tests(&c))).await;
    assert_eq!(r.status, 201, "{}", r.text());
    let j = r.json();
    assert_eq!(j["imported"]["functions_bound"], 2, "{j}");
    let fns = j["functions"].as_array().unwrap();
    assert_eq!(fns.len(), 2, "{j}");
    let f = fns.iter().find(|f| f["name"] == c.per_unit.as_str()).unwrap_or_else(|| panic!("{j}"));
    assert_eq!(f["page"], format!("https://{HOST}/fn/{}", c.per_unit));
    // the tests are held: describe counts them, the page shows them
    let d = p.get(&format!("/describe/{}", common::world::call_path(&c.per_unit)), &[]).await.json();
    assert_eq!(d["tests"]["count"], 4, "{d}");
    assert_eq!(d["page"], f["page"], "{d}");
    let page = p.get(&format!("/fn/{}", c.per_unit), &[("accept", "application/json")]).await;
    assert_eq!(page.status, 200, "{}", page.text());
    assert_eq!(page.json()["tests"]["count"], 4);
    assert_eq!(page.json()["tests"]["covered"], page.json()["tests"]["branches"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_test_is_refused_with_its_case() {
    let stub = Stub::start(PUB).await;
    let p = public("pbt-fail", &[stub.peer()], Some(PUB)).await;
    let c = computed_calc(PUB);
    let mut t = per_unit_bundle_tests(&c);
    let wrong = test_line(&c.per_unit, &pu_call(&c, "ZZ", "2"), None, None, num("1"));
    t[3] = wrong.clone();
    let r = p.post(make(PUB, t)).await;
    assert_eq!(r.status, 422, "{}", r.text());
    p.refused(&r, "E_TESTS_FAILING");
    assert!(r.json()["detail"].to_string().contains(&case_id(&wrong)), "{}", r.text());
    for name in [&c.per_unit, &c.reading] {
        assert!(p.m.lite.store.resolve_name(name, END).unwrap().is_none(), "{name} is bound");
    }
}

/// Tests that leave a derived case out: E_TESTS_MISSING, the case listed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tests_that_leave_a_case_out_are_missing() {
    let stub = Stub::start(PUB).await;
    let p = public("pbt-miss", &[stub.peer()], Some(PUB)).await;
    let c = computed_calc(PUB);
    let t = per_unit_bundle_tests(&c)[..3].to_vec(); // the absent-input case is left out
    let r = p.post(make(PUB, t)).await;
    p.refused(&r, "E_TESTS_MISSING");
    assert!(r.json()["detail"].to_string().contains(&format!("absent({})", c.reading)), "{}", r.text());
    assert!(p.m.lite.store.resolve_name(&c.per_unit, END).unwrap().is_none());
}
