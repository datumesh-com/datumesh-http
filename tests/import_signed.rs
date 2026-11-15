//! `bundle 1` and `datumesh-mesh import-signed` (`CONTRACT-1.8b.md` H2). SQLite store (the CLI
//! test, which runs the binary, uses a PostgreSQL database as the binary does); the clock is fixed
//! at 2026-11-15T12:00:00.000000Z; the host is `mesh-test.datumesh.com`, whose key document in the
//! store holds the test key K from 2026-01-01.

mod common;

use std::sync::Arc;

use common::lite::*;
use common::*;
use datumesh_core::json::{self, Value};
use datumesh_core::{Cell, FunctionDescriptor, Receipt, ReceiptBuilder};
use datumesh_http::bundle::{self, BundleError};
use datumesh_http::{Config, Mesh};
use datumesh_store::Store;

const NS: &str = "mesh-test.datumesh.com/scratch_1_8b";
const LEVEL: &str = "mesh-test.datumesh.com/scratch_1_8b/level";
const OTHER: &str = "mesh-test.datumesh.com/scratch_1_8b/other";
const TX: &str = "2026-11-15T11:00:00.000000Z";
const DESCRIPTOR: &str = r#"{"code":{"body":"","lang":"stored"},"inputs":[],"signature":{"args":{"day":{"type":"date","valid":true}},"order":["day"],"returns":{"type":"number","unit":"1"}}}"#;

fn function() -> FunctionDescriptor {
    FunctionDescriptor::from_json(DESCRIPTOR.as_bytes()).unwrap()
}

fn fn_hash() -> String {
    function().hash()
}

fn day_args(day: &str) -> datumesh_core::Args {
    args(&[("day", d(day))])
}

fn cell_id(ns: &str, day: &str) -> String {
    Cell { ns: ns.into(), fn_name: "level".into(), fn_hash: fn_hash(), args: day_args(day) }.id()
}

/// A receipt of `level(day: <day>)` under `ns`, value number `value`, `tx` = asof = issued (SPEC 8), signed by K as
/// publisher `ns`; `inputs` makes it a computed receipt.
fn receipt(ns: &str, day: &str, value: &str, tx: &str, inputs: &[&str]) -> Vec<u8> {
    let cell = Cell { ns: ns.into(), fn_name: "level".into(), fn_hash: fn_hash(), args: day_args(day) };
    let mut b = ReceiptBuilder::new();
    b = b
        .cell(cell)
        .value(n(value))
        .valid(day_args(day))
        .asof(tx)
        .tx(tx)
        .issued(tx)
        .publisher(ns)
        .signer_host(HOST)
        .licence("CC0-1.0")
        .summary("test");
    if !inputs.is_empty() {
        b = b.inputs(inputs.iter().copied());
    }
    receipt_json(&b.sign(&key_for(HOST)).unwrap())
}

fn hash_of(r: &[u8]) -> String {
    Receipt::from_json(r).unwrap().hash().to_string()
}

/// A bundle of `ns`: the descriptor, `names` bound to its hash, `receipts` (put in ascending order
/// of their hashes), the manifest signed by `key`.
fn make(ns: &str, names: &[&str], mut receipts: Vec<Vec<u8>>, seed: &[u8; 32]) -> Vec<u8> {
    receipts.sort_by_key(|r| hash_of(r));
    let mut names: Vec<(String, String)> = names.iter().map(|n| (n.to_string(), fn_hash())).collect();
    names.sort();
    bundle::build(ns, vec![function().canonical()], names, receipts, seed).unwrap()
}

/// The bundle B, with its receipt's `tx` given.
fn bundle_b_at(tx: &str) -> Vec<u8> {
    make(NS, &[LEVEL], vec![receipt(NS, "2026-11-14", "42", tx, &[])], &seed_for(HOST))
}

fn bundle_b() -> Vec<u8> {
    bundle_b_at(TX)
}

/// The bundle with the object edited.
fn edit(bytes: &[u8], f: impl FnOnce(&mut Vec<(String, Value)>)) -> Vec<u8> {
    let mut v = json::parse(bytes).unwrap();
    if let Value::Object(m) = &mut v {
        f(m);
    }
    v.to_jcs()
}

