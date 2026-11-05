//! spec-0.4.3 (http 0.2.1): licence inheritance (SPEC 8.5) as the mesh answers it, and
//! `declarations` in the key document (SPEC 9.2).
//!
//! - `/.well-known/datumesh.json` serves the declarations the store's key document holds;
//!   without any, the same bytes as 0.2.0. `keydoc::with_declaration` adds, keeps and closes
//!   entries; `keydoc::with_key` keeps them through a rotation.
//! - The seed (deploy/seed/ecb.py, deploy/seed/declarations.py, and the strip_niqqud line of
//!   deploy/seed/scripture.py): fx_cross over the NOK cell carries the restricted test licence,
//!   fx_cross_undeclared over it answers `unlicensed` (403), strip_niqqud carries the UXLC
//!   licence; the entitlement check (core's `check_entitlement`, SPEC 10.3) answers entitled for
//!   every receipt served, from the served key document and input receipts.
//! - Another mesh's formula, computed here: its publisher's declarations come from its key
//!   document (fetched, then in the store); a publisher that gives no response, once its
//!   document's TTL has passed, is `E_FOREIGN_UNAVAILABLE` (502) with its `detail`.

mod common;

use std::process::Command;
use std::time::Duration;

use common::*;
use datumesh_core::json::{self, Value};
use datumesh_core::{check_entitlement, KeyDocument, LicenceReason, Receipt};
use datumesh_http::keydoc::{with_declaration, with_key, Declared};

const A: &str = "mesh-a.example";
const B: &str = "mesh-b.example";
const UXLC: &str = "https://tanach.us/License.html";
const FROM: &str = "2020-01-01T00:00:00.000000Z";

