//! http 0.4.9, CONTRACT-0.4.9 H8 (decision D43): reserved paths under the mesh's own host.
//!
//! Under `DATUMESH_HOST` a name whose first segment after the host is `series`, `fn`, `takeover` or `x` is
//! refused at every write path with `E_NAME_RESERVED` (422 for `POST /bundles`; the file or bundle refused
//! whole, nothing written, file and line given). A name under `key/` is accepted only as a key-derived name
//! `key/k<24 lowercase hex>/..` and only from the key it is derived from (the 0.4.8 rule); any other `key/..`
//! name is `E_NAME_RESERVED`, the mesh's own key included. Names of other hosts are not reserved.
//!
//! Write paths covered: `publish-stored`, `import-signed`, `POST /bundles` and `declare`. (`import-vectors`
//! and a bind line share the name check of `publish-stored`; they are not repeated here.)
//!
//! How the error is read: `PublishError::detail` and a `BundleError`'s text contain `E_NAME_RESERVED` (as
//! `E_TESTS_MISSING` is in the detail of a publish error); the declare error's text likewise; POST /bundles
//! answers `{"error": "E_NAME_RESERVED", "detail": ..}` with status 422.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use common::lite::*;
use common::pub031::{function_line, rate_sig, refused_whole, run, snapshot, unbound, write};
use common::readpath::*;
use common::*;
use datumesh_core::lexical::to_hex;
use datumesh_core::{sha256_hex, Cell, ReceiptBuilder, SigningKey};
use datumesh_http::{bundle, keydoc, Config};
use datumesh_store::Store;
use publish4::*;
use serde_json::json;

const SEED: [u8; 32] = [41; 32];
const CODE: &str = "E_NAME_RESERVED";

fn hex24(seed: &[u8; 32]) -> String {
    sha256_hex(&SigningKey::from_seed(seed).public_key())[..24].to_string()
}

fn key_ns(seed: &[u8; 32]) -> String {
    format!("{HOST}/key/k{}", hex24(seed))
}

/// The names that are reserved under the host.
fn reserved() -> Vec<String> {
    vec![
        format!("{HOST}/series/a"),
        format!("{HOST}/fn/a"),
        format!("{HOST}/takeover/a"),
        format!("{HOST}/x/a"),
        format!("{HOST}/series/deep/er/a"),
        format!("{HOST}/key/foo"),
        format!("{HOST}/key/foo/f"),
        format!("{HOST}/key/k{}/f", &hex24(&SEED)[..23]), // 23 digits: not a key namespace
        format!("{HOST}/key/k{}/f", hex24(&SEED).to_uppercase()),
        format!("{HOST}/key/k{}/f", hex24(&SEED)), // the mesh's own key may not publish here
    ]
}

/// Names that are fine.
fn fine() -> Vec<String> {
    vec![
        format!("{HOST}/seriesx/a"),
        format!("{HOST}/series_a/b"),
        format!("{HOST}/series_a"),
        format!("{HOST}/fnord/a"),
        format!("{HOST}/h049/series/a"), // only the first segment after the host counts
        format!("{HOST}/h049/key/a"),
    ]
}

fn mesh_key_bundle(ns: &str, name: &str) -> Vec<u8> {
    bundle::build_with_catalog(
        ns,
        vec![stored_fn().canonical()],
        vec![(name.to_string(), stored_fn().hash())],
        vec![],
        vec![],
        &seed_for(HOST),
    )
    .unwrap()
}

/// The bundle of `name` (a stored function) under `ns`, signed by `seed`.
fn bundle_of(ns: &str, name: &str, seed: &[u8; 32]) -> Vec<u8> {
    bundle::build_with_catalog(
        ns,
        vec![stored_fn().canonical()],
        vec![(name.to_string(), stored_fn().hash())],
        vec![],
        vec![],
        seed,
    )
    .unwrap()
}

fn split(name: &str) -> (String, String) {
    let (ns, f) = name.rsplit_once('/').unwrap();
    (ns.to_string(), f.to_string())
}

// ---- publish-stored ----------------------------------------------------------------------------

#[test]
fn publish_stored_refuses_a_reserved_name_whole() {
    for name in reserved() {
        let lite = Lite::new("h8-publish");
        let ok = format!("{HOST}/h049/ok/level");
        // the reserved name on line 2: the file is refused whole, the good function of line 1 is not written
        let lines = [function_line(&ok, &rate_sig(), None), function_line(&name, &rate_sig(), None)];
        let e = refused_whole(&lite, "f.jsonl", &lines, 2, NOW);
        assert!(e.detail.contains(CODE), "{name}: {e}");
        assert!(unbound(&lite, &ok), "{name}: the good function was written");
        assert!(unbound(&lite, &name));
    }
}