fn import(lite: &Lite, bytes: &[u8]) -> Result<bundle::Imported, BundleError> {
    let owned: Vec<String> = Vec::new();
    bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(NOW), bytes)
}

/// Nothing of the bundle B (or of the days given) is in the store.
fn nothing_written(lite: &Lite, name: &str, ns: &str, days: &[&str]) {
    assert!(lite.store.resolve_name(name, END).unwrap().is_none(), "{name} is bound");
    assert!(lite.store.descriptor(&fn_hash()).is_none(), "the descriptor is stored");
    for day in days {
        assert!(lite.store.latest(&cell_id(ns, day)).unwrap().is_none(), "{day} has a version");
    }
}

/// 1. B imports; the name binds at now; the version is the receipt's; a mesh `get` answers ok.
#[test]
fn b_imports() {
    let lite = Lite::new("import-1");
    let i = import(&lite, &bundle_b()).unwrap();
    assert_eq!((i.functions_bound, i.receipts, i.versions), (1, 1, 1), "{i:?}");
    assert_eq!(i.bound_at, NOW);
    assert_eq!(lite.bound_tx(LEVEL).as_deref(), Some(NOW));
    let id = cell_id(NS, "2026-11-14");
    assert_eq!(lite.store.latest(&id).unwrap().unwrap().tx, TX);

    let shared: Arc<dyn Store + Send + Sync> = lite.store.clone();
    let mesh = Mesh::builder(Config::new(HOST), shared)
        .signing_key(lite.key.clone())
        .clock(Arc::new(FixedClock(NOW)))
        .build()
        .unwrap();
    let reply = mesh.cell(&format!("{LEVEL}?day=2026-11-14"), None);
    let env = json::parse(&reply.body).unwrap();
    assert_eq!(env.get("outcome").and_then(Value::as_str), Some("ok"), "{}", String::from_utf8_lossy(&reply.body));
    let got = Receipt::from_json(&env.get("receipt").unwrap().to_jcs()).unwrap();
    assert_eq!(got.hash(), hash_of(&receipt(NS, "2026-11-14", "42", TX, &[])));
}

/// 2. Importing B twice binds and versions nothing the second time.
#[test]
fn importing_twice_is_idempotent() {
    let lite = Lite::new("import-2");
    import(&lite, &bundle_b()).unwrap();
    let again = import(&lite, &bundle_b()).unwrap();
    assert_eq!((again.functions_bound, again.versions), (0, 0), "{again:?}");
}

/// 3. A namespace whose host is not the mesh's or an owned one is `NotOwned`; nothing is written.
#[test]
fn a_namespace_that_is_not_owned() {
    let lite = Lite::new("import-3");
    let other = "other.example/scratch";
    let name = "other.example/scratch/level";
    let b = make(other, &[name], vec![receipt(other, "2026-11-14", "42", TX, &[])], &seed_for(HOST));
    let e = import(&lite, &b).unwrap_err();
    assert!(matches!(e, BundleError::NotOwned(_)), "{e:?}");
    nothing_written(&lite, name, other, &["2026-11-14"]);
}

/// 4. A manifest signed by a key that is not in the key document is `Manifest`.
#[test]
fn a_manifest_signed_by_a_fresh_key() {
    let lite = Lite::new("import-4");
    let fresh = datumesh_core::sha256(b"datumesh-http test key: fresh");
    let b = make(NS, &[LEVEL], vec![receipt(NS, "2026-11-14", "42", TX, &[])], &fresh);
    let e = import(&lite, &b).unwrap_err();
    assert!(matches!(e, BundleError::Manifest(_)), "{e:?}");
    nothing_written(&lite, LEVEL, NS, &["2026-11-14"]);
}

/// 5. A receipt with one byte changed (value 42 to 43, signature kept), in a bundle whose manifest
///    signs it, is `Receipt`.
#[test]
fn a_tampered_receipt() {
    let lite = Lite::new("import-5");
    let good = String::from_utf8(receipt(NS, "2026-11-14", "42", TX, &[])).unwrap();
    let bad = good.replace("\"42\"", "\"43\"");
    assert_ne!(good, bad);
    let b = make(NS, &[LEVEL], vec![bad.into_bytes()], &seed_for(HOST));
    let e = import(&lite, &b).unwrap_err();
    assert!(matches!(e, BundleError::Receipt(_)), "{e:?}");
    nothing_written(&lite, LEVEL, NS, &["2026-11-14"]);
}

