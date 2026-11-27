//! The guard patch, third review (27 Nov): the `latest` cache (mesh.rs `answer_target`) answers a
//! local stored cell before the engine, without the barrier, and `cache_rules` marks the answer
//! Immutable for an asof at or before now. In the gap between a write's commit and `version_added`,
//! a request naming an asof at or after the new version's recorded time could get the old version,
//! cached for a year. The fix chosen: the latest cache serves only requests that name no asof. Here a
//! new version is written through the store directly, bypassing `version_added`: a request naming an
//! asof after it must not get the old version as Immutable.

mod common;

use common::*;
use datumesh_core::{Cell, FunctionDescriptor, ReceiptBuilder};

const A: &str = "mesh-a.example";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

fn value_of(r: &Resp) -> Option<String> {
    use datumesh_core::json::Value;
    r.json().get("receipt").and_then(|x| x.get("value")).and_then(Value::as_str).map(str::to_string)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_naming_an_asof_never_gets_an_immutable_answer_from_the_latest_cache() {
    let n = Node::start(A, |c| c.rate.enabled = false).await;
    let rate = format!("{A}/rate");
    n.stored(&rate, RATE_SIG);
    let usd = args(&[("ccy", s("USD")), ("date", d("2026-03-01"))]);
    n.put_value(&rate, &usd, common::n("1.08"), T1);
    let path = format!("/cell/{rate}?ccy=USD&date=2026-03-01");
    // A plain read fills the latest cache with the first version.
    let r = n.get(&path).await;
    assert_eq!(value_of(&r).as_deref(), Some("1.08"), "{}", r.text());
    // A second version written through the store alone: committed, version_added not yet called.
    bl(|| {
        let fn_hash = n.store.resolve_name(&rate, T2).unwrap().unwrap();
        let cell = Cell { ns: A.into(), fn_name: "rate".into(), fn_hash: fn_hash.clone(), args: usd.clone() };
        let desc = FunctionDescriptor::from_json(&n.store.descriptor(&fn_hash).unwrap()).unwrap();
        let valid: datumesh_core::Args = usd
            .iter()
            .filter(|(k, _)| desc.signature.args.get(*k).is_some_and(|x| x.valid))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let receipt = ReceiptBuilder::new()
            .cell(cell.clone())
            .value(common::n("1.09"))
            .valid(valid)
            .asof(T2)
            .tx(T2)
            .issued(T2)
            .publisher(A)
            .signer_host(A)
            .licence("CC-BY-4.0")
            .summary("A test value.")
            .sign(&n.key)
            .unwrap();
        let h = n.store.put_receipt(&receipt_json(&receipt)).unwrap();
        n.store.put_version(&cell.id(), T2, &h).unwrap();
    });
    // A request naming an asof after the new version's recorded time (T2: no recording clock).
    let r = n.get(&format!("{path}&asof=2026-06-01T00:00:00.000000Z")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let value = value_of(&r);
    if value.as_deref() == Some("1.08") {
        assert_ne!(
            r.header("cache-control").as_deref(),
            Some(IMMUTABLE),
            "the old version from the latest cache must not be cached for a year: {}",
            r.text()
        );
    }
    // Either version may answer (the engine's memo learns of the write from version_added), but the
    // old one never as Immutable.
    assert!(matches!(value.as_deref(), Some("1.08") | Some("1.09")), "{}", r.text());
}
