//! The catalogue, phase 1 (CONTRACT-classes-round.md), C2: facets on catalogue entries.
//!
//! The `catalog` line of `publish-stored` and the `catalog` member of bundle 1 may carry `"facets": {"domain": [...], "thing":
//! [...], "measure": [...], "region": [...], "frequency": [...]}`: the five keys, each optional, none other; each value a name
//! (2.2) of at most 32 bytes; a list of 1 to 8 values, ascending and unique. Facets are signed with the bundle and outside every
//! hash. The store (0.5.3: `upsert_catalog_ext`, `catalog_entry_ext`, `catalog_entries_with_facets`) keeps them. A function
//! answer and a class answer of `describe`, and a `find` result, carry `"facets"` when the entry has them and no member when it
//! has none.
//!
//! Choices where the contract is silent (the implementer follows them):
//! * A `catalog` line without `facets` for an entry that had them REMOVES them (the line is the publisher's whole statement of
//!   the entry, so a removed facet is a republished line without it; C5's "removed facet" needs this).
//! * `"facets": {}` is not tested (an empty object is not one of the bounds the contract states).
//! * The schemas of the spec are not asserted here (the spec's own vectors and schemas are its tests).

#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;

use catworld::*;
use common::lite::*;
use common::pub031::{function_line, refused_whole, run, snapshot, write, BEFORE};
use common::readpath::*;
use common::world::call_path;
use common::*;
use crworld::*;
use datumesh_http::bundle::{self, BundleError};
use datumesh_store::{CatalogKind, Store};
use serde_json::{json, Value};

const F: &str = "mesh-test.datumesh.com/scratch_c2/level";
const G: &str = "mesh-test.datumesh.com/scratch_c2/plain";
const BNS: &str = "mesh-test.datumesh.com/scratch_c2b";
const BF: &str = "mesh-test.datumesh.com/scratch_c2b/level";

fn line_with(facets: Value) -> String {
    catalog_line_of(F, "function", json!({"facets": facets}))
}

fn base() -> String {
    fn_line(F)
}

// ---- parse bounds ---------------------------------------------------------------------------------------------------------

/// What the bounds accept: each key alone, all five, 8 values, a name of 32 bytes, several values ascending.
#[test]
fn c2_facets_within_the_bounds_are_accepted() {
    let eight = ["a1", "b2", "c3", "d4", "e5", "f6", "g7", "h8"];
    let long32 = "a".repeat(32);
    let cases: Vec<(&str, Value)> = vec![
        ("domain", json!({"domain": ["finance"]})),
        ("thing", json!({"thing": ["currency"]})),
        ("measure", json!({"measure": ["exchange_rate"]})),
        ("region", json!({"region": ["ea"]})),
        ("frequency", json!({"frequency": ["on_change"]})),
        ("none as a value", json!({"thing": ["none"], "region": ["none"]})),
        (
            "all five",
            json!({"domain": ["finance"], "thing": ["rate"], "measure": ["interest_rate"], "region": ["ea"], "frequency": ["daily"]}),
        ),
        ("eight values", json!({"measure": eight})),
        ("32 bytes", json!({"measure": [long32]})),
        ("two values ascending", json!({"region": ["ea", "gb"]})),
    ];
    for (i, (what, facets)) in cases.into_iter().enumerate() {
        let lite = Lite::new(&format!("c2-ok-{i}"));
        let f = write(&lite, "ok.jsonl", &[base(), line_with(facets.clone())]);
        run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{what}: {e}"));
        let e = lite
            .store
            .catalog_entry_ext(CatalogKind::Function, F)
            .unwrap()
            .unwrap_or_else(|| panic!("{what}: no entry"));
        let got: Value = serde_json::to_value(&e.facets).unwrap();
        assert_eq!(got, facets, "{what}");
    }
}