/// 6. A receipt whose tx is later than now is `TxInFuture`.
#[test]
fn a_receipt_from_the_future() {
    let lite = Lite::new("import-6");
    let b =
        make(NS, &[LEVEL], vec![receipt(NS, "2026-11-14", "42", "2026-11-15T12:00:00.000001Z", &[])], &seed_for(HOST));
    let e = import(&lite, &b).unwrap_err();
    assert!(matches!(e, BundleError::TxInFuture { .. }), "{e:?}");
    nothing_written(&lite, LEVEL, NS, &["2026-11-14"]);
}

/// 7. A receipt with non-empty `inputs` (computed) is `Receipt`.
#[test]
fn a_computed_receipt() {
    let lite = Lite::new("import-7");
    let input = "ab".repeat(32);
    let b = make(NS, &[LEVEL], vec![receipt(NS, "2026-11-14", "42", TX, &[&input])], &seed_for(HOST));
    let e = import(&lite, &b).unwrap_err();
    assert!(matches!(e, BundleError::Receipt(_)), "{e:?}");
    nothing_written(&lite, LEVEL, NS, &["2026-11-14"]);
}

/// 8. `names` out of order, and `"bundle": 2`, are `Format`.
#[test]
fn format_errors() {
    let lite = Lite::new("import-8");
    let two = make(NS, &[LEVEL, OTHER], vec![receipt(NS, "2026-11-14", "42", TX, &[])], &seed_for(HOST));
    // Sanity: the two-name bundle is fine as built.
    let ordered = edit(&two, |_| {});
    assert!(import(&lite, &ordered).is_ok());
    let lite = Lite::new("import-8b");
    let swapped = edit(&two, |m| {
        for (k, v) in m.iter_mut() {
            if k == "names" {
                if let Value::Array(a) = v {
                    a.reverse();
                }
            }
        }
    });
    let e = import(&lite, &swapped).unwrap_err();
    assert!(matches!(e, BundleError::Format(_)), "{e:?}");
    nothing_written(&lite, LEVEL, NS, &["2026-11-14"]);

    let v2 = edit(&bundle_b(), |m| {
        for (k, v) in m.iter_mut() {
            if k == "bundle" {
                *v = json::parse(b"2").unwrap();
            }
        }
    });
    let e = import(&lite, &v2).unwrap_err();
    assert!(matches!(e, BundleError::Format(_)), "{e:?}");
    nothing_written(&lite, LEVEL, NS, &["2026-11-14"]);
}

/// 9. All or nothing: two receipts, the second tampered: neither cell has a version, and the name
///    is unbound.
#[test]
fn all_or_nothing() {
    let lite = Lite::new("import-9");
    let first = receipt(NS, "2026-11-14", "42", TX, &[]);
    let second = String::from_utf8(receipt(NS, "2026-11-13", "41", TX, &[])).unwrap().replace("\"41\"", "\"43\"");
    let b = make(NS, &[LEVEL], vec![first, second.into_bytes()], &seed_for(HOST));
    let e = import(&lite, &b).unwrap_err();
    assert!(matches!(e, BundleError::Receipt(_)), "{e:?}");
    nothing_written(&lite, LEVEL, NS, &["2026-11-14", "2026-11-13"]);
}

