//! Build 2 Part 2, R5: the publisher's catalogue in `bundle 1`. A bundle may carry a `catalog`
//! member (an array of entries `{"name", "kind", "label", "description", "args", "examples",
//! "dictionary"}` sorted by name), built by `bundle::build_with_catalog` (`build` is unchanged and
//! makes a bundle without one). The manifest signature covers it: a changed catalogue is refused
//! (`BundleError::Manifest`, the contract's `E_SIGNATURE`) and nothing is written. On import the
//! entries are the publisher's: the operator's catalogue file never overrides them.

mod common;

use std::sync::Arc;

use common::lite::*;
use common::readpath::*;
use common::*;
use datumesh_core::json::{self as cj, Value};
use datumesh_core::{Cell, FunctionDescriptor, Receipt, ReceiptBuilder};
use datumesh_http::bundle::{self, BundleError};
use datumesh_http::Config;
use datumesh_store::Store;
use serde_json::json;

const BNS: &str = "mesh-test.datumesh.com/scratch_b2b";
const LEVEL: &str = "mesh-test.datumesh.com/scratch_b2b/level";
const OTHER: &str = "mesh-test.datumesh.com/scratch_b2b/other";
const TX: &str = "2026-11-15T11:00:00.000000Z";
const DESCRIPTOR: &str = r#"{"code":{"body":"","lang":"stored"},"inputs":[],"signature":{"args":{"site":{"type":"string","valid":false}},"order":["site"],"returns":{"type":"number","unit":"1"}}}"#;

fn function() -> FunctionDescriptor {
    FunctionDescriptor::from_json(DESCRIPTOR.as_bytes()).unwrap()
}

fn receipt(site: &str, value: &str) -> Vec<u8> {
    let a = args(&[("site", s(site))]);
    let cell = Cell { ns: BNS.into(), fn_name: "level".into(), fn_hash: function().hash(), args: a };
    let r = ReceiptBuilder::new()
        .cell(cell)
        .value(n(value))
        .valid(args(&[]))
        .asof(TX)
        .tx(TX)
        .issued(TX)
        .publisher(BNS)
        .signer_host(HOST)
        .licence("CC0-1.0")
        .summary("test")
        .sign(&key_for(HOST))
        .unwrap();
    receipt_json(&r)
}

fn dict() -> serde_json::Value {
    json!({"site": [{"value": "NORTH", "label": "North pier", "aliases": ["np"]}]})
}

fn entry(name: &str, label: &str, description: &str) -> Vec<u8> {
    catalog_entry(name, "function", label, description, json!({"site": "the measuring site"}), dict())
}

fn make(catalog: Vec<Vec<u8>>) -> Vec<u8> {
    let names = vec![(LEVEL.to_string(), function().hash()), (OTHER.to_string(), function().hash())];
    let mut names = names;
    names.sort();
    let r = receipt("NORTH", "42");
    bundle::build_with_catalog(BNS, vec![function().canonical()], names, vec![r], catalog, &seed_for(HOST)).unwrap()
}

fn good() -> Vec<u8> {
    make(vec![entry(LEVEL, "Water level", "Water level readings at a site, osprey edition")])
}

fn import(lite: &Lite, bytes: &[u8]) -> Result<bundle::Imported, BundleError> {
    let owned: Vec<String> = Vec::new();
    bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(NOW), bytes)
}

