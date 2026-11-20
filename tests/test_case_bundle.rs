//! http 0.4.2 (D20, spec-0.6.1 7.3; CONTRACT-build2-spec-0.6.1 S2, S5 item 4): how a host without
//! our mesh publishes its `test_case` cells. A bundle whose namespace is the bare host is accepted
//! when every name in it is `<host>/test_case`; a bundle of a bare host with any other name is
//! refused with the code a bare host always got, `E_BUNDLE_FORMAT` (400). The domain proof and the
//! signature are the same as for any bundle.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use common::lite::END;
use common::pub031::stored_descriptor;
use common::{args, key_for, receipt_json, s, seed_for};
use datumesh_core::{Cell, FunctionDescriptor, ReceiptBuilder, Scalar, Type};
use datumesh_http::bundle;
use datumesh_store::Store;
use publish4::*;
use serde_json::Value;

const FH: &str = "ab";
const CASE: &str = "00000000000000aa";

fn test_case_signature() -> String {
    r#"{"args":{"case":{"type":"string","valid":false},"function_hash":{"type":"string","valid":false}},"order":["function_hash","case"],"returns":"boolean"}"#
        .to_string()
}

/// A bundle of the host `PUB` (`ns`) whose names are `names`, all bound to the `test_case`
/// descriptor, with one signed `false` test cell of `FH`.
fn bundle_with(ns: &str, names: &[&str]) -> Vec<u8> {
    let d = FunctionDescriptor::from_json(stored_descriptor(&test_case_signature()).as_bytes()).unwrap();
    let cell = Cell {
        ns: PUB.to_string(),
        fn_name: "test_case".into(),
        fn_hash: d.hash(),
        args: args(&[("case", s(CASE)), ("function_hash", s(&FH.repeat(32)))]),
    };
    let r = ReceiptBuilder::new()
        .cell(cell)
        .value(Scalar::from_lexical(Type::Boolean, "false").unwrap())
        .valid(args(&[]))
        .asof(TX)
        .tx(TX)
        .issued(TX)
        .publisher(PUB)
        .signer_host(PUB)
        .licence("CC0-1.0")
        .summary("A test of someone else's function.")
        .sign(&key_for(PUB))
        .unwrap();
    let names: Vec<(String, String)> = names.iter().map(|n| (n.to_string(), d.hash())).collect();
    bundle::build_with_catalog(ns, vec![d.canonical()], names, vec![receipt_json(&r)], Vec::new(), &seed_for(PUB))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bare_host_bundle_of_test_cases_imports() {
    let stub = Stub::start(PUB).await;
    let p = public("tcb-ok", &[stub.peer()], Some(PUB)).await;
    let r = p.post(bundle_with(PUB, &[&format!("{PUB}/test_case")])).await;
    assert_eq!(r.status, 201, "{}", r.text());
    let j = r.json();
    assert_eq!(j["names"], serde_json::json!([format!("{PUB}/test_case")]), "{j}");
    // the cell is held: served by URI, as the host's signed false statement
    let uri = format!("/cell/{PUB}/test_case?case={CASE}&function_hash={}", FH.repeat(32));
    let c = p.get(&uri, &[]).await;
    assert_eq!(c.status, 200, "{}", c.text());
    let c: Value = c.json();
    assert_eq!(c["outcome"], "ok", "{c}");
    assert_eq!(c["receipt"]["value"], Value::Bool(false), "{c}");
    assert_eq!(c["receipt"]["cell"]["ns"], PUB, "{c}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bare_host_bundle_with_another_name_is_refused_as_before() {
    let stub = Stub::start(PUB).await;
    let p = public("tcb-no", &[stub.peer()], Some(PUB)).await;
    // another name under the bare host
    let other = format!("{PUB}/level");
    let r = p.post(bundle_with(PUB, &[&format!("{PUB}/test_case"), &other])).await;
    p.refused(&r, "E_BUNDLE_FORMAT");
    // a test_case under a namespace of the host is not the host's test case
    let nested = format!("{PUB}/x/test_case");
    let r = p.post(bundle_with(PUB, &[&nested])).await;
    p.refused(&r, "E_BUNDLE_FORMAT");
    // nothing was written for either
    assert!(p.m.lite.store.resolve_name(&format!("{PUB}/test_case"), END).unwrap().is_none());
}