/// What the bounds refuse, at the catalogue line (line 2), the whole file, nothing written.
#[test]
fn c2_facets_out_of_bounds_are_refused() {
    let nine = ["a1", "b2", "c3", "d4", "e5", "f6", "g7", "h8", "i9"];
    let cases: Vec<(&str, Value)> = vec![
        ("an unknown key", json!({"colour": ["red"]})),
        ("a key in capitals", json!({"Domain": ["finance"]})),
        ("an unknown key beside a good one", json!({"domain": ["finance"], "unit": ["x"]})),
        ("an empty list", json!({"domain": []})),
        ("nine values", json!({"measure": nine})),
        ("values not ascending", json!({"region": ["gb", "ea"]})),
        ("a duplicated value", json!({"region": ["ea", "ea"]})),
        ("33 bytes", json!({"measure": ["a".repeat(33)]})),
        ("capitals in a value", json!({"domain": ["Finance"]})),
        ("a space in a value", json!({"domain": ["a b"]})),
        ("a slash in a value", json!({"domain": ["a/b"]})),
        ("an empty value", json!({"domain": [""]})),
        ("a string instead of a list", json!({"domain": "finance"})),
        ("a number in a list", json!({"domain": [1]})),
        ("facets as a list", json!(["finance"])),
        ("facets as a string", json!("finance")),
    ];
    for (i, (what, facets)) in cases.into_iter().enumerate() {
        let lite = Lite::new(&format!("c2-bad-{i}"));
        let e = refused_whole(&lite, "bad.jsonl", &[base(), line_with(facets)], 2, NOW);
        eprintln!("{what}: {e}");
        assert!(
            lite.store.catalog_entry_ext(CatalogKind::Function, F).unwrap().is_none(),
            "{what}: the entry was written"
        );
    }
}

// ---- the store (SQLite) ---------------------------------------------------------------------------------------------------

/// The store keeps the facets: `catalog_entry_ext` reads them, `catalog_entries_with_facets` lists the entries that have some,
/// with the instant their name was first bound; an entry without facets is not listed and has none. A second run writes
/// nothing; a changed facet is a new value; a line without facets removes them.
#[test]
fn c2_the_store_round_trip_in_sqlite() {
    let lite = Lite::new("c2-sqlite");
    let facets = json!({"domain": ["finance"], "region": ["ea", "gb"]});
    let lines = [base(), line_with(facets.clone()), fn_line(G), catalog_line_of(G, "function", json!({}))];
    let f = write(&lite, "a.jsonl", &lines);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    let e = lite.store.catalog_entry_ext(CatalogKind::Function, F).unwrap().unwrap();
    assert_eq!(serde_json::to_value(&e.facets).unwrap(), facets);
    let plain = lite.store.catalog_entry_ext(CatalogKind::Function, G).unwrap().unwrap();
    assert!(plain.facets.is_empty(), "an entry without facets has none: {:?}", plain.facets);
    let listed = lite.store.catalog_entries_with_facets().unwrap();
    assert_eq!(listed.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), [F], "only the entry with facets is listed");
    assert_eq!(listed[0].first_bound, lite.bound_tx(F), "first_bound is the instant the name was first bound");
    assert_eq!(listed[0].kind, CatalogKind::Function);
    // the same file again writes nothing
    let before = snapshot(&lite);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(snapshot(&lite), before, "a second run wrote something");
    // changed facets
    let g = write(&lite, "b.jsonl", &[line_with(json!({"domain": ["finance"], "region": ["ea"]}))]);
    run(&lite, &g, T2_CLOCK).unwrap_or_else(|e| panic!("{e}"));
    let e = lite.store.catalog_entry_ext(CatalogKind::Function, F).unwrap().unwrap();
    assert_eq!(serde_json::to_value(&e.facets).unwrap(), json!({"domain": ["finance"], "region": ["ea"]}));
    // a line without facets removes them
    let h = write(&lite, "c.jsonl", &[catalog_line_of(F, "function", json!({}))]);
    run(&lite, &h, T2_CLOCK).unwrap_or_else(|e| panic!("{e}"));
    assert!(lite.store.catalog_entry_ext(CatalogKind::Function, F).unwrap().unwrap().facets.is_empty());
    assert!(lite.store.catalog_entries_with_facets().unwrap().is_empty());
}

