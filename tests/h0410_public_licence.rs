//! http 0.4.10 (spec-0.6.2 in the mesh; CONTRACT-spec-0.6.2 P4, D47; SPEC 8.1, 8.5, 10.3): the
//! `public:<https URL>` licence class through the mesh.
//!
//! * A stored cell with a `public:` licence publishes, and its receipt verifies; malformed `public:`
//!   forms are refused (`E_CANON_LICENCE`).
//! * A formula over `public:` inputs computes with NO declaration; its receipt licence is the public
//!   one, or the byte-greatest of several different ones; with CC-BY-4.0 it is CC-BY-4.0; with CC0 it
//!   is the public one.
//! * The entitlement check (core) answers `open` for the computed receipt.

#[path = "common/calculus.rs"]
mod calculus;
mod common;

use calculus::*;
use common::lite::*;
use common::pub031::*;
use common::readpath::*;
use common::world::call_path;
use datumesh_core::{check_entitlement, KeyDocument, LicenceReason, Receipt};
use serde_json::{json, Value};

const SEC: &str = "public:https://www.sec.gov/about/privacy-information";
const TREASURY: &str = "public:https://www.treasury.gov/privacy-act/";
const P: &str = "mesh-test.datumesh.com/scratch_h0410p";

fn f(n: &str) -> String {
    format!("{P}/{n}")
}

fn licensed_cell(function: &str, licence: &str) -> String {
    let mut v: Value = serde_json::from_str(&site_cell(function, "N", "10")).unwrap();
    v["licence"] = json!(licence);
    v.to_string()
}

fn sum_sig() -> String {
    reading_sig()
}

/// Stored functions `sec_a`, `sec_b` (SEC), `tre` (Treasury), `by` (CC-BY-4.0), `zero` (CC0-1.0), and
/// formulas over pairs of them.
fn lines() -> Vec<String> {
    let mut v = Vec::new();
    for (n, lic) in [("sec_a", SEC), ("sec_b", SEC), ("tre", TREASURY), ("by", "CC-BY-4.0"), ("zero", "CC0-1.0")] {
        v.push(function_line(&f(n), &reading_sig(), None));
        v.push(licensed_cell(&f(n), lic));
    }
    for (n, a, b) in [
        ("sec_sec", "sec_a", "sec_b"),
        ("sec_tre", "sec_a", "tre"),
        ("sec_by", "sec_a", "by"),
        ("sec_zero", "sec_a", "zero"),
    ] {
        let body = format!("{}(site: site) + {}(site: site)", f(a), f(b));
        v.push(formula_line(&f(n), &body, &sum_sig(), None));
    }
    v
}

async fn cell(m: &Served, name: &str) -> Value {
    let v = m.get(&format!("/cell/{}", call_path(&format!("{name}(site: \"N\")")))).await;
    assert_eq!(v["outcome"], "ok", "{name}: {v}");
    v
}

fn verify_receipt(v: &Value) {
    let mut keys = datumesh_core::Keys::new();
    keys.insert_response(
        HOST,
        200,
        Some(&datumesh_http::keydoc::with_key(HOST, None, &common::key_for(HOST), K_FROM).unwrap()),
    );
    let r = datumesh_core::verify(&serde_json::to_vec(&v["receipt"]).unwrap(), &keys, None, None);
    assert!(r.ok, "{:?}", r.error());
}