#[test]
fn publish_stored_refuses_a_cell_line_of_a_reserved_name() {
    let lite = Lite::new("h8-publish-cell");
    let name = format!("{HOST}/series/a");
    let lines = [common::pub031::rate_cell(&name, "1", None, None, None)];
    let f = write(&lite, "f.jsonl", &lines);
    let before = snapshot(&lite);
    let e = run(&lite, &f, NOW).expect_err("refused");
    assert_eq!(e.line, 1, "{e}");
    assert!(e.detail.contains(CODE), "{e}");
    assert_eq!(snapshot(&lite), before);
}

#[test]
fn publish_stored_takes_names_that_only_look_alike() {
    for name in fine() {
        let lite = Lite::new("h8-publish-fine");
        let f = write(&lite, "f.jsonl", &[function_line(&name, &rate_sig(), None)]);
        run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{name} is not reserved: {e}"));
        assert!(!unbound(&lite, &name), "{name}");
    }
}

#[test]
fn the_names_of_other_hosts_are_not_reserved() {
    let lite = Lite::new("h8-publish-other");
    let name = "other.example/series/a";
    let f = write(&lite, "f.jsonl", &[function_line(name, &rate_sig(), None)]);
    // refused or not for other reasons (the mesh does not own that host), never as reserved
    if let Err(e) = run(&lite, &f, NOW) {
        assert!(!e.detail.contains(CODE), "{e}");
    }
}

// ---- import-signed -----------------------------------------------------------------------------

fn import(lite: &Lite, bytes: &[u8]) -> Result<bundle::Imported, bundle::BundleError> {
    let owned: Vec<String> = Vec::new();
    bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(NOW), bytes)
}

#[test]
fn import_signed_refuses_a_reserved_name_whole() {
    for name in reserved() {
        let (ns, _) = split(&name);
        // a bundle can only be built for a well-formed namespace; a malformed one is refused earlier, for another reason
        let Ok(b) = std::panic::catch_unwind(|| mesh_key_bundle(&ns, &name)) else { continue };
        let lite = Lite::new("h8-import");
        let before = snapshot(&lite);
        let e = import(&lite, &b).expect_err(&format!("{name} must be refused"));
        assert!(format!("{e}").contains(CODE), "{name}: {e}");
        assert!(lite.store.resolve_name(&name, END).unwrap().is_none(), "{name} is bound");
        assert!(lite.store.descriptor(&stored_fn().hash()).is_none(), "{name}: the descriptor is stored");
        assert_eq!(snapshot(&lite), before, "{name}: wrote something");
    }
}

#[test]
fn import_signed_still_takes_ordinary_names() {
    // (a bundle's namespace needs a segment after the host, so `<host>/series_a` is not a bundle name)
    for name in fine().into_iter().filter(|n| n.matches('/').count() >= 2) {
        let (ns, _) = split(&name);
        let lite = Lite::new("h8-import-fine");
        let b = mesh_key_bundle(&ns, &name);
        import(&lite, &b).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(lite.store.resolve_name(&name, END).unwrap().is_some(), "{name}");
    }
}

// ---- declare -----------------------------------------------------------------------------------

#[test]
fn declare_refuses_a_reserved_name_and_changes_nothing() {
    let lite = Lite::new("h8-declare");
    let before = lite.store.namespace(HOST).unwrap().unwrap().keydoc;
    for name in reserved() {
        let e = keydoc::declare_at(&*lite.store, HOST, &name, "CC0-1.0", None, None, &FixedClock(NOW))
            .expect_err(&format!("{name} must be refused"));
        assert!(e.to_string().contains(CODE), "{name}: {e}");
        assert_eq!(lite.store.namespace(HOST).unwrap().unwrap().keydoc, before, "{name}: the key document changed");
    }
    // an ordinary name is declared
    let ok = format!("{HOST}/seriesx/a");
    keydoc::declare_at(&*lite.store, HOST, &ok, "CC0-1.0", None, None, &FixedClock(NOW)).unwrap_or_else(|e| panic!("{e}"));
}

// ---- POST /bundles -----------------------------------------------------------------------------

const KEY_SEED: [u8; 32] = [41; 32];

fn config(log: &std::path::Path) -> Config {
    let env = vec![
        ("DATUMESH_HOST".to_string(), HOST.to_string()),
        ("DATUMESH_KEY_NAMES".to_string(), "1".to_string()),
        ("DATUMESH_LOG_DIR".to_string(), log.to_string_lossy().into_owned()),
    ];
    Config::from_vars(move |k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).unwrap()
}

