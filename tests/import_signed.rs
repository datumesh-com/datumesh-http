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

fn key_doc_from(from: &str) -> Vec<u8> {
    datumesh_http::keydoc::with_key(HOST, None, &key_for(HOST), from).unwrap()
}
