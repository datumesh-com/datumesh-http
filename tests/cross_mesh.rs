//! Test 4 of the brief: two meshes on two ports with two namespaces and two keys. A function
//! on A reads a cell on B; A's receipt verifies at level recomputed against both key
//! documents with no trust in either mesh (the value recomputed here by the formula crate's
//! evaluator from B's signed receipt); B's key rotation is picked up within the TTL; B down
//! gives `error`, not `not_covered`; a host whose documents disagree is `E_MESH_DOC`.

mod common;

use std::time::{Duration, Instant};

use common::*;
use datumesh_core::json::{self, Value};
use datumesh_core::{verify, Keys, Level, MapResolver, Receipt};
use datumesh_formula::eval::CellKey;
use datumesh_formula::memory::MemoryWorld;

const A: &str = "mesh-a.example";
const B: &str = "mesh-b.example";
const C: &str = "mesh-c.example";
const PRICE_SIG: &str =
    r#"{"args":{"item":{"type":"string","valid":false}},"order":["item"],"returns":{"type":"number","unit":"EUR"}}"#;

async fn fetch(http: &reqwest::Client, url: &str) -> Vec<u8> {
    let r = http.get(url).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200, "{url}");
    r.bytes().await.unwrap().to_vec()
}

/// Verifies A's receipt at level recomputed trusting neither mesh: key documents fetched
/// from each host, the descriptors and B's receipt fetched by hash (the verifier checks the
/// hashes), and the value recomputed here from B's receipt.
async fn verify_without_trust(a: &Node, b_base: &str, receipt: &[u8]) -> Level {
    let http = client();
    let r = Receipt::from_json(receipt).unwrap();
    let mut keys = Keys::new();
    keys.insert_response(A, 200, Some(&fetch(&http, &format!("{}/.well-known/datumesh.json", a.base)).await));
    keys.insert_response(B, 200, Some(&fetch(&http, &format!("{b_base}/.well-known/datumesh.json")).await));
    let mut map = MapResolver::new();
    let a_desc = fetch(&http, &format!("{}/descriptor/{}", a.base, r.cell().fn_hash)).await;
    map.insert_descriptor(r.cell().fn_hash.clone(), a_desc.clone());
    let mut world = MemoryWorld::new(r.asof());
    world.bind(&r.cell().function_name(), &json::parse(&a_desc).unwrap(), BIND_TX).unwrap();
    let bind =
        json::parse(&fetch(&http, &format!("{b_base}/bindings?name={B}%2Fprice&asof={}", r.asof())).await).unwrap();
    let price = bind.get("bindings").and_then(|b| b.get(&format!("{B}/price"))).unwrap();
    let b_hash = price.get("fn_hash").and_then(Value::as_str).unwrap();
    let b_desc = fetch(&http, &format!("{b_base}/descriptor/{b_hash}")).await;
    world
        .bind(
            &format!("{B}/price"),
            &json::parse(&b_desc).unwrap(),
            price.get("since").and_then(Value::as_str).unwrap(),
        )
        .unwrap();
    for h in r.inputs() {
        let bytes = fetch(&http, &format!("{b_base}/receipt/{h}")).await;
        world.add_receipt(&json::parse(&bytes).unwrap()).unwrap();
        map.insert_receipt(h.clone(), bytes);
    }
    let args = r.cell().args.iter().map(|(k, v)| (k.clone(), v.scalar().unwrap().clone())).collect();
    let computed = world.compute(&CellKey { name: r.cell().function_name(), args }, r.asof()).unwrap();
    assert_eq!(computed.outcome.lineage, r.inputs(), "the recomputed lineage is the receipt's");
    map.insert_value(r.cell().id(), computed.outcome.value.clone());
    map.insert_lineage(r.cell().id(), computed.outcome.lineage.clone());
    let v = verify(receipt, &keys, Some(&map), None);
    assert!(v.ok, "{:?}", v.error());
    v.level.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_meshes() {
    let mut b = Node::start(B, |_| {}).await;
    b.stored(&format!("{B}/price"), PRICE_SIG);
    for (item, v) in [("bolt", "100"), ("nut", "5"), ("washer", "1")] {
        b.put_value(&format!("{B}/price"), &args(&[("item", s(item))]), n(v), T1);
    }
    let b_base = b.base.clone();
    let a = Node::start(A, |c| {
        c.peers.insert(B.into(), b_base.clone());
        c.peers.insert(C.into(), b_base.clone());
        c.ttl.current = Duration::from_secs(1);
        c.ttl.binding = Duration::from_secs(1);
        c.ttl.key_doc = Duration::from_secs(2);
        c.ttl.key_refetch = Duration::from_millis(200);
    })
    .await;
    // Defining on A a function that reads B's name: the name is resolved across meshes first.
    let m = a.mesh.clone();
    tokio::task::spawn_blocking(move || m.foreign().ensure_name(&format!("{B}/price")).unwrap()).await.unwrap();
    a.formula(&format!("{A}/with_tax"), &format!("{B}/price(item: item) * 1.2"), PRICE_SIG);

    let r = a.get(&format!("/cell/{A}/with_tax?item=bolt")).await;
    assert_eq!(r.outcome(), "ok", "{}", r.text());
    let env = r.json();
    assert_eq!(env.get("trust").and_then(Value::as_str), Some("verified"));
    let receipt = env.get("receipt").unwrap().to_jcs();
    let rec = Receipt::from_json(&receipt).unwrap();
    assert_eq!(rec.value().lexical(), "120");
    assert_eq!(rec.computed_by().split('#').next(), Some(A));
    assert_eq!(rec.inputs().len(), 1);
    assert_eq!(verify_without_trust(&a, &b_base, &receipt).await, Level::Recomputed);
    // B's own cell through A: B's receipt, unchanged.
    let direct = a.get(&format!("/cell/{B}/price?item=nut")).await;
    assert_eq!(direct.outcome(), "ok");
    assert_eq!(direct.json().get("trust").and_then(Value::as_str), Some("primary"));
    let from_b = b.get(&format!("/cell/{B}/price?item=nut")).await;
    assert_eq!(direct.json().get("receipt"), from_b.json().get("receipt"));

    // A host whose key document names another host: E_MESH_DOC.
    let r = a.get(&format!("/cell/{C}/price?item=bolt")).await;
    assert_eq!(r.outcome(), "error", "{}", r.text());
    assert_eq!(r.json().get("error").and_then(Value::as_str), Some("E_MESH_DOC"));

    // B rotates its key: the new key from now, the old one ending now; B restarts with it and
    // publishes a new price signed with the new key.
    let rotated_at = now();
    let k2 = datumesh_core::SigningKey::from_seed(&[77; 32]);
    let doc2 = datumesh_http::keydoc::with_key(B, Some(&key_doc(B, &b.key)), &k2, &rotated_at).unwrap();
    bl(|| b.store.put_namespace(B, &doc2, &rotated_at).unwrap());
    b.stop().await;
    let addr = b.addr.to_string();
    let b2 = Node::start_with(
        Setup { host: B.into(), key: k2.clone(), listen: addr, db: Some(b.db.clone()) },
        |_| {},
        |x| x,
    )
    .await;
    let t_new = plus(&now(), 1);
    b2.put_value_signed(&k2, &format!("{B}/price"), &args(&[("item", s("bolt"))]), n("110"), &t_new);
    let t0 = Instant::now();
    let picked = loop {
        let r = a.get(&format!("/cell/{A}/with_tax?item=bolt")).await;
        let v = r.json().get("receipt").and_then(|x| x.get("value")).and_then(Value::as_str).map(str::to_string);
        if v.as_deref() == Some("132") {
            break r;
        }
        assert!(t0.elapsed() < Duration::from_secs(6), "rotation not picked up: {}", r.text());
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let took = t0.elapsed();
    assert!(took <= Duration::from_secs(3), "picked up in {took:?}, beyond the TTLs (1 s current, 2 s key document)");
    let receipt2 = picked.json().get("receipt").unwrap().to_jcs();
    let input = Receipt::from_json(&bl(|| {
        b2.store.receipt(&Receipt::from_json(&receipt2).unwrap().inputs()[0]).unwrap().into_owned()
    }))
    .unwrap();
    assert_eq!(input.key_id(), k2.key_id(), "the input is signed with B's new key");
    assert_eq!(verify_without_trust(&a, &b2.base, &receipt2).await, Level::Recomputed);
    // The receipt from before the rotation still verifies against B's document now.
    assert_eq!(verify_without_trust(&a, &b2.base, &receipt).await, Level::Recomputed);
    std::fs::write(out_dir().join("cross-mesh.txt"), format!("rotation picked up in {took:?}\n")).unwrap();

    // B down: a cell A never read from B is an error, never not_covered; one A served
    // before, once its confirmation is older than the TTL, is stale.
    let mut b2 = b2;
    b2.stop().await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let r = a.get(&format!("/cell/{A}/with_tax?item=washer")).await;
    assert_eq!((r.status, r.outcome()), (503, "error".to_string()), "{}", r.text());
    assert_eq!(r.json().get("error").and_then(Value::as_str), Some("E_INPUT_UNAVAILABLE"));
    let r = a.get(&format!("/cell/{B}/price?item=washer")).await;
    assert_eq!(r.outcome(), "error", "{}", r.text());
    let r = a.get(&format!("/cell/{A}/with_tax?item=bolt")).await;
    assert_eq!(r.outcome(), "stale", "{}", r.text());
    assert_eq!(r.json().get("receipt").unwrap().to_jcs(), receipt2);
}