/// Edits the catalogue member of a built bundle.
fn edit_catalog(bytes: &[u8], f: impl FnOnce(&mut Vec<Value>)) -> Vec<u8> {
    let mut f = Some(f);
    let mut v = cj::parse(bytes).unwrap();
    if let Value::Object(m) = &mut v {
        for (k, x) in m.iter_mut() {
            if k == "catalog" {
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

fn replace_in_entry(a: &mut [Value], from: &str, to: &str) {
    let text = String::from_utf8(a[0].to_jcs()).unwrap();
    assert!(text.contains(from), "{from} not in {text}");
    a[0] = cj::parse(text.replace(from, to).as_bytes()).unwrap();
}

fn nothing_written(lite: &Lite) {
    assert!(lite.store.resolve_name(LEVEL, END).unwrap().is_none(), "{LEVEL} is bound");
    assert!(lite.store.descriptor(&function().hash()).is_none(), "the descriptor is stored");
}

/// The catalogue is a member of the bundle, in the JCS order, and `build` without one is unchanged.
#[test]
fn the_catalog_is_a_member_of_the_bundle() {
    let v = cj::parse(&good()).unwrap();
    let keys: Vec<&str> = v.as_object().unwrap().iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["bundle", "catalog", "descriptors", "manifest", "names", "namespace", "receipts"]);
    let old = bundle::build(BNS, vec![function().canonical()], vec![], vec![], &seed_for(HOST)).unwrap();
    let v = cj::parse(&old).unwrap();
    assert!(v.get("catalog").is_none());
}

/// The signed catalogue imports, and its entries are served: label, description, the call template.
/// A bundle without a catalogue still imports.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_signed_catalog_imports_and_is_served() {
    let lite = Lite::new("bc-1");
    let i = import(&lite, &good()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(i.functions_bound, 2);
    let m = build_served(lite, None, |_| {}).await;
    let v = m.find("osprey").await;
    let r = result(&v, LEVEL);
    assert_eq!(r["label"], "Water level");
    assert_eq!(r["description"], "Water level readings at a site, osprey edition");
    assert_eq!(r["call"], format!("{LEVEL}(site: <string>)"));
    // the dictionary came with it: the alias `np` fills the key (`water` and `level` are catalogue words)
    let v = m.find("np water level").await;
    assert_eq!(status(&v), "exact", "{v}");
    assert_eq!(norm(v["resolution"]["call"].as_str().unwrap()), norm(&format!("{LEVEL}(site: \"NORTH\")")));
    assert!(v["resolution"]["label"].as_str().unwrap().contains("North pier"), "{v}");

    let lite = Lite::new("bc-1b");
    let plain = bundle::build(
        BNS,
        vec![function().canonical()],
        vec![(LEVEL.to_string(), function().hash())],
        vec![receipt("NORTH", "42")],
        &seed_for(HOST),
    )
    .unwrap();
    import(&lite, &plain).unwrap_or_else(|e| panic!("{e}"));
}

/// Importing twice changes nothing the second time.
#[test]
fn importing_twice_is_idempotent() {
    let lite = Lite::new("bc-2");
    import(&lite, &good()).unwrap();
    let again = import(&lite, &good()).unwrap();
    assert_eq!((again.functions_bound, again.versions), (0, 0));
}

/// The manifest signature covers the catalogue: any change to it, in the bundle as sent, fails
/// the import with `Manifest` (E_SIGNATURE) and writes nothing.
#[test]
fn a_tampered_catalog_is_refused() {
    let tampers: Vec<(&str, Box<dyn Fn(&[u8]) -> Vec<u8>>)> = vec![
        ("description", Box::new(|b| edit_catalog(b, |a| replace_in_entry(a, "osprey", "pelican")))),
        ("label", Box::new(|b| edit_catalog(b, |a| replace_in_entry(a, "Water level\"", "Free money\"")))),
        ("dictionary label", Box::new(|b| edit_catalog(b, |a| replace_in_entry(a, "North pier", "South pier")))),
        ("dictionary alias", Box::new(|b| edit_catalog(b, |a| replace_in_entry(a, "\"np\"", "\"sp\"")))),
        ("an entry added", Box::new(|b| {
            edit_catalog(b, |a| a.push(cj::parse(&entry(OTHER, "Other", "Another entry")).unwrap()))
        })),
        ("the catalogue removed", Box::new(|b| edit_catalog(b, |a| a.clear()))),
    ];
    for (what, f) in tampers {
        let lite = Lite::new("bc-3");
        let e = import(&lite, &f(&good())).unwrap_err();
        assert!(matches!(e, BundleError::Manifest(_)), "{what}: {e:?}");
        assert!(e.to_string().to_lowercase().contains("signature"), "{what}: {e}");
        nothing_written(&lite);
    }
    // the member dropped altogether from a bundle that was signed with one
    let lite = Lite::new("bc-3b");
    let mut v = cj::parse(&good()).unwrap();
    if let Value::Object(m) = &mut v {
        m.retain(|(k, _)| k != "catalog");
    }
    let e = import(&lite, &v.to_jcs()).unwrap_err();
    assert!(matches!(e, BundleError::Manifest(_)), "{e:?}");
    nothing_written(&lite);
}

/// A catalogue entry for a name outside the bundle's namespace is refused, nothing written.
#[test]
fn an_entry_outside_the_namespace_is_refused() {
    let lite = Lite::new("bc-4");
    let b = make(vec![entry("mesh-test.datumesh.com/elsewhere/level", "Level", "Not ours")]);
    let e = import(&lite, &b).unwrap_err();
    assert!(matches!(e, BundleError::NotOwned(_) | BundleError::Format(_)), "{e:?}");
    nothing_written(&lite);
}

/// The operator's catalogue file never overrides a publisher entry for the same name, and may add
/// an entry when there is none (`OTHER` has no publisher entry).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_operator_file_does_not_override_the_bundle() {
    let lite = Lite::new("bc-5");
    import(&lite, &good()).unwrap();
    let m = build_served(lite, None, |c: &mut Config| {
        c.catalog.insert(LEVEL.to_string(), "Operator words, pelican".to_string());
        c.catalog.insert(OTHER.to_string(), "Operator words about herons".to_string());
    })
    .await;
    let v = m.find("level").await;
    assert_eq!(result(&v, LEVEL)["description"], "Water level readings at a site, osprey edition");
    assert_eq!(result(&v, LEVEL)["label"], "Water level");
    assert!(m.find("pelican").await["results"].as_array().unwrap().is_empty());
    let v = m.find("herons").await;
    assert_eq!(result(&v, OTHER)["description"], "Operator words about herons");
    let _: Arc<dyn Store + Send + Sync> = m.lite.store.clone();
    let _ = Receipt::from_json;
}
