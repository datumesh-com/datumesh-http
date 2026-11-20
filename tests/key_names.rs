//! Build 2, QUESTIONS-build2 D39 (DESIGN-build2-hosted-names, option B), http 0.4.8: key-derived
//! hosted names. These tests are written first and FAIL until the feature exists.
//!
//! # What these tests fix (the implementer follows)
//!
//! * `DATUMESH_KEY_NAMES=1` turns the feature on; unset, empty, `0` or anything else but `1` is off.
//!   Off: `GET /x/keys/challenge` and `POST /x/keys` are `404`, and the mesh document has no
//!   `endpoints.x_keys`. On: the mesh document lists `endpoints.x_keys` = `https://<host>/x/keys`.
//! * `GET /x/keys/challenge`: `200`, `Cache-Control: no-store`,
//!   `{"challenge": "<opaque string>", "expires": "<instant, microsecond Z form>"}`. A challenge is valid
//!   for `DATUMESH_KEY_CHALLENGE_TTL_SECS` seconds of the mesh clock (default 300; a test knob, `0` is
//!   expired as soon as it is issued: valid while `now < expires`), and for one successful `POST` only.
//!   Fetching challenges costs nothing from the `/x/keys` budget (only the per-IP limit applies).
//! * `POST /x/keys`, `{"public_key": "<64 lower-case hex digits>", "challenge": "<as issued>",
//!   "signature": "<128 hex digits>"}`; the signature is Ed25519 over the UTF-8 bytes of the challenge string.
//!   `201` `{"namespace", "key_id"}` the first time and `200` with the same body when the key is already
//!   registered (idempotent; the key document is not touched again). The namespace is
//!   `<host>/key/k<first 24 hex digits of SHA-256 over the 32 raw public key bytes>`; `key_id` is the key id
//!   of 9.1. The key is added to the host's key document (`/.well-known/datumesh.json`, valid from the
//!   registration instant of the mesh clock), which stays valid for core.
//! * Refusals are `{"error": <code>, "detail": ...}`: `E_KEY_MALFORMED` (400: not JSON, a missing member, a
//!   public key that is not 64 hex digits, a signature that is not 128), `E_CHALLENGE` (400: unknown, expired
//!   or already used), `E_KEY_SIGNATURE` (403: the signature does not verify), and `429 rate_limited` with
//!   `Retry-After` from the budget `DATUMESH_RATE_KEYS_PER_IP` (default 5 a minute, every `POST` counts).
//! * `POST /bundles` signed by a registered key is accepted only when the manifest namespace is that key's
//!   own namespace (every name under `<ns>/`); any other namespace is refused (403 or 422) with
//!   `E_DOMAIN_NOT_ALLOWED` or `E_NOT_OWNED`, nothing written. A name `<host>/key/k<23 hex>/..` (wrong
//!   length), or with upper-case digits, is not a key namespace. Bundles of the mesh's own key and of
//!   allowed domains are unchanged. Receipts the key signed (published under its namespace, `signer_host`
//!   the mesh host) verify with core against the key document: the cell is `trust: primary`.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use common::lite::*;
use common::readpath::*;
use datumesh_core::lexical::to_hex;
use datumesh_core::{sha256_hex, Cell, ReceiptBuilder, SigningKey};
use datumesh_http::{bundle, Config};
use datumesh_store::Store;
use publish4::*;
use serde_json::{json, Value};

const SEED: [u8; 32] = [41; 32];
const SEED2: [u8; 32] = [42; 32];

fn config(vars: &[(&str, &str)], log: &std::path::Path) -> Config {
    let mut env: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    env.push(("DATUMESH_HOST".into(), HOST.into()));
    env.push(("DATUMESH_LOG_DIR".into(), log.to_string_lossy().into_owned()));
    Config::from_vars(move |k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).unwrap()
}

