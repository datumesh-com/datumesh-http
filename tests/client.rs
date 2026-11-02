//! The store-less client (`datumesh_http::client`): every resource of another mesh fetched and
//! checked with the core, nothing stored. Two meshes, as in the cross-mesh test.

mod common;

use std::time::Duration;

use common::*;
use datumesh_core::{Receipt, Reference, Request, SliceRequest};
use datumesh_http::client::{Client, ClientConfig};

const A: &str = "mesh-a.example";
const B: &str = "mesh-b.example";
const C: &str = "mesh-c.example";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fetch_and_check_without_a_store() {
    let b = Node::start(B, |c| c.rate.enabled = false).await;
    seed(&b, 10);
    let blob = bl(|| b.store.put_blob(b"blob content").unwrap());
    let mut cfg = ClientConfig::default();
    cfg.peers.insert(B.into(), b.base.clone());
    cfg.peers.insert(C.into(), b.base.clone());
    cfg.timeout = Duration::from_secs(5);
    let client = Client::new(cfg);
    let rate = format!("{B}/rate");
    let usd = args(&[("ccy", s("USD")), ("date", d("2026-03-01"))]);
    let id = b.cell_id(&rate, &usd);
    let c = client.clone();
    let (rate2, id2, blob2) = (rate.clone(), id.clone(), blob.clone());
    tokio::task::spawn_blocking(move || {
        let c = &c;
        // The host: its key document and mesh document, resolved together.
        let mesh = c.resolve(B).unwrap();
        assert!(mesh.endpoints.contains_key("versions"));
        assert!(c.key_document(B, false).unwrap().key_ids().len() == 1);
        // A host whose documents name another host.
        assert_eq!(c.resolve(C).unwrap_err().code.as_str(), "E_MESH_DOC");
        // Bindings.
        let bound = c.binding(B, &rate2, None).unwrap().unwrap();
        assert_eq!(bound.since, BIND_TX);
        assert!(c.binding(B, &format!("{B}/nope"), None).unwrap().is_none());
        // A cell by URI rest, by call, and pinned; each checked against its request.
        let f = c.cell(B, "mesh-b.example/rate?ccy=USD&date=2026-03-01", &Request::new(&id2, None).unwrap()).unwrap();
        assert_eq!(f.answer.outcome.as_str(), "ok");
        let call =
            c.cell(B, "mesh-b.example/rate(%22USD%22,%202026-03-01)", &Request::new(&id2, None).unwrap()).unwrap();
        assert_eq!(call.body, f.body);
        let mut r = Reference::new(B, "rate", usd.clone());
        r.fn_hash = Some(bound.fn_hash.clone());
        let pinned = c.reference(B, &r).unwrap();
        assert_eq!(pinned.answer.fn_hash.as_deref(), Some(bound.fn_hash.as_str()));
        // The wrong request: refused by the check, E_ENVELOPE.
        let other = c.cell(B, "mesh-b.example/rate?ccy=GBP&date=2026-03-01", &Request::new(&id2, None).unwrap());
        assert_eq!(other.unwrap_err().code.as_str(), "E_ENVELOPE");
        // Receipts (hash and signature checked), descriptors, blobs, versions.
        let rec = f.answer.receipt.clone().unwrap();
        let bytes = c.receipt(B, rec.hash()).unwrap();
        assert_eq!(Receipt::from_json(&bytes).unwrap().hash(), rec.hash());
        let desc = c.descriptor(B, &bound.fn_hash).unwrap();
        assert_eq!(datumesh_core::FunctionDescriptor::from_json(&desc).unwrap().hash(), bound.fn_hash);
        assert_eq!(c.blob(B, &blob2).unwrap(), b"blob content");
        let vs = c.versions(B, "mesh-b.example/rate?ccy=USD&date=2026-03-01").unwrap();
        assert_eq!(vs.versions.len(), 1);
        assert_eq!(vs.versions[0].receipt, rec.hash());
        // A slice page, checked as 5.6 says.
        let page = c.slice(B, "mesh-b.example/rate?ccy=*&date=2026-03-01", None).unwrap();
        let p = page.answer.page.as_ref().unwrap();
        assert_eq!(p.members.len(), 2);
        let sreq = SliceRequest::new(p.slice.id(), None).unwrap();
        datumesh_core::check_slice_answer(page.status, &page.body, &c.keys_for_body(&page.body), &sreq).unwrap();
        // A batch, item by item.
        let items = [
            "mesh-b.example/rate?ccy=USD&date=2026-03-01".to_string(),
            "mesh-b.example/rate?ccy=CHF&date=2026-03-01".to_string(),
        ];
        let chf = Request::new(
            &datumesh_core::Cell {
                ns: B.into(),
                fn_name: "rate".into(),
                fn_hash: bound.fn_hash.clone(),
                args: args(&[("ccy", s("CHF")), ("date", d("2026-03-01"))]),
            }
            .id(),
            None,
        )
        .unwrap();
        let reqs = [Request::new(&id2, None).unwrap(), chf];
        match c.batch(B, &items, &reqs).unwrap() {
            datumesh_core::Batch::Items(its) => {
                assert_eq!(its.len(), 2);
                assert_eq!(its[0].as_ref().unwrap().outcome.as_str(), "ok");
                assert_eq!(its[1].as_ref().unwrap().outcome.as_str(), "not_covered");
            }
            other => panic!("{other:?}"),
        }
        // A manifest, by its entry.
        let total = c
            .cell(
                B,
                "mesh-b.example/total?date=2026-03-01",
                &Request::new(
                    &datumesh_core::Cell {
                        ns: B.into(),
                        fn_name: "total".into(),
                        fn_hash: c.binding(B, "mesh-b.example/total", None).unwrap().unwrap().fn_hash,
                        args: args(&[("date", d("2026-03-01"))]),
                    }
                    .id(),
                    None,
                )
                .unwrap(),
            )
            .unwrap();
        let entry = total.answer.receipt.unwrap().inputs()[0].clone();
        let parts: Vec<&str> = entry.split(':').collect();
        let m = c.manifest(B, parts[1], parts[2]).unwrap();
        assert_eq!(datumesh_core::Manifest::from_json(&m).unwrap().entry(), entry);
    })
    .await
    .unwrap();
    // The allowlist: a host outside it is never fetched.
    let mut cfg = ClientConfig::default();
    cfg.peers.insert(B.into(), b.base.clone());
    cfg.allowed = Some(vec![A.into()]);
    let fenced = Client::new(cfg);
    let e = tokio::task::spawn_blocking(move || fenced.resolve(B).unwrap_err()).await.unwrap();
    assert_eq!(e.code.as_str(), "E_INPUT_UNAVAILABLE", "{}", e.detail);
    let _ = rate;
    let _ = id;
}