const T2_CLOCK: &str = "2026-11-15T12:00:02.000000Z";

// ---- the store (PostgreSQL) -----------------------------------------------------------------------------------------------

/// The same round trip on PostgreSQL (migration 0012).
#[test]
fn c2_the_store_round_trip_in_postgres() {
    let db = TestDb::migrated();
    let store = db.store(4);
    let key = key_for(HOST);
    store.put_namespace(HOST, &datumesh_http::keydoc::with_key(HOST, None, &key, K_FROM).unwrap(), K_FROM).unwrap();
    let dir = out_dir().join(format!("c2-pg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let facets = json!({"domain": ["finance"], "measure": ["interest_rate"], "region": ["ea"]});
    let path = dir.join("a.jsonl");
    std::fs::write(
        &path,
        [base(), line_with(facets.clone()), fn_line(G), catalog_line_of(G, "function", json!({}))]
            .iter()
            .map(|l| format!("{l}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let owned = vec![HOST.to_string()];
    bl(|| datumesh_http::publish::publish_stored_with(&store, &key, HOST, &owned, &path, &common::pub031::opts(NOW)))
        .unwrap_or_else(|e| panic!("{e}"));
    let e = bl(|| store.catalog_entry_ext(CatalogKind::Function, F)).unwrap().unwrap();
    assert_eq!(serde_json::to_value(&e.facets).unwrap(), facets);
    let plain = bl(|| store.catalog_entry_ext(CatalogKind::Function, G)).unwrap().unwrap();
    assert!(plain.facets.is_empty());
    let listed = bl(|| store.catalog_entries_with_facets()).unwrap();
    assert_eq!(listed.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), [F]);
    assert!(listed[0].first_bound.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- describe and find ----------------------------------------------------------------------------------------------------

/// The facets of the entries of `facets_world`.
fn f_function() -> Value {
    json!({"domain": ["finance"], "measure": ["interest_rate"], "region": ["ea", "gb"], "thing": ["rate"]})
}
fn f_class() -> Value {
    json!({"domain": ["shop"], "frequency": ["daily"], "measure": ["price"]})
}

/// A mesh with a function with facets (`F`), a function without (`G`), a class with (`shop/product`) and a class without
/// (`shop/crate`).
async fn facets_world(label: &str) -> Served {
    let lite = Lite::new(label);
    let mut a = shop_functions();
    a.push(fn_line(F));
    a.push(fn_line(G));
    crworld::publish(&lite, "f.jsonl", &a, BEFORE).unwrap_or_else(|e| panic!("functions: {e}"));
    let mut b = shop_classes(&lite);
    b.push(catalog_line_of(
        F,
        "function",
        json!({"label": "Zebrafish levels", "description": "Zebrafish water levels.", "facets": f_function()}),
    ));
    b.push(catalog_line_of(G, "function", json!({"label": "Narwhal levels", "description": "Narwhal water levels."})));
    b.push(catalog_line_of(
        &product(),
        "class",
        json!({"label": "Zebrafish products", "description": "Zebrafish products of the shop.", "facets": f_class()}),
    ));
    b.push(catalog_line_of(
        &crate_class(),
        "class",
        json!({"label": "Narwhal crates", "description": "Narwhal crates of the shop."}),
    ));
    crworld::publish(&lite, "c.jsonl", &b, NOW).unwrap_or_else(|e| panic!("classes and catalogue: {e}"));
    build_served(lite, None, |_| {}).await
}

fn describe_path(id: &str) -> String {
    format!("/describe/{}", call_path(id))
}

/// A function answer carries `facets` as the entry has them; an entry without them has no member.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2_describe_of_a_function_carries_its_facets() {
    let m = facets_world("c2-describe-fn").await;
    let a = ask(&m, &describe_path(F)).await;
    assert_eq!((a.status, a.kind()), (200, Some("function")), "{}", a.text);
    assert_eq!(a.v["facets"], f_function(), "{}", a.text);
    let g = ask(&m, &describe_path(G)).await;
    assert_eq!((g.status, g.kind()), (200, Some("function")), "{}", g.text);
    assert!(g.v.get("facets").is_none(), "an entry without facets carries none: {}", g.text);
}

/// A class answer likewise.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2_describe_of_a_class_carries_its_facets() {
    let m = facets_world("c2-describe-class").await;
    let a = ask(&m, &describe_path(&product())).await;
    assert_eq!((a.status, a.kind()), (200, Some("class")), "{}", a.text);
    assert_eq!(a.v["facets"], f_class(), "{}", a.text);
    let c = ask(&m, &describe_path(&crate_class())).await;
    assert_eq!((c.status, c.kind()), (200, Some("class")), "{}", c.text);
    assert!(c.v.get("facets").is_none(), "{}", c.text);
}

/// A `find` result carries `facets` when its entry has them, for a function and for a class.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2_find_results_carry_facets() {
    let m = facets_world("c2-find").await;
    let v = m.find("zebrafish").await;
    assert_eq!(result(&v, F)["facets"], f_function(), "{v}");
    assert_eq!(result(&v, &product())["facets"], f_class(), "{v}");
    let w = m.find("narwhal").await;
    for n in [G.to_string(), crate_class()] {
        assert!(result(&w, &n).get("facets").is_none(), "{n}: {w}");
    }
}

// ---- bundle 1 -------------------------------------------------------------------------------------------------------------

fn bundle_with(facets: Option<Value>) -> Vec<u8> {
    foreign_bundle(HOST, BNS, BF, facets)
}

fn import(lite: &Lite, bytes: &[u8]) -> Result<bundle::Imported, BundleError> {
    let owned: Vec<String> = Vec::new();
    bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(NOW), bytes)
}

fn nothing_written(lite: &Lite) {
    assert!(lite.store.resolve_name(BF, END).unwrap().is_none(), "{BF} is bound");
    assert!(lite.store.catalog_entry_ext(CatalogKind::Function, BF).unwrap().is_none(), "the entry is stored");
}

/// A bundle whose catalogue member has facets imports; the store has them, `describe` and `find` carry them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2_a_bundle_with_facets_imports() {
    let lite = Lite::new("c2-bundle");
    let facets = json!({"domain": ["finance"], "region": ["ea", "gb"]});
    let i = import(&lite, &bundle_with(Some(facets.clone()))).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(i.functions_bound, 1);
    let e = lite.store.catalog_entry_ext(CatalogKind::Function, BF).unwrap().unwrap();
    assert_eq!(serde_json::to_value(&e.facets).unwrap(), facets);
    let listed = lite.store.catalog_entries_with_facets().unwrap();
    assert_eq!(listed.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), [BF]);
    let m = build_served(lite, None, |_| {}).await;
    let a = ask(&m, &describe_path(BF)).await;
    assert_eq!(a.v["facets"], facets, "{}", a.text);
    let v = m.find("foreign description").await;
    assert_eq!(result(&v, BF)["facets"], facets, "{v}");
    // a bundle without facets still imports, and its entry has none
    let lite = Lite::new("c2-bundle-plain");
    import(&lite, &bundle_with(None)).unwrap_or_else(|e| panic!("{e}"));
    assert!(lite.store.catalog_entry_ext(CatalogKind::Function, BF).unwrap().unwrap().facets.is_empty());
}