/// The mesh with `vars` as its environment (`DATUMESH_KEY_NAMES=1` unless given).
async fn mesh(label: &str, vars: &[(&str, &str)]) -> Public {
    let lite = Lite::new(label);
    let log_dir = lite.path("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let mut all = vec![("DATUMESH_KEY_NAMES", "1")];
    all.extend_from_slice(vars);
    // later entries win: Config::from_vars looks for the first, so drop the default when overridden
    let all: Vec<(&str, &str)> = all
        .iter()
        .enumerate()
        .filter(|(i, (k, _))| !all[i + 1..].iter().any(|(k2, _)| k2 == k))
        .map(|(_, kv)| *kv)
        .collect();
    let cfg = config(&all, &log_dir);
    let m = build_served(lite, Some(cfg), |_| {}).await;
    Public { m, log_dir }
}

fn sign(seed: &[u8; 32], message: &[u8]) -> String {
    use ed25519_dalek::Signer;
    to_hex(&ed25519_dalek::SigningKey::from_bytes(seed).sign(message).to_bytes())
}

fn public_hex(seed: &[u8; 32]) -> String {
    to_hex(&SigningKey::from_seed(seed).public_key())
}

fn hex24(seed: &[u8; 32]) -> String {
    sha256_hex(&SigningKey::from_seed(seed).public_key())[..24].to_string()
}

fn ns_of_key(seed: &[u8; 32]) -> String {
    format!("{HOST}/key/k{}", hex24(seed))
}

async fn post_keys(p: &Public, body: Value) -> Reply4 {
    post_raw(p, body.to_string().into_bytes()).await
}

async fn post_raw(p: &Public, body: Vec<u8>) -> Reply4 {
    let r =
        p.m.http
            .post(format!("{}/x/keys", p.m.base))
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
    let (status, headers) = (r.status().as_u16(), r.headers().clone());
    Reply4 { status, headers, body: r.bytes().await.unwrap().to_vec() }
}

async fn challenge(p: &Public) -> String {
    let r = p.get("/x/keys/challenge", &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()["challenge"].as_str().unwrap_or_else(|| panic!("no challenge: {}", r.text())).to_string()
}

/// A full registration of the key of `seed`: a fresh challenge, signed by the key.
async fn register(p: &Public, seed: &[u8; 32]) -> Reply4 {
    let c = challenge(p).await;
    post_keys(p, json!({"public_key": public_hex(seed), "challenge": c, "signature": sign(seed, c.as_bytes())})).await
}

fn key_ids_of(doc: &[u8]) -> Vec<String> {
    let keys =
        datumesh_core::keys_from_response(HOST, 200, Some(doc)).unwrap_or_else(|e| panic!("not a key document: {e}"));
    keys.key_ids().iter().map(|s| s.to_string()).collect()
}

async fn key_doc_bytes(p: &Public) -> Vec<u8> {
    let r = p.get("/.well-known/datumesh.json", &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    r.body
}

fn code(r: &Reply4) -> String {
    r.json()["error"].as_str().unwrap_or_else(|| panic!("no error code: {}", r.text())).to_string()
}

// ---- off by default ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_feature_is_off_by_default() {
    for (label, vars) in [
        ("kn-unset", vec![("DATUMESH_KEY_NAMES", "")]),
        ("kn-zero", vec![("DATUMESH_KEY_NAMES", "0")]),
        ("kn-yes", vec![("DATUMESH_KEY_NAMES", "yes")]),
    ] {
        let p = mesh(label, &vars).await;
        assert_eq!(p.get("/x/keys/challenge", &[]).await.status, 404, "{label}");
        let r = post_keys(&p, json!({"public_key": public_hex(&SEED), "challenge": "x", "signature": "y"})).await;
        assert_eq!(r.status, 404, "{label}: {}", r.text());
        let md = p.get("/.well-known/datumesh-mesh.json", &[]).await.json();
        assert!(md["endpoints"].get("x_keys").is_none(), "{label}: {md}");
        assert_eq!(key_ids_of(&key_doc_bytes(&p).await).len(), 1, "{label}: the key document is the mesh's own");
    }
    // a mesh built with no variable at all is off as well
    let lite = Lite::new("kn-none");
    let m = build_served(lite, None, |_| {}).await;
    assert_eq!(get(&m, "/x/keys/challenge", &[]).await.status, 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mesh_document_lists_the_endpoint_when_on() {
    let p = mesh("kn-md", &[]).await;
    let md = p.get("/.well-known/datumesh-mesh.json", &[]).await.json();
    assert_eq!(md["endpoints"]["x_keys"], format!("https://{HOST}/x/keys"), "{md}");
}

// ---- the challenge ------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_challenge_has_a_nonce_and_a_short_expiry() {
    let p = mesh("kn-challenge", &[]).await;
    let r = p.get("/x/keys/challenge", &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
    let j = r.json();
    let c = j["challenge"].as_str().unwrap_or_else(|| panic!("{j}"));
    assert!(c.len() >= 16, "an unguessable nonce: {j}");
    let expires = j["expires"].as_str().unwrap_or_else(|| panic!("{j}"));
    assert!(expires > MESH_NOW && expires < "2026-11-16T00:15:00.000000Z", "short-lived: {j}");
    assert_eq!(expires.len(), MESH_NOW.len(), "{j}");
    assert_ne!(challenge(&p).await, challenge(&p).await, "every challenge is new");
}

// ---- registering --------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_valid_proof_adds_the_key_and_gives_its_namespace() {
    let p = mesh("kn-register", &[]).await;
    let r = register(&p, &SEED).await;
    assert_eq!(r.status, 201, "{}", r.text());
    let j = r.json();
    assert_eq!(j["namespace"], ns_of_key(&SEED), "{j}");
    assert_eq!(j["key_id"], SigningKey::from_seed(&SEED).key_id(), "{j}");
    assert_eq!(j.as_object().unwrap().len(), 2, "{j}");
    assert!(ns_of_key(&SEED)
        .strip_prefix(&format!("{HOST}/key/k"))
        .unwrap()
        .starts_with(j["key_id"].as_str().unwrap()));
    // the key is in the served key document, which core accepts, beside the mesh's own key
    let doc = key_doc_bytes(&p).await;
    let ids = key_ids_of(&doc);
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert!(ids.contains(&SigningKey::from_seed(&SEED).key_id().to_string()), "{ids:?}");
    assert!(ids.contains(&p.m.lite.key.key_id().to_string()), "{ids:?}");
    let v: Value = serde_json::from_slice(&doc).unwrap();
    let entry = v["keys"].as_array().unwrap().iter().find(|k| k["id"] == j["key_id"]).unwrap();
    assert_eq!(entry["public"], public_hex(&SEED), "{v}");
    assert_eq!(entry["valid_from"], MESH_NOW, "valid from the registration: {v}");
    // the store holds the same document
    assert_eq!(key_ids_of(&p.m.lite.store.namespace(HOST).unwrap().unwrap().keydoc).len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn registering_again_is_idempotent() {
    let p = mesh("kn-again", &[]).await;
    let first = register(&p, &SEED).await;
    assert_eq!(first.status, 201, "{}", first.text());
    let doc = key_doc_bytes(&p).await;
    let again = register(&p, &SEED).await;
    assert_eq!(again.status, 200, "{}", again.text());
    assert_eq!(again.json(), first.json());
    assert_eq!(key_doc_bytes(&p).await, doc, "the key document is not touched");
    // another key is another namespace, and both are in the document
    let other = register(&p, &SEED2).await;
    assert_eq!(other.status, 201, "{}", other.text());
    assert_ne!(other.json()["namespace"], first.json()["namespace"]);
    assert_eq!(key_ids_of(&key_doc_bytes(&p).await).len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refusals_have_codes_and_change_nothing() {
    // A budget high enough for every refusal to get its code; the budget itself is tested below.
    let p = mesh("kn-refuse", &[("DATUMESH_RATE_KEYS_PER_IP", "1000")]).await;
    let before = key_doc_bytes(&p).await;
    let pk = public_hex(&SEED);

    // a signature of another message, and one of another key
    let c = challenge(&p).await;
    let r = post_keys(&p, json!({"public_key": pk, "challenge": c, "signature": sign(&SEED, b"something else")})).await;
    assert_eq!((r.status, code(&r)), (403, "E_KEY_SIGNATURE".to_string()), "{}", r.text());
    assert!(r.json().get("detail").is_some());
    let c = challenge(&p).await;
    let r = post_keys(&p, json!({"public_key": pk, "challenge": c, "signature": sign(&SEED2, c.as_bytes())})).await;
    assert_eq!((r.status, code(&r)), (403, "E_KEY_SIGNATURE".to_string()), "{}", r.text());

    // an unknown challenge
    let r = post_keys(
        &p,
        json!({"public_key": pk, "challenge": "never-issued", "signature": sign(&SEED, b"never-issued")}),
    )
    .await;
    assert_eq!((r.status, code(&r)), (400, "E_CHALLENGE".to_string()), "{}", r.text());

    // a challenge is good once
    let c = challenge(&p).await;
    let body = json!({"public_key": public_hex(&SEED), "challenge": c, "signature": sign(&SEED, c.as_bytes())});
    assert_eq!(post_keys(&p, body.clone()).await.status, 201);
    let r = post_keys(&p, body).await;
    assert_eq!((r.status, code(&r)), (400, "E_CHALLENGE".to_string()), "reused: {}", r.text());
    // a refusal leaves the key document as the one success made it: one key more
    assert_eq!(key_ids_of(&key_doc_bytes(&p).await).len(), key_ids_of(&before).len() + 1);

    // malformed keys, signatures and bodies
    let c = challenge(&p).await;
    let good_sig = sign(&SEED2, c.as_bytes());
    for (what, body) in [
        ("not hex", json!({"public_key": "zz".repeat(32), "challenge": c, "signature": good_sig})),
        ("31 bytes", json!({"public_key": public_hex(&SEED2)[..62], "challenge": c, "signature": good_sig})),
        ("33 bytes", json!({"public_key": format!("{}00", public_hex(&SEED2)), "challenge": c, "signature": good_sig})),
        ("empty", json!({"public_key": "", "challenge": c, "signature": good_sig})),
        ("a number", json!({"public_key": 7, "challenge": c, "signature": good_sig})),
        ("no key", json!({"challenge": c, "signature": good_sig})),
        ("no challenge", json!({"public_key": public_hex(&SEED2), "signature": good_sig})),
        ("no signature", json!({"public_key": public_hex(&SEED2), "challenge": c})),
        ("short signature", json!({"public_key": public_hex(&SEED2), "challenge": c, "signature": "ab"})),
    ] {
        let r = post_keys(&p, body).await;
        assert_eq!((r.status, code(&r)), (400, "E_KEY_MALFORMED".to_string()), "{what}: {}", r.text());
    }
    for body in [b"not json".to_vec(), b"[]".to_vec(), Vec::new()] {
        let r = post_raw(&p, body).await;
        assert_eq!((r.status, code(&r)), (400, "E_KEY_MALFORMED".to_string()), "{}", r.text());
    }
    // none of the refusals registered SEED2
    assert_eq!(key_ids_of(&key_doc_bytes(&p).await).len(), key_ids_of(&before).len() + 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_expired_challenge_is_refused() {
    let p = mesh("kn-expired", &[("DATUMESH_KEY_CHALLENGE_TTL_SECS", "0")]).await;
    let r = register(&p, &SEED).await;
    assert_eq!((r.status, code(&r)), (400, "E_CHALLENGE".to_string()), "{}", r.text());
    assert_eq!(key_ids_of(&key_doc_bytes(&p).await).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn registering_has_a_budget_of_its_own() {
    // default 5 a minute per IP: every POST counts, whatever its outcome; challenges do not
    let p = mesh("kn-rate", &[]).await;
    for _ in 0..8 {
        assert_eq!(p.get("/x/keys/challenge", &[]).await.status, 200);
    }
    for i in 0..5 {
        let r = post_raw(&p, b"{}".to_vec()).await;
        assert_eq!(r.status, 400, "{i}: {}", r.text());
    }
    let r = post_raw(&p, b"{}".to_vec()).await;
    assert_eq!(r.status, 429, "{}", r.text());
    assert_eq!(code(&r), "rate_limited");
    assert!(r.header("retry-after").is_some());
    // a good proof is refused as well while the budget is spent
    assert_eq!(register(&p, &SEED).await.status, 429);
    // the rest of the mesh is not affected
    assert_eq!(p.get("/.well-known/datumesh.json", &[]).await.status, 200);

    // the budget is a setting
    let p = mesh("kn-rate2", &[("DATUMESH_RATE_KEYS_PER_IP", "2")]).await;
    assert_eq!(post_raw(&p, b"{}".to_vec()).await.status, 400);
    assert_eq!(post_raw(&p, b"{}".to_vec()).await.status, 400);
    assert_eq!(post_raw(&p, b"{}".to_vec()).await.status, 429);
}

// ---- publishing under the key's namespace ------------------------------------------------------

fn key_receipt(ns: &str, seed: &[u8; 32], site: &str, value: &str) -> Vec<u8> {
    let cell = Cell {
        ns: ns.into(),
        fn_name: "level".into(),
        fn_hash: stored_fn().hash(),
        args: common::args(&[("site", common::s(site))]),
    };
    let key = SigningKey::from_seed(seed);
    let r = ReceiptBuilder::new()
        .cell(cell)
        .value(common::n(value))
        .valid(common::args(&[]))
        .asof(MESH_NOW)
        .tx(MESH_NOW)
        .issued(MESH_NOW)
        .publisher(ns)
        .signer_host(HOST)
        .licence("CC0-1.0")
        .summary("test")
        .sign(&key)
        .unwrap();
    common::receipt_json(&r)
}

/// A bundle of `<ns>/level` signed by the key of `seed`, with a receipt of it.
fn key_bundle(ns: &str, seed: &[u8; 32]) -> Vec<u8> {
    bundle::build_with_catalog(
        ns,
        vec![stored_fn().canonical()],
        vec![(format!("{ns}/level"), stored_fn().hash())],
        vec![key_receipt(ns, seed, "N", "42")],
        vec![],
        seed,
    )
    .unwrap()
}

fn nothing_under(p: &Public, ns: &str) {
    assert!(p.m.lite.store.resolve_name(&format!("{ns}/level"), END).unwrap().is_none(), "{ns}/level is bound");
}

#[track_caller]
fn not_allowed(r: &Reply4) {
    assert!([403, 422].contains(&r.status), "status {}: {}", r.status, r.text());
    let c = code(r);
    assert!(["E_DOMAIN_NOT_ALLOWED", "E_NOT_OWNED"].contains(&c.as_str()), "{c}: {}", r.text());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_registered_key_publishes_under_its_own_namespace() {
    let p = mesh("kn-bundle", &[]).await;
    assert_eq!(register(&p, &SEED).await.status, 201);
    let ns = ns_of_key(&SEED);
    let r = p.post(key_bundle(&ns, &SEED)).await;
    assert_eq!(r.status, 201, "{}", r.text());
    let j = r.json();
    assert_eq!(j["names"], json!([format!("{ns}/level")]), "{j}");
    assert_eq!(j["imported"]["receipts"], 1, "{j}");
    // the cell answers, signed by the key, and verifies against the key document
    let c = p.get(&format!("/cell/{ns}/level?site=N"), &[]).await;
    assert_eq!(c.status, 200, "{}", c.text());
    let cj = c.json();
    assert_eq!((cj["outcome"].as_str(), cj["receipt"]["value"].as_str()), (Some("ok"), Some("42")), "{cj}");
    assert_eq!(cj["trust"], "primary", "{cj}");
    assert_eq!(cj["receipt"]["computed_by"], format!("{HOST}#{}", SigningKey::from_seed(&SEED).key_id()), "{cj}");
    // posting it again changes nothing
    assert_eq!(p.post(key_bundle(&ns, &SEED)).await.status, 201);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_key_may_not_publish_under_any_other_namespace() {
    let p = mesh("kn-other", &[]).await;
    assert_eq!(register(&p, &SEED).await.status, 201);
    assert_eq!(register(&p, &SEED2).await.status, 201);
    let own = ns_of_key(&SEED);
    let other_key = ns_of_key(&SEED2);
    let unregistered = ns_of_key(&[43; 32]);
    let hex = hex24(&SEED);
    for (what, ns) in [
        ("another registered key's namespace", other_key.clone()),
        ("a key namespace nobody registered", unregistered.clone()),
        ("the mesh's own domain namespace", format!("{HOST}/b4own")),
        ("the key segment itself", format!("{HOST}/key")),
        ("the key's namespace with a segment less", format!("{HOST}/key/k{hex}x")),
        ("23 digits (wrong length)", format!("{HOST}/key/k{}", &hex[..23])),
        (
            "25 digits (wrong length)",
            format!("{HOST}/key/k{}", &sha256_hex(&SigningKey::from_seed(&SEED).public_key())[..25]),
        ),
        ("no leading k", format!("{HOST}/key/{hex}")),
    ] {
        let r = p.post(key_bundle(&ns, &SEED)).await;
        assert!([400, 403, 422].contains(&r.status), "{what}: {} {}", r.status, r.text());
        assert!(r.json()["error"].is_string(), "{what}: {}", r.text());
        if !ns.ends_with(&format!("k{hex}x")) && ns != format!("{HOST}/key") {
            // the refusals of a namespace that is well formed are the ownership and domain codes
            if [other_key.as_str(), unregistered.as_str(), &format!("{HOST}/b4own")].contains(&ns.as_str()) {
                not_allowed(&r);
            }
        }
        nothing_under(&p, &ns);
    }
    // upper-case digits are not a key namespace (the name itself is not valid either)
    let upper = format!("{HOST}/key/k{}", hex.to_uppercase());
    if let Ok(b) = bundle::build_with_catalog(
        &upper,
        vec![stored_fn().canonical()],
        vec![(format!("{upper}/level"), stored_fn().hash())],
        vec![],
        vec![],
        &SEED,
    ) {
        let r = p.post(b).await;
        assert!([400, 403, 422].contains(&r.status), "{} {}", r.status, r.text());
    }
    assert!(
        p.m.lite.store.resolve_name(&format!("{own}/level"), END).unwrap().is_none(),
        "nothing of the key's own was written"
    );
    // and the key's own namespace still works after all that
    assert_eq!(p.post(key_bundle(&own, &SEED)).await.status, 201);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_key_names_bundle_with_a_name_outside_its_namespace_is_refused() {
    let p = mesh("kn-mixed", &[]).await;
    assert_eq!(register(&p, &SEED).await.status, 201);
    let own = ns_of_key(&SEED);
    let outside = format!("{HOST}/b4own/level");
    let b = bundle::build_with_catalog(
        &own,
        vec![stored_fn().canonical()],
        vec![(format!("{own}/level"), stored_fn().hash()), (outside.clone(), stored_fn().hash())],
        vec![],
        vec![],
        &SEED,
    );
    if let Ok(b) = b {
        let r = p.post(b).await;
        assert!([400, 403, 422].contains(&r.status), "{} {}", r.status, r.text());
        nothing_under(&p, &own);
        assert!(p.m.lite.store.resolve_name(&outside, END).unwrap().is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordinary_bundles_are_unchanged_with_key_names_on() {
    let stub = Stub::start(PUB).await;
    let peers = format!("{PUB}={}", stub.base);
    let p = mesh("kn-normal", &[("DATUMESH_PEERS", &peers), ("DATUMESH_PUBLISH_DOMAINS", PUB)]).await;
    // the mesh's own key, its own namespace
    let r = p.post(simple_bundle(HOST)).await;
    assert_eq!(r.status, 201, "{}", r.text());
    // an allowed domain with its own key document
    let r = p.post(simple_bundle(PUB)).await;
    assert_eq!(r.status, 201, "{}", r.text());
    // a domain that is not allowed is still refused
    let q = mesh("kn-normal2", &[("DATUMESH_PEERS", &peers)]).await;
    let r = q.post(simple_bundle(PUB)).await;
    q.refused(&r, "E_DOMAIN_NOT_ALLOWED");
    // a registered key does not widen anything for others: its namespace is not the mesh's own
    assert_eq!(register(&q, &SEED).await.status, 201);
    let r = q.post(simple_bundle(PUB)).await;
    q.refused(&r, "E_DOMAIN_NOT_ALLOWED");
}