fn repo() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Runs a seed script of deploy/seed with `python3 -I`; its standard output.
fn python(script: &str, args: &[&str]) -> Vec<u8> {
    let out = Command::new("python3")
        .arg("-I")
        .arg(repo().join("deploy/seed").join(script))
        .args(args)
        .output()
        .expect("python3 runs the seed scripts");
    assert!(out.status.success(), "{script}: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

/// The entitlement check of a receipt this mesh served, with the publisher's key document as
/// the mesh serves it and the input receipts from `/receipt/<hash>`.
async fn entitled(node: &Node, receipt: &[u8]) -> datumesh_core::Entitlement {
    let r = Receipt::from_json(receipt).unwrap();
    let host = r.cell().ns.split('/').next().unwrap().to_string();
    assert_eq!(host, node.host);
    let doc = node.get("/.well-known/datumesh.json").await;
    let k = KeyDocument::from_json(&host, &doc.body).unwrap();
    let mut licences = Vec::new();
    for h in r.inputs() {
        assert!(!h.starts_with("slice:"));
        let got = node.get(&format!("/receipt/{h}")).await;
        assert_eq!(got.status, 200);
        licences.push(Receipt::from_json(&got.body).unwrap().licence().to_string());
    }
    check_entitlement(&r, licences, &k).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn declarations_are_served_and_kept() {
    let a = Node::start(A, |_| {}).await;
    // None declared: the document as 0.2.0 served it, byte for byte.
    let plain = a.get("/.well-known/datumesh.json").await;
    assert_eq!(plain.status, 200);
    assert_eq!(plain.body, key_doc(A, &a.key));
    let name = format!("{A}/ecb/fx_cross");
    let restricted = format!("https://{A}/licences/restricted-test");
    let (doc, what) = with_declaration(A, &plain.body, &name, &restricted, FROM, None).unwrap();
    assert_eq!(what, Declared::Added);
    // Again: unchanged, the same bytes.
    assert_eq!(with_declaration(A, &doc, &name, &restricted, FROM, None).unwrap(), (doc.clone(), Declared::Unchanged));
    bl(|| a.store.put_namespace(A, &doc, &now()).unwrap());
    let served = a.get("/.well-known/datumesh.json").await;
    assert_eq!(served.body, doc);
    let k = KeyDocument::from_json(A, &served.body).unwrap();
    let d = k.declarations.unwrap();
    assert_eq!(d.get(&name).len(), 1);
    assert_eq!(d.get(&name)[0].licence, restricted);
    // The keys verify as before (declarations change nothing in verification).
    assert_eq!(datumesh_core::keys_from_response(A, 200, Some(&served.body)).unwrap().key_ids(), vec![a.key.key_id()]);
    // A rotation keeps them.
    let k2 = datumesh_core::SigningKey::from_seed(&[42; 32]);
    let rotated = with_key(A, Some(&doc), &k2, "2030-01-01T00:00:00.000000Z").unwrap();
    let kd = KeyDocument::from_json(A, &rotated).unwrap();
    assert_eq!(kd.declarations.as_ref().unwrap().get(&name).len(), 1);
    assert_eq!(kd.keys.key_ids().len(), 2);
    // Closed, never removed; another entry with the same start is refused; a name under
    // another host too.
    let to = "2030-01-01T00:00:00.000000Z";
    let (closed, what) = with_declaration(A, &doc, &name, &restricted, FROM, Some(to)).unwrap();
    assert_eq!(what, Declared::Closed);
    let kd = KeyDocument::from_json(A, &closed).unwrap();
    assert_eq!(kd.declarations.unwrap().get(&name)[0].valid_to.as_deref(), Some(to));
    assert!(with_declaration(A, &closed, &name, &restricted, FROM, Some("2031-01-01T00:00:00.000000Z")).is_err());
    assert!(with_declaration(A, &doc, &format!("{B}/f"), &restricted, FROM, None).is_err());
    assert!(with_declaration(A, &doc, &name, "not a licence", FROM, None).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_seed_under_licence_inheritance() {
    let a = Node::start(A, |_| {}).await;
    let dir = out_dir().join(format!("licence-seed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    python("ecb.py", &[dir.to_str().unwrap(), A]);
    let key = a.key.clone();
    let ecb = dir.join("ecb.jsonl");
    bl(|| datumesh_http::publish::publish_stored(&*a.store, &key, A, &ecb).unwrap());
    // The strip_niqqud seed, as deploy/seed/scripture.py writes it (one verse).
    let text = "בְּרֵאשִׁ֖ית בָּרָ֣א אֱלֹהִ֑ים".as_bytes();
    std::fs::write(dir.join("gen-1-1.txt"), text).unwrap();
    // The characters of the spec vector formula-eval/hebrew-consonantal, as scripture.py takes
    // them.
    let vector =
        json::parse(&std::fs::read(spec_dir().join("vectors/formula-eval/hebrew-consonantal.json")).unwrap()).unwrap();
    let body = vector.get("world").and_then(|w| w.get("functions")).and_then(Value::as_object).unwrap()[0]
        .1
        .get("descriptor")
        .and_then(|d| d.get("code"))
        .and_then(|c| c.get("body"))
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let strip = body.rsplit_once(", \"").unwrap().1.split('"').next().unwrap().to_string();
    let source = format!(
        r#"remove_chars(text_of({A}/scripture/verse_text(corpus: "tanakh", book: book, chapter: chapter, verse: verse, edition: "uxlc")), "{strip}")"#
    );
    let formula = Value::Object(vec![
        ("bound".into(), Value::String("2026-11-02T00:00:00.000000Z".into())),
        ("formula".into(), Value::String(format!("{A}/scripture/strip_niqqud"))),
        ("signature".into(), json::parse(br#"{"args":{"book":{"type":"string","valid":false},"chapter":{"type":"number","unit":"1","valid":false},"verse":{"type":"number","unit":"1","valid":false}},"order":["book","chapter","verse"],"returns":"string"}"#).unwrap()),
        ("source".into(), Value::String(source)),
    ]);
    let lines = format!(
        r#"{{"function":"{A}/scripture/verse_text","signature":{{"args":{{"book":{{"type":"string","valid":false}},"chapter":{{"type":"number","unit":"1","valid":false}},"corpus":{{"type":"string","valid":false}},"edition":{{"type":"string","valid":false}},"verse":{{"type":"number","unit":"1","valid":false}}}},"order":["corpus","book","chapter","verse","edition"],"returns":"blob"}},"bound":"2026-11-02T00:00:00.000000Z"}}
{{"cell":{{"fn":"{A}/scripture/verse_text","args":{{"book":{{"type":"string","value":"Gen"}},"chapter":{{"type":"number","value":"1"}},"corpus":{{"type":"string","value":"tanakh"}},"edition":{{"type":"string","value":"uxlc"}},"verse":{{"type":"number","value":"1"}}}}}},"value":{{"blob":"gen-1-1.txt","media_type":"text/plain"}},"tx":"2026-11-02T00:00:00.000000Z","asof":"2026-11-02T00:00:00.000000Z","issued":"2026-11-02T00:00:00.000000Z","licence":"{UXLC}","summary":"Test: Genesis 1:1, UXLC 2.6."}}
{}
"#,
        String::from_utf8(formula.to_jcs()).unwrap()
    );
    let scripture = dir.join("scripture.jsonl");
    std::fs::write(&scripture, lines).unwrap();
    bl(|| datumesh_http::publish::publish_stored(&*a.store, &key, A, &scripture).unwrap());
    // The declarations, as deploy/licences.sh applies them with `datumesh-mesh declare`.
    let decls = python("declarations.py", &[A]);
    let mut n = 0;
    for line in String::from_utf8(decls).unwrap().lines() {
        let v = json::parse(line.as_bytes()).unwrap();
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap().to_string();
        a.declare(&s("fn"), &s("licence"), &s("valid_from"));
        n += 1;
    }
    assert_eq!(n, 2, "exactly two declarations");
    let restricted = format!("https://{A}/licences/restricted-test");

    // fx_cross over the NOK cell: the restricted licence, entitled (declared).
    let q = "base=USD&date=2026-10-30&quote=NOK";
    let r = a.get(&format!("/cell/{A}/ecb/fx_cross?{q}")).await;
    assert_eq!(r.outcome(), "ok", "{}", r.text());
    assert_eq!(r.json().get("trust").and_then(Value::as_str), Some("computed"));
    let receipt = r.json().get("receipt").unwrap().to_jcs();
    let rec = Receipt::from_json(&receipt).unwrap();
    assert_eq!(rec.licence(), restricted);
    assert_eq!(rec.inputs().len(), 2);
    let e = entitled(&a, &receipt).await;
    assert!(e.entitled && e.reason == LicenceReason::Declared, "{e:?}");
    // Over open cells only: CC-BY-4.0, entitled (open).
    let r = a.get(&format!("/cell/{A}/ecb/fx_cross?base=USD&date=2026-10-30&quote=GBP")).await;
    let receipt = r.json().get("receipt").unwrap().to_jcs();
    assert_eq!(Receipt::from_json(&receipt).unwrap().licence(), "CC-BY-4.0");
    assert_eq!(entitled(&a, &receipt).await.reason, LicenceReason::Open);

    // fx_cross_undeclared over the NOK cell: unlicensed (403), the function hash echoed;
    // over open cells it computes.
    let r = a.get(&format!("/cell/{A}/ecb/fx_cross_undeclared?{q}")).await;
    assert_eq!((r.status, r.outcome()), (403, "unlicensed".to_string()), "{}", r.text());
    assert!(r.json().get("fn_hash").and_then(Value::as_str).is_some());
    assert!(r.json().get("receipt").is_none());
    schemas().assert_envelope(&r.body);
    let r = a.get(&format!("/cell/{A}/ecb/fx_cross_undeclared?base=USD&date=2026-10-30&quote=GBP")).await;
    assert_eq!(r.outcome(), "ok", "{}", r.text());

    // strip_niqqud: the UXLC licence, entitled (declared).
    let r = a.get(&format!("/cell/{A}/scripture/strip_niqqud?book=Gen&chapter=1&verse=1")).await;
    assert_eq!(r.outcome(), "ok", "{}", r.text());
    let receipt = r.json().get("receipt").unwrap().to_jcs();
    assert_eq!(Receipt::from_json(&receipt).unwrap().licence(), UXLC);
    let e = entitled(&a, &receipt).await;
    assert!(e.entitled && e.reason == LicenceReason::Declared, "{e:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

const PRICE_SIG: &str =
    r#"{"args":{"item":{"type":"string","valid":false}},"order":["item"],"returns":{"type":"number","unit":"EUR"}}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn another_meshs_formula_with_its_publishers_declarations() {
    let mut b = Node::start(B, |_| {}).await;
    let restricted = format!("https://{B}/licences/restricted");
    b.stored(&format!("{B}/price"), PRICE_SIG);
    let bk = b.key.clone();
    for item in ["bolt", "nut"] {
        b.put_value_licensed(&bk, &format!("{B}/price"), &args(&[("item", s(item))]), n("100"), T1, &restricted);
    }
    for f in ["with_tax", "with_tax_undeclared"] {
        b.formula(&format!("{B}/{f}"), &format!("{B}/price(item: item) * 1.2"), PRICE_SIG);
    }
    b.declare(&format!("{B}/with_tax"), &restricted, FROM);
    let b_base = b.base.clone();
    let a = Node::start(A, |c| {
        c.peers.insert(B.into(), b_base.clone());
        c.ttl.current = Duration::from_secs(1);
        c.ttl.binding = Duration::from_secs(1);
        c.ttl.key_doc = Duration::from_secs(1);
        c.ttl.key_refetch = Duration::from_millis(200);
    })
    .await;
    // Computed by A, under the licence B declared for its function.
    let r = a.get(&format!("/cell/{B}/with_tax?item=bolt")).await;
    assert_eq!(r.outcome(), "ok", "{}", r.text());
    let rec = Receipt::from_json(&r.json().get("receipt").unwrap().to_jcs()).unwrap();
    assert_eq!(rec.computed_by().split('#').next(), Some(A));
    assert_eq!(rec.licence(), restricted);
    let stored = bl(|| a.store.namespace(B).unwrap().unwrap().keydoc);
    assert!(KeyDocument::from_json(B, &stored).unwrap().declarations.is_some(), "B's declarations in A's store");
    // Not declared by B: unlicensed.
    let r = a.get(&format!("/cell/{B}/with_tax_undeclared?item=bolt")).await;
    assert_eq!((r.status, r.outcome()), (403, "unlicensed".to_string()), "{}", r.text());
    // B gives no response; once its key document's TTL has passed, A cannot read its
    // declarations: E_FOREIGN_UNAVAILABLE, 502, detail `refused` (a cell A never served, so
    // no stale answer).
    b.stop().await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let r = a.get(&format!("/cell/{B}/with_tax?item=nut")).await;
    assert_eq!((r.status, r.outcome()), (502, "error".to_string()), "{}", r.text());
    assert_eq!(r.json().get("error").and_then(Value::as_str), Some("E_FOREIGN_UNAVAILABLE"));
    assert_eq!(r.json().get("detail").and_then(Value::as_str), Some("refused"));
    schemas().assert_envelope(&r.body);
}
