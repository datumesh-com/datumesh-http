//! Every cross-mesh fetch that gets no HTTP response is `E_FOREIGN_UNAVAILABLE` (502) with
//! core 0.3.7's `ForeignDetail`, whichever the path: key and mesh documents, bindings,
//! descriptors, receipts, manifests, blobs, cells, slices, batches, versions. Refused (the
//! other mesh stopped) and DNS (a name that does not resolve). A mesh that answers keeps its
//! own outcome. A cell served before is still `stale`.

mod common;

use std::time::Duration;

use common::*;
use datumesh_core::json::Value;
use datumesh_core::{ErrorCode, ForeignDetail, Request};
use datumesh_http::client::{Client, ClientConfig, ClientError};

const A: &str = "mesh-a.example";
const B: &str = "mesh-b.example";
const SIG: &str =
    r#"{"args":{"item":{"type":"string","valid":false}},"order":["item"],"returns":{"type":"number","unit":"EUR"}}"#;

fn client(host: &str, origin: Option<&str>) -> Client {
    let mut cfg = ClientConfig::default();
    if let Some(o) = origin {
        cfg.peers.insert(host.to_string(), o.to_string());
    }
    cfg.timeout = Duration::from_secs(5);
    Client::new(cfg)
}

fn hex64() -> String {
    "ab".repeat(32)
}

fn unit<T>(r: Result<T, ClientError>) -> Result<(), ClientError> {
    r.map(|_| ())
}

/// Every client path against `host`, as (name, result).
fn paths(c: &Client, host: &str) -> Vec<(&'static str, Result<(), ClientError>)> {
    let req = Request::new(&hex64(), None).unwrap();
    let rest = format!("{host}/price?item=bolt");
    vec![
        ("key_document", unit(c.key_document(host, true))),
        ("resolve", unit(c.resolve(host))),
        ("binding", unit(c.binding(host, &format!("{host}/price"), None))),
        ("descriptor", unit(c.descriptor(host, &hex64()))),
        ("receipt", unit(c.receipt(host, &hex64()))),
        ("manifest", unit(c.manifest(host, &hex64(), &hex64()))),
        ("blob", unit(c.blob(host, &hex64()))),
        ("cell", unit(c.cell(host, &rest, &req))),
        ("slice", unit(c.slice(host, &format!("{host}/price?item=*"), None))),
        ("batch", unit(c.batch(host, std::slice::from_ref(&rest), std::slice::from_ref(&req)))),
        ("versions", unit(c.versions(host, &rest))),
    ]
}

fn all_foreign(results: Vec<(&'static str, Result<(), ClientError>)>, want: ForeignDetail) {
    for (name, r) in results {
        let e = r.expect_err(name);
        assert_eq!(e.code, ErrorCode::ForeignUnavailable, "{name}: {e}");
        assert_eq!(e.foreign_detail(), Some(want), "{name}: {e}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_path_to_a_stopped_mesh_is_foreign_unavailable_refused() {
    let mut b = Node::start(B, |_| {}).await;
    b.stored(&format!("{B}/price"), SIG);
    let base = b.base.clone();
    // Fresh client: the documents themselves cannot be fetched.
    let fresh = client(B, Some(&base));
    // Warm client: documents cached while the mesh was up, so each later path fetches its own.
    let warm = client(B, Some(&base));
    let b_base = base.clone();
    let w = tokio::task::spawn_blocking(move || {
        warm.resolve(B).unwrap();
        warm.key_document(B, false).unwrap();
        let _ = b_base;
        warm
    })
    .await
    .unwrap();
    b.stop().await;
    tokio::task::spawn_blocking(move || {
        let fresh_results = paths(&fresh, B);
        all_foreign(fresh_results, ForeignDetail::Refused);
        // The warm client's cached documents answer key_document and resolve; the rest fetch.
        let rest: Vec<_> =
            paths(&w, B).into_iter().filter(|(n, _)| !matches!(*n, "key_document" | "resolve")).collect();
        assert_eq!(rest.len(), 9);
        all_foreign(rest, ForeignDetail::Refused);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_path_to_a_name_that_does_not_resolve_is_foreign_unavailable_dns() {
    tokio::task::spawn_blocking(|| {
        let c = client("no-such-mesh.invalid", None);
        let results = paths(&c, "no-such-mesh.invalid");
        for (name, r) in &results {
            let e = r.as_ref().expect_err(name);
            assert_eq!(e.code, ErrorCode::ForeignUnavailable, "{name}: {e}");
            assert_eq!(e.foreign_detail(), Some(ForeignDetail::Dns), "{name}: {e}");
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mesh_that_answers_keeps_its_own_outcome() {
    let b = Node::start(B, |_| {}).await;
    let base = b.base.clone();
    tokio::task::spawn_blocking(move || {
        let c = client(B, Some(&base));
        // B answers 404 for a descriptor it lacks: E_FN_UNAVAILABLE, not a reachability failure.
        let e = c.descriptor(B, &hex64()).unwrap_err();
        assert_eq!(e.code, ErrorCode::FnUnavailable, "{e}");
        assert_eq!(e.foreign_detail(), None);
        // And a receipt it lacks: not E_FOREIGN_UNAVAILABLE.
        let e = c.receipt(B, &hex64()).unwrap_err();
        assert_ne!(e.code, ErrorCode::ForeignUnavailable, "{e}");
    })
    .await
    .unwrap();
}

/// Through a mesh: a cell A served before is `stale` once B is stopped; one A never read,
/// and a name A never resolved, are `error` E_FOREIGN_UNAVAILABLE, 502, with the detail.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn through_a_mesh_stale_is_kept_and_the_rest_is_502() {
    let mut b = Node::start(B, |_| {}).await;
    b.stored(&format!("{B}/price"), SIG);
    for (item, v) in [("bolt", "100"), ("nut", "5")] {
        b.put_value(&format!("{B}/price"), &args(&[("item", s(item))]), n(v), "2026-03-01T00:00:00.000000Z");
    }
    let base = b.base.clone();
    let a = Node::start(A, |c| {
        c.peers.insert(B.into(), base.clone());
        c.ttl.current = Duration::from_secs(1);
        c.ttl.binding = Duration::from_secs(1);
    })
    .await;
    let r = a.get(&format!("/cell/{B}/price?item=bolt")).await;
    assert_eq!(r.outcome(), "ok", "{}", r.text());
    b.stop().await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    // Served before: stale.
    let r = a.get(&format!("/cell/{B}/price?item=bolt")).await;
    assert_eq!(r.outcome(), "stale", "{}", r.text());
    // Never read from B: 502 with the detail.
    let r = a.get(&format!("/cell/{B}/price?item=nut")).await;
    assert_eq!(r.status, 502, "{}", r.text());
    assert_eq!(r.json().get("error").and_then(Value::as_str), Some("E_FOREIGN_UNAVAILABLE"));
    assert_eq!(r.json().get("detail").and_then(Value::as_str), Some("refused"), "{}", r.text());
    // A name under a host never resolved, unreachable by DNS or refused: the binding path.
    let r = a.get("/cell/no-such-mesh.invalid/price?item=bolt").await;
    assert_eq!(r.status, 502, "{}", r.text());
    assert_eq!(r.json().get("error").and_then(Value::as_str), Some("E_FOREIGN_UNAVAILABLE"));
    assert!(r.json().get("detail").is_some());
}