/// A bundle with `facets` written into its catalogue entry by hand and the manifest signed again by `HOST`'s key. The
/// bundle builder stays strict (coordinator decision), so a bad facet cannot be built: it is built valid, then changed
/// and re-signed, so that the import's refusal is what a test sees (the signature is good).
fn resigned_bundle_with_facets(facets: Value) -> Vec<u8> {
    use datumesh_core::json;
    use ed25519_dalek::Signer;
    let mut b: Value = serde_json::from_slice(&bundle_with(Some(json!({"domain": ["finance"]})))).unwrap();
    b["catalog"][0]["facets"] = facets;
    // the manifest signs: names, namespace, receipts (hashes) and catalog, as JCS
    let manifest = json!({"names": b["names"], "namespace": b["namespace"], "receipts": [], "catalog": b["catalog"]});
    assert_eq!(b["receipts"], json!([]), "the manifest above lists no receipts");
    let jcs = json::parse(manifest.to_string().as_bytes()).unwrap().to_jcs();
    let sig = ed25519_dalek::SigningKey::from_bytes(&seed_for(HOST)).sign(&jcs).to_bytes();
    b["manifest"]["signature"] = Value::String(sig.iter().map(|x| format!("{x:02x}")).collect());
    json::parse(b.to_string().as_bytes()).unwrap().to_jcs()
}