async fn mesh(label: &str) -> Public {
    let lite = Lite::new(label);
    let log_dir = lite.path("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let m = build_served(lite, Some(config(&log_dir)), |_| {}).await;
    Public { m, log_dir }
}

fn sign(seed: &[u8; 32], message: &[u8]) -> String {
    use ed25519_dalek::Signer;
    to_hex(&ed25519_dalek::SigningKey::from_bytes(seed).sign(message).to_bytes())
}

async fn register(p: &Public, seed: &[u8; 32]) -> Reply4 {
    let c = p.get("/x/keys/challenge", &[]).await.json()["challenge"].as_str().unwrap().to_string();
    let body = json!({"public_key": to_hex(&SigningKey::from_seed(seed).public_key()), "challenge": c,
        "signature": sign(seed, c.as_bytes())});
    let r = p
        .m
        .http
        .post(format!("{}/x/keys", p.m.base))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    let (status, headers) = (r.status().as_u16(), r.headers().clone());
    Reply4 { status, headers, body: r.bytes().await.unwrap().to_vec() }
}

fn key_receipt(ns: &str, seed: &[u8; 32]) -> Vec<u8> {
    let cell = Cell {
        ns: ns.into(),
        fn_name: "level".into(),
        fn_hash: stored_fn().hash(),
        args: common::args(&[("site", common::s("N"))]),
    };
    let r = ReceiptBuilder::new()
        .cell(cell)
        .value(common::n("42"))
        .valid(common::args(&[]))
        .asof(MESH_NOW)
        .tx(MESH_NOW)
        .issued(MESH_NOW)
        .publisher(ns)
        .signer_host(HOST)
        .licence("CC0-1.0")
        .summary("test")
        .sign(&SigningKey::from_seed(seed))
        .unwrap();
    common::receipt_json(&r)
}

#[track_caller]
fn reserved_refusal(r: &Reply4, what: &str) {
    assert_eq!(r.status, 422, "{what}: {}", r.text());
    let j = r.json();
    assert_eq!(j["error"], CODE, "{what}: {}", r.text());
    assert!(j.get("detail").is_some(), "{what}: no detail: {}", r.text());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn post_bundles_refuses_a_reserved_name_whole() {
    let p = mesh("h8-post").await;
    for name in reserved() {
        let (ns, _) = split(&name);
        let Ok(b) = std::panic::catch_unwind(|| mesh_key_bundle(&ns, &name)) else { continue };
        let r = p.post(b).await;
        reserved_refusal(&r, &name);
        assert!(p.m.lite.store.resolve_name(&name, END).unwrap().is_none(), "{name} is bound");
        assert!(p.m.lite.store.descriptor(&stored_fn().hash()).is_none(), "{name}: the descriptor is stored");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn post_bundles_takes_names_that_only_look_alike() {
    let p = mesh("h8-post-fine").await;
    let name = format!("{HOST}/seriesx/a");
    let (ns, _) = split(&name);
    let r = p.post(mesh_key_bundle(&ns, &name)).await;
    assert_eq!(r.status, 201, "{}", r.text());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_key_derived_name_is_accepted_from_its_own_key_only() {
    let p = mesh("h8-key").await;
    assert_eq!(register(&p, &KEY_SEED).await.status, 201);
    let ns = key_ns(&KEY_SEED);
    // by its own registered key: accepted (the 0.4.8 path)
    let b = bundle::build_with_catalog(
        &ns,
        vec![stored_fn().canonical()],
        vec![(format!("{ns}/level"), stored_fn().hash())],
        vec![key_receipt(&ns, &KEY_SEED)],
        vec![],
        &KEY_SEED,
    )
    .unwrap();
    let r = p.post(b).await;
    assert_eq!(r.status, 201, "{}", r.text());
    assert!(p.m.lite.store.resolve_name(&format!("{ns}/level"), END).unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mesh_own_key_may_not_publish_under_a_key_namespace() {
    let p = mesh("h8-key-mesh").await;
    assert_eq!(register(&p, &KEY_SEED).await.status, 201);
    let ns = key_ns(&KEY_SEED);
    let r = p.post(mesh_key_bundle(&ns, &format!("{ns}/level"))).await;
    reserved_refusal(&r, "the mesh's own key on a key namespace");
    assert!(p.m.lite.store.resolve_name(&format!("{ns}/level"), END).unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_registered_key_may_not_publish_a_key_name_that_is_not_its_own_derived_one() {
    let p = mesh("h8-key-foo").await;
    assert_eq!(register(&p, &KEY_SEED).await.status, 201);
    let ns = format!("{HOST}/key/foo");
    let r = p.post(bundle_of(&ns, &format!("{ns}/level"), &KEY_SEED)).await;
    reserved_refusal(&r, "key/foo by a registered key");
    assert!(p.m.lite.store.resolve_name(&format!("{ns}/level"), END).unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn another_hosts_series_name_is_not_refused_as_reserved() {
    let p = mesh("h8-other").await;
    let ns = "other.example/series";
    let b = bundle_of(ns, "other.example/series/a", &seed_for("other.example"));
    let r = p.post(b).await;
    // not allowed for some other reason (the mesh does not take that domain), never `E_NAME_RESERVED`
    assert_ne!(r.json()["error"], CODE, "{}", r.text());
}
