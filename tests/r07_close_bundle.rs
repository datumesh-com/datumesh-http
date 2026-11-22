//! The 0.7 round (CONTRACT-round-0.7.md), R2: the bundle 1 member `"close": ["<full name>", ...]` (SPEC 9.7 "Closing a name").
//! A fifth file beside the four the coordinator named, so that the guess below cannot stop the others compiling.
//!
//! **Assumed API** (the contract does not name it): `datumesh_http::bundle::build_with_close(namespace, descriptors, names,
//! receipts, close, seed)` where `close` is a `Vec<String>` of full names, as `build` takes its other lists; the member is
//! signed by the manifest like `catalog` and `tests`, and sorted ascending in the bundle. The import closes each name at the
//! importing mesh's clock (here the fixed `T2`); a name that is not bound is refused (`E_NAME_UNBOUND`), and nothing is written.

mod common;

use common::lite::*;
use common::*;
use datumesh_core::{Cell, FunctionDescriptor, ReceiptBuilder};
use datumesh_http::bundle::{self, BundleError};
use datumesh_store::Store;

const BNS: &str = "mesh-test.datumesh.com/scratch_r07c";
const LEVEL: &str = "mesh-test.datumesh.com/scratch_r07c/level";
const OTHER: &str = "mesh-test.datumesh.com/scratch_r07c/other";
const GONE: &str = "mesh-test.datumesh.com/scratch_r07c/never_bound";
const TX: &str = "2026-11-15T11:00:00.000000Z";
const T2: &str = "2026-11-15T12:00:02.000000Z";
const DESCRIPTOR: &str = r#"{"code":{"body":"","lang":"stored"},"inputs":[],"signature":{"args":{"site":{"type":"string","valid":false}},"order":["site"],"returns":{"type":"number","unit":"1"}}}"#;

fn function() -> FunctionDescriptor {
    FunctionDescriptor::from_json(DESCRIPTOR.as_bytes()).unwrap()
}

fn receipt() -> Vec<u8> {
    let a = args(&[("site", s("NORTH"))]);
    let cell = Cell { ns: BNS.into(), fn_name: "level".into(), fn_hash: function().hash(), args: a };
    let r = ReceiptBuilder::new()
        .cell(cell)
        .value(n("42"))
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

fn names() -> Vec<(String, String)> {
    let mut v = vec![(LEVEL.to_string(), function().hash()), (OTHER.to_string(), function().hash())];
    v.sort();
    v
}

/// The first bundle: two names bound, one receipt.
fn first() -> Vec<u8> {
    bundle::build(BNS, vec![function().canonical()], names(), vec![receipt()], &seed_for(HOST)).unwrap()
}

/// A bundle that only closes `close`.
fn closing(close: &[&str]) -> Vec<u8> {
    bundle::build_with_close(
        BNS,
        vec![],
        vec![],
        vec![],
        close.iter().map(|c| c.to_string()).collect(),
        &seed_for(HOST),
    )
    .unwrap()
}

fn import(lite: &Lite, bytes: &[u8], clock: &'static str) -> Result<bundle::Imported, BundleError> {
    let owned: Vec<String> = Vec::new();
    bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(clock), bytes)
}

/// The `close` member is in the bundle, in the JCS order of the members, signed with the manifest: a changed list is refused
/// as a changed catalogue is (`BundleError::Manifest`), and nothing is closed.
#[test]
fn r2_the_close_member_is_signed() {
    let b = closing(&[LEVEL]);
    let v = datumesh_core::json::parse(&b).unwrap();
    let keys: Vec<&str> = v.as_object().unwrap().iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["bundle", "close", "descriptors", "manifest", "names", "namespace", "receipts"]);
    let tampered = String::from_utf8(b).unwrap().replace(LEVEL, OTHER);
    let lite = Lite::new("r07-cb-sign");
    import(&lite, &first(), NOW).unwrap_or_else(|e| panic!("{e}"));
    let e = import(&lite, tampered.as_bytes(), T2).unwrap_err();
    assert!(matches!(e, BundleError::Manifest(_)), "{e:?}");
    assert!(lite.store.resolve_binding(OTHER, END).unwrap().is_some(), "nothing closed");
}

/// A bundle with `close` ends the named bindings at the importing mesh's clock; the others stay; history keeps the old binding
/// (as of before the close the name still resolves, with its `since`).
#[test]
fn r2_a_bundle_closes_the_names_it_lists() {
    let lite = Lite::new("r07-cb-close");
    import(&lite, &first(), NOW).unwrap_or_else(|e| panic!("{e}"));
    assert!(lite.store.resolve_binding(LEVEL, END).unwrap().is_some());
    import(&lite, &closing(&[LEVEL]), T2).unwrap_or_else(|e| panic!("{e}"));
    assert!(lite.store.resolve_binding(LEVEL, END).unwrap().is_none(), "closed");
    assert!(lite.store.resolve_binding(LEVEL, T2).unwrap().is_none(), "closed at the clock of the import");
    let was =
        lite.store.resolve_binding(LEVEL, "2026-11-15T12:00:01.000000Z").unwrap().expect("bound before the close");
    assert_eq!(was.tx, NOW);
    assert!(lite.store.resolve_binding(OTHER, END).unwrap().is_some(), "the other name stays");
}

/// A name that is not bound cannot be closed by a bundle either: refused, and nothing is written (the bound name before it in
/// the list stays bound).
#[test]
fn r2_a_bundle_that_closes_an_unbound_name_is_refused() {
    let lite = Lite::new("r07-cb-unbound");
    import(&lite, &first(), NOW).unwrap_or_else(|e| panic!("{e}"));
    let e = import(&lite, &closing(&[GONE, LEVEL]), T2).expect_err("an unbound name cannot be closed");
    assert!(format!("{e:?}{e}").contains("E_NAME_UNBOUND"), "{e:?}");
    assert!(lite.store.resolve_binding(LEVEL, END).unwrap().is_some(), "nothing was closed");
}