/// core's entitlement check of a receipt this mesh served (key document and input receipts as served).
async fn entitlement(m: &Served, receipt: &Value) -> datumesh_core::Entitlement {
    let bytes = serde_json::to_vec(receipt).unwrap();
    let r = Receipt::from_json(&bytes).unwrap();
    let (status, doc) = m.raw("/.well-known/datumesh.json").await;
    assert_eq!(status, 200);
    let k = KeyDocument::from_json(HOST, &doc).unwrap();
    let mut licences = Vec::new();
    for h in r.inputs() {
        assert!(!h.starts_with("slice:"));
        let (s, body) = m.raw(&format!("/receipt/{h}")).await;
        assert_eq!(s, 200);
        licences.push(Receipt::from_json(&body).unwrap().licence().to_string());
    }
    check_entitlement(&r, licences, &k).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stored_public_cells_publish_and_their_receipts_verify() {
    let m = serve_lite("h0410-pub", &lines(), None, |_| {}).await;
    for n in ["sec_a", "sec_b"] {
        let v = cell(&m, &f(n)).await;
        assert_eq!(v["receipt"]["licence"], SEC, "{v}");
        verify_receipt(&v);
    }
    let v = cell(&m, &f("tre")).await;
    assert_eq!(v["receipt"]["licence"], TREASURY, "{v}");
    verify_receipt(&v);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_public_licences_are_refused() {
    for bad in [
        "public:",
        "public:http://www.sec.gov/about/privacy-information",
        "public:CC0-1.0",
        "public:not a url",
        "public:https://",
    ] {
        let lite = Lite::new("h0410-bad");
        let ls = vec![function_line(&f("x"), &reading_sig(), None), licensed_cell(&f("x"), bad)];
        let file = write(&lite, "f.jsonl", &ls);
        let e = run(&lite, &file, NOW).expect_err(&format!("{bad} must be refused"));
        assert!(e.to_string().contains("E_CANON_LICENCE"), "{bad}: {e}");
    }
    // the whole string, prefix included, is at most 512 characters
    let long = format!("public:https://www.sec.gov/{}", "a".repeat(512 - "public:https://www.sec.gov/".len() + 1));
    assert_eq!(long.len(), 513);
    let lite = Lite::new("h0410-long");
    let file = write(&lite, "f.jsonl", &[function_line(&f("x"), &reading_sig(), None), licensed_cell(&f("x"), &long)]);
    let e = run(&lite, &file, NOW).expect_err("513 characters");
    assert!(e.to_string().contains("E_CANON_LICENCE") || e.to_string().contains("E_CANON_LIMIT"), "{e}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_formula_over_public_inputs_computes_without_a_declaration() {
    let m = serve_lite("h0410-formula", &lines(), None, |_| {}).await;
    // one public licence: that licence; no declaration is held, the mesh still computes
    let v = cell(&m, &f("sec_sec")).await;
    assert_eq!(v["receipt"]["value"], "20", "{v}");
    assert_eq!(v["receipt"]["licence"], SEC, "{v}");
    verify_receipt(&v);
    let e = entitlement(&m, &v["receipt"]).await;
    assert!(e.entitled && e.reason == LicenceReason::Open, "{e:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_public_licences_give_the_byte_greatest() {
    let m = serve_lite("h0410-two", &lines(), None, |_| {}).await;
    let v = cell(&m, &f("sec_tre")).await;
    let want = SEC.max(TREASURY);
    assert_eq!(want, TREASURY);
    assert_eq!(v["receipt"]["licence"], want, "{v}");
    verify_receipt(&v);
    let e = entitlement(&m, &v["receipt"]).await;
    assert!(e.entitled && e.reason == LicenceReason::Open, "{e:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_with_attribution_gives_attribution_and_with_cc0_gives_public() {
    let m = serve_lite("h0410-mixed", &lines(), None, |_| {}).await;
    let v = cell(&m, &f("sec_by")).await;
    assert_eq!(v["receipt"]["licence"], "CC-BY-4.0", "{v}");
    verify_receipt(&v);
    let e = entitlement(&m, &v["receipt"]).await;
    assert!(e.entitled && e.reason == LicenceReason::Open, "{e:?}");

    let v = cell(&m, &f("sec_zero")).await;
    assert_eq!(v["receipt"]["licence"], SEC, "{v}");
    let e = entitlement(&m, &v["receipt"]).await;
    assert!(e.entitled && e.reason == LicenceReason::Open, "{e:?}");
}