/// An unknown facet key, and every bound of the line, are refused in a bundle too; nothing is written. The bundle is
/// well signed (the control imports): it is the import that refuses the facets.
#[test]
fn c2_a_bundle_with_bad_facets_is_refused() {
    // control: re-signing by hand with good facets imports, so a refusal below is the facets' and not the signature's
    let control = Lite::new("c2-bundle-resigned-ok");
    import(&control, &resigned_bundle_with_facets(json!({"domain": ["finance"], "region": ["ea"]})))
        .unwrap_or_else(|e| panic!("the hand-signed control bundle must import: {e}"));
    let cases: Vec<(&str, Value)> = vec![
        ("an unknown key", json!({"colour": ["red"]})),
        ("an empty list", json!({"domain": []})),
        ("nine values", json!({"measure": ["a1", "b2", "c3", "d4", "e5", "f6", "g7", "h8", "i9"]})),
        ("not ascending", json!({"region": ["gb", "ea"]})),
        ("33 bytes", json!({"measure": ["a".repeat(33)]})),
        ("capitals", json!({"domain": ["Finance"]})),
    ];
    for (what, facets) in cases {
        let lite = Lite::new("c2-bundle-bad");
        let e = import(&lite, &resigned_bundle_with_facets(facets)).expect_err(what);
        assert!(!matches!(e, BundleError::Manifest(_)), "{what}: refused for its signature, not its facets: {e:?}");
        eprintln!("{what}: {e}");
        nothing_written(&lite);
    }
}

/// The facets are signed with the bundle: a changed facet in the bundle as sent fails the import (`Manifest`, `E_SIGNATURE`),
/// and so does a bundle whose facets were dropped; nothing is written.
#[test]
fn c2_facets_are_covered_by_the_bundle_signature() {
    let good = bundle_with(Some(json!({"region": ["ea"]})));
    let text = String::from_utf8(good.clone()).unwrap();
    assert!(text.contains(r#""region":["ea"]"#), "{text}");
    let tampered = text.replace(r#""region":["ea"]"#, r#""region":["gb"]"#).into_bytes();
    let lite = Lite::new("c2-bundle-tamper");
    let e = import(&lite, &tampered).unwrap_err();
    assert!(matches!(e, BundleError::Manifest(_)), "{e:?}");
    nothing_written(&lite);
    let dropped = text.replace(r#""facets":{"region":["ea"]},"#, "").into_bytes();
    assert_ne!(dropped, good, "the facets were not found to drop: {text}");
    let lite = Lite::new("c2-bundle-dropped");
    let e = import(&lite, &dropped).unwrap_err();
    assert!(matches!(e, BundleError::Manifest(_)), "{e:?}");
    nothing_written(&lite);
}

/// Facets are outside every hash: the function's hash and the class's are what they were without facets.
#[test]
fn c2_facets_are_outside_every_hash() {
    let (a, b) = (Lite::new("c2-hash-a"), Lite::new("c2-hash-b"));
    run(&a, &write(&a, "a.jsonl", &[base(), catalog_line_of(F, "function", json!({}))]), NOW)
        .unwrap_or_else(|e| panic!("{e}"));
    run(&b, &write(&b, "a.jsonl", &[base(), line_with(f_function())]), NOW).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(fn_hash(&a, F), fn_hash(&b, F));
    let _ = (function_line, BEFORE);
}