/// 10. The CLI `import-signed` prints a JSON line with `"functions_bound":1`. The binary reads the
///     system clock and a PostgreSQL store (`DATABASE_URL`), so the receipt's tx is long past.
#[test]
fn the_cli_prints_imported() {
    let db = TestDb::migrated();
    {
        let store = db.store(2);
        store.put_namespace(HOST, &key_doc_from(K_FROM), K_FROM).unwrap();
    }
    let dir = out_dir().join(format!("import-signed-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("bundle.json");
    std::fs::write(&file, bundle_b_at("2026-11-01T00:00:00.000000Z")).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_datumesh-mesh"))
        .args(["import-signed", file.to_str().unwrap()])
        .env("DATABASE_URL", db.url())
        .env("DATUMESH_HOST", HOST)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{stdout} {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("\"functions_bound\":1"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

const LATER_NOW: &str = "2026-11-15T12:00:05.000000Z";
const GET_AT: &str = "2026-11-15T12:00:10.000000Z";
const FIRST_DAY: &str = "0001-01-01T00:00:00.000000Z";

fn mesh_at(lite: &Lite, clock: &'static str) -> Mesh {
    let shared: Arc<dyn Store + Send + Sync> = lite.store.clone();
    Mesh::builder(Config::new(HOST), shared)
        .signing_key(lite.key.clone())
        .clock(Arc::new(FixedClock(clock)))
        .build()
        .unwrap()
}

/// The outcome of a mesh `get` of `level(day: 2026-11-14)` at the clock `GET_AT`.
fn outcome(lite: &Lite) -> String {
    let reply = mesh_at(lite, GET_AT).cell(&format!("{LEVEL}?day=2026-11-14"), None);
    let env = json::parse(&reply.body).unwrap();
    env.get("outcome").and_then(Value::as_str).unwrap_or_default().to_string()
}

/// H5 1. Stopped after the n-th write (n in 1..=3): nothing is answerable (the name is unbound), and
///    a re-run at a later clock completes the import: the binding is the re-run's, one version, `ok`.
#[test]
fn a_stopped_import_is_completed_by_a_rerun() {
    for n in 1..=3 {
        let lite = Lite::new(&format!("import-h5-{n}"));
        let owned: Vec<String> = Vec::new();
        let e =
            bundle::import_signed_with(&*lite.store, HOST, &owned, &FixedClock(NOW), &bundle_b(), Some(n)).unwrap_err();
        assert!(matches!(e, BundleError::Stopped(m) if m == n), "n = {n}: {e:?}");
        assert_ne!(outcome(&lite), "ok", "n = {n}: the name is unbound, so nothing is answered");

        let i = bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(LATER_NOW), &bundle_b())
            .unwrap_or_else(|e| panic!("n = {n}: {e:?}"));
        assert_eq!(i.bound_at, LATER_NOW);
        assert_eq!(lite.bound_tx(LEVEL).as_deref(), Some(LATER_NOW), "n = {n}");
        let id = cell_id(NS, "2026-11-14");
        assert_eq!(lite.store.versions(&id, FIRST_DAY, END).unwrap().len(), 1, "n = {n}");
        assert_eq!(outcome(&lite), "ok", "n = {n}");
    }
}

/// H5 2. Stopped after the 4th write (everything written): the re-run binds and versions nothing, and
///    the binding's tx is the first run's.
#[test]
fn a_rerun_after_everything_was_written_changes_nothing() {
    let lite = Lite::new("import-h5-4");
    let owned: Vec<String> = Vec::new();
    let e = bundle::import_signed_with(&*lite.store, HOST, &owned, &FixedClock(NOW), &bundle_b(), Some(4)).unwrap_err();
    assert!(matches!(e, BundleError::Stopped(4)), "{e:?}");
    let again = bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(LATER_NOW), &bundle_b()).unwrap();
    assert_eq!((again.functions_bound, again.versions), (0, 0), "{again:?}");
    assert_eq!(lite.bound_tx(LEVEL).as_deref(), Some(NOW));
}

// ---------------------------------------------------------------------------------------------
// R1: refusals no test covered

/// A receipt of `fn_name` with the function hash `fh`, signed by `key` as signer host `signer`.
fn receipt_with(fn_name: &str, fh: &str, signer: &str, key: &datumesh_core::SigningKey) -> Vec<u8> {
    let cell = Cell { ns: NS.into(), fn_name: fn_name.into(), fn_hash: fh.into(), args: day_args("2026-11-14") };
    let r = ReceiptBuilder::new()
        .cell(cell)
        .value(n("42"))
        .valid(day_args("2026-11-14"))
        .asof(TX)
        .tx(TX)
        .issued(TX)
        .publisher(NS)
        .signer_host(signer)
        .licence("CC0-1.0")
        .summary("test")
        .sign(key)
        .unwrap();
    receipt_json(&r)
}

fn import_owned(lite: &Lite, owned: &[&str], bytes: &[u8]) -> Result<bundle::Imported, BundleError> {
    let owned: Vec<String> = owned.iter().map(|o| o.to_string()).collect();
    bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(NOW), bytes)
}

fn refused_with(lite: &Lite, bytes: &[u8], is: impl Fn(&BundleError) -> bool) {
    let e = import(lite, bytes).unwrap_err();
    assert!(is(&e), "{e:?}");
    nothing_written(lite, LEVEL, NS, &["2026-11-14"]);
}

/// R1.1. An owned host whose key document is not in the store is `Manifest`.
#[test]
fn an_owned_host_without_a_key_document() {
    let lite = Lite::new("import-r1-1");
    let other = "other.example/ns";
    let name = "other.example/ns/level";
    let b = make(other, &[name], vec![receipt(other, "2026-11-14", "42", TX, &[])], &seed_for("other.example"));
    let e = import_owned(&lite, &["other.example"], &b).unwrap_err();
    assert!(matches!(e, BundleError::Manifest(_)), "{e:?}");
    nothing_written(&lite, name, other, &["2026-11-14"]);
}

/// R1.2. The manifest key is in the key document, but its window does not hold now (`valid_to`
///    before the clock) is `Manifest`.
#[test]
fn a_key_whose_window_has_closed() {
    let lite = Lite::new("import-r1-2");
    let closed = "2026-11-15T00:00:00.000000Z";
    let successor = datumesh_core::SigningKey::from_seed(&datumesh_core::sha256(b"datumesh-http test key: successor"));
    let existing = key_doc_from(K_FROM);
    let doc = datumesh_http::keydoc::with_key(HOST, Some(&existing), &successor, closed).unwrap();
    assert!(String::from_utf8_lossy(&doc).contains(&format!(r#""valid_to":"{closed}""#)));
    lite.store.put_namespace(HOST, &doc, closed).unwrap();
    refused_with(&lite, &bundle_b(), |e| matches!(e, BundleError::Manifest(_)));
}

/// R1.3. A valid bundle whose `names` gain an entry after signing is `Manifest`.
#[test]
fn names_added_after_signing() {
    let lite = Lite::new("import-r1-3");
    let b = edit(&bundle_b(), |m| {
        for (k, v) in m.iter_mut() {
            if k == "names" {
                if let Value::Array(a) = v {
                    let entry = format!(r#"{{"fn_hash":"{}","name":"{OTHER}"}}"#, fn_hash());
                    a.push(json::parse(entry.as_bytes()).unwrap());
                }
            }
        }
    });
    refused_with(&lite, &b, |e| matches!(e, BundleError::Manifest(_)));
    assert!(lite.store.resolve_name(OTHER, END).unwrap().is_none());
}

/// R1.4. A descriptor that is not canonical (members out of JCS order) is `Descriptor`. (The bundle's
///    bytes are edited as text, so that the members are out of order as the importer reads them.)
#[test]
fn a_descriptor_that_is_not_canonical() {
    let lite = Lite::new("import-r1-4");
    let text = String::from_utf8(bundle_b()).unwrap();
    // Member order inside an embedded object carries no meaning (JCS); an unknown member does.
    let from = r#""code":{"body":"","lang":"stored"},"inputs":[],"#;
    let to = r#""code":{"body":"","lang":"stored"},"extra":1,"inputs":[],"#;
    assert!(text.contains(from));
    let b = text.replace(from, to).into_bytes();
    refused_with(&lite, &b, |e| matches!(e, BundleError::Descriptor(_)));
}

/// R1.5. A name whose `fn_hash` is neither among the bundle's descriptors nor stored is `Descriptor`.
#[test]
fn a_name_without_a_descriptor() {
    let lite = Lite::new("import-r1-5");
    let b = bundle::build(NS, vec![], vec![(LEVEL.to_string(), fn_hash())], vec![], &seed_for(HOST)).unwrap();
    refused_with(&lite, &b, |e| matches!(e, BundleError::Descriptor(_)));
}

/// R1.6. A receipt of a function not among `names` is `Receipt`.
#[test]
fn a_receipt_of_an_unnamed_function() {
    let lite = Lite::new("import-r1-6");
    let b = make(NS, &[LEVEL], vec![receipt_with("other", &fn_hash(), HOST, &key_for(HOST))], &seed_for(HOST));
    refused_with(&lite, &b, |e| matches!(e, BundleError::Receipt(_)));
}

/// R1.7. A receipt whose `cell.fn_hash` differs from its name's `fn_hash` is `Receipt`.
#[test]
fn a_receipt_of_another_function_hash() {
    let lite = Lite::new("import-r1-7");
    let b = make(NS, &[LEVEL], vec![receipt_with("level", &"ab".repeat(32), HOST, &key_for(HOST))], &seed_for(HOST));
    refused_with(&lite, &b, |e| matches!(e, BundleError::Receipt(_)));
}

/// R1.8. A receipt signed by another host's key (signer host `other.example`) is `Receipt`.
#[test]
fn a_receipt_signed_by_another_host() {
    let lite = Lite::new("import-r1-8");
    let r = receipt_with("level", &fn_hash(), "other.example", &key_for("other.example"));
    let b = make(NS, &[LEVEL], vec![r], &seed_for(HOST));
    refused_with(&lite, &b, |e| matches!(e, BundleError::Receipt(_)));
}

/// R1.9. A name already bound to another hash from a start at or after the clock is `Bind`.
#[test]
fn a_name_bound_from_the_future() {
    let lite = Lite::new("import-r1-9");
    let other = "{\"code\":{\"body\":\"\",\"lang\":\"stored\"},\"inputs\":[],\"signature\":{\"args\":{\"day\":{\"type\":\"date\",\"valid\":true}},\"order\":[\"day\"],\"returns\":{\"type\":\"number\",\"unit\":\"EUR\"}}}";
    let h = lite
        .store
        .put_descriptor(FunctionDescriptor::from_json(other.as_bytes()).unwrap().canonical().as_slice())
        .unwrap();
    assert_ne!(h, fn_hash());
    let from = "2026-11-15T13:00:00.000000Z";
    lite.store.bind_name(LEVEL, &h, from).unwrap();
    let e = import(&lite, &bundle_b()).unwrap_err();
    assert!(matches!(e, BundleError::Bind(_)), "{e:?}");
    assert_eq!(lite.store.resolve_name(LEVEL, END).unwrap().as_deref(), Some(h.as_str()));
    assert_eq!(lite.bound_tx(LEVEL).as_deref(), Some(from));
    assert!(lite.store.descriptor(&fn_hash()).is_none(), "the descriptor is stored");
    assert!(lite.store.latest(&cell_id(NS, "2026-11-14")).unwrap().is_none());
}

/// R1.10. An extra top-level member, a manifest signature that is not hex: `Format`. A receipt that
///    does not parse: an error, and nothing is written.
#[test]
fn malformed_bundles() {
    let lite = Lite::new("import-r1-10");
    let extra = edit(&bundle_b(), |m| m.push(("extra".to_string(), json::parse(b"1").unwrap())));
    refused_with(&lite, &extra, |e| matches!(e, BundleError::Format(_)));

    let not_hex = edit(&bundle_b(), |m| {
        for (k, v) in m.iter_mut() {
            if k == "manifest" {
                if let Value::Object(mm) = v {
                    for (mk, mv) in mm.iter_mut() {
                        if mk == "signature" {
                            *mv = Value::String("not hex at all".into());
                        }
                    }
                }
            }
        }
    });
    refused_with(&lite, &not_hex, |e| matches!(e, BundleError::Format(_)));

    let junk = edit(&bundle_b(), |m| {
        for (k, v) in m.iter_mut() {
            if k == "receipts" {
                *v = json::parse(br#"[{"junk":1}]"#).unwrap();
            }
        }
    });
    assert!(import(&lite, &junk).is_err());
    nothing_written(&lite, LEVEL, NS, &["2026-11-14"]);
}

fn key_doc_from(from: &str) -> Vec<u8> {
    datumesh_http::keydoc::with_key(HOST, None, &key_for(HOST), from).unwrap()
}
