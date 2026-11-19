//! Build 2 Part 4, P1 at the HTTP surface: `POST /bundles` (CONTRACT-build2-part4-publish).
//!
//! The public mesh is the test mesh (`mesh-test.datumesh.com`, SQLite, clock fixed at
//! 2026-11-16T00:00:00Z). The publishers are invented hosts (`pub.example`, ...), each a stub mesh on
//! loopback that the public mesh reaches through its `peers` (the cross-mesh tests' resolver hook): the
//! mesh fetches `https://pub.example/.well-known/datumesh.json` live and proves the domain with it.
//!
//! What these tests specify (the implementer follows):
//! * `POST /bundles`, a bundle as the body (`Content-Type: application/json`); `201` with
//!   `{"imported": {"functions_bound", "receipts", "versions", ...}, "names": [...], "functions":
//!   [{"name", "fn_hash", "page"}]}` (`page` is `https://<mesh host>/fn/<name>`); otherwise 400, 403
//!   or 422 with `{"error": "<code>", "detail": ...}`;
//! * the checks run in the contract's order: manifest signature (E_BUNDLE_SIGNATURE), then the
//!   domain proof (E_KEY_DOC for a key document that is unreadable, or that does not hold the
//!   manifest's key valid now, E_KEY_UNAVAILABLE for no answer within 5 s, E_NOT_OWNED for a name outside
//!   the host), then the allow list (E_DOMAIN_NOT_ALLOWED, 403), then `import-signed`'s rules
//!   (E_BACKDATED for a receipt older than a version the mesh holds), then the public test rule
//!   (E_TESTS_MISSING, with the lists), nothing written on any refusal;
//! * `DATUMESH_PUBLISH_DOMAINS` (`Config::from_vars`): empty or unset is the mesh's own namespaces only;
//!   a comma-separated list of hosts; `*` is any host with a valid key document;
//! * more than 16 MiB is E_TOO_LARGE; 10 bundles an hour per host and 60 per mesh, the next is
//!   `429` `rate_limited` with `Retry-After`;
//! * every attempt is one line in `<DATUMESH_LOG_DIR>/bundles.log`, with host, result and time, and
//!   no bundle contents;
//! * the mesh document lists `endpoints.x_bundles`, `https://<host>/bundles`.
//!
//! The tests that need `bundle::build_with_tests` (Part 3) are in `post_bundles_tests.rs`.

mod common;
#[path = "common/calculus.rs"]
mod calculus;
#[path = "common/publish4.rs"]
mod publish4;

use std::time::{Duration, Instant};

use common::lite::*;
use common::seed_for;
use datumesh_core::json::Value;
use datumesh_core::Receipt;
use datumesh_http::bundle;
use datumesh_store::Store;
use publish4::*;
use serde_json::json;

async fn allowed(domains: &str) -> (Public, Stub) {
    let stub = Stub::start(PUB).await;
    let p = public("pb", &[stub.peer()], Some(domains)).await;
    (p, stub)
}

// ---- accepted ---------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bundle_of_an_allowed_host_is_accepted_and_served() {
    let (p, _stub) = allowed(PUB).await;
    let b = simple_bundle_with(PUB, &seed_for(PUB), vec![level_receipt(PUB, "N", "42", TX)], Some("Levels, osprey edition"));
    let r = p.post(b.clone()).await;
    assert_eq!(r.status, 201, "{}", r.text());
    let j = r.json();
    let name = level_name(PUB);
    assert_eq!(j["imported"]["functions_bound"], 1, "{j}");
    assert_eq!(j["imported"]["receipts"], 1, "{j}");
    assert_eq!(j["imported"]["versions"], 1, "{j}");
    assert_eq!(j["names"], json!([name]), "{j}");
    assert_eq!(
        j["functions"],
        json!([{"name": name, "fn_hash": stored_fn().hash(), "page": format!("https://{HOST}/fn/{name}")}]),
        "{j}"
    );
    // the function is bound and its cell answers, signed by the publisher
    let c = p.get(&level_cell_path(PUB, "N"), &[]).await;
    assert_eq!(c.status, 200, "{}", c.text());
    let cj = c.json();
    assert_eq!((cj["outcome"].as_str(), cj["receipt"]["value"].as_str()), (Some("ok"), Some("42")), "{cj}");
    assert_eq!(cj["trust"], "primary");
    // the catalogue came with it
    let f = p.m.find("osprey").await;
    assert_eq!(common::readpath::result(&f, &name)["description"], "Levels, osprey edition");
    // posting it again changes nothing
    let again = p.post(b).await;
    assert_eq!(again.status, 201, "{}", again.text());
    let a = again.json();
    assert_eq!((a["imported"]["functions_bound"].as_u64(), a["imported"]["versions"].as_u64()), (Some(0), Some(0)), "{a}");
}

/// The mesh's own host needs no allow list: the default is its own namespaces only.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mesh_s_own_namespace_is_accepted_by_default() {
    let p = public("pb-own", &[], None).await;
    let r = p.post(simple_bundle(HOST)).await;
    assert_eq!(r.status, 201, "{}", r.text());
    assert_eq!(r.json()["names"], json!([level_name(HOST)]));
    assert_eq!(p.get(&level_cell_path(HOST, "N"), &[]).await.status, 200);
}

/// `DATUMESH_PUBLISH_DOMAINS`: unset or empty is own namespaces only; a list names hosts (spaces
/// allowed); `*` is any host with a valid key document.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_allow_list() {
    let stub = Stub::start(PUB).await;
    for (domains, ok) in [
        (None, false),
        (Some(""), false),
        (Some("other.example"), false),
        (Some("other.example, pub.example"), true),
        (Some(PUB), true),
        (Some("*"), true),
    ] {
        let p = public("pb-allow", &[stub.peer()], domains).await;
        let r = p.post(simple_bundle(PUB)).await;
        if ok {
            assert_eq!(r.status, 201, "{domains:?}: {}", r.text());
        } else {
            assert_eq!(r.status, 403, "{domains:?}: {}", r.text());
            p.refused(&r, "E_DOMAIN_NOT_ALLOWED");
            p.nothing_of(PUB);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mesh_document_lists_the_bundles_endpoint() {
    let p = public("pb-md", &[], None).await;
    let r = p.get("/.well-known/datumesh-mesh.json", &[]).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["endpoints"]["x_bundles"], format!("https://{HOST}/bundles"), "{}", r.text());
}

// ---- refused ------------------------------------------------------------------------------------

/// A bundle changed after signing: E_BUNDLE_SIGNATURE (before the allow list: the host here is not
/// allowed), nothing written. A body that is not a bundle is 400.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bad_signature() {
    let stub = Stub::start(PUB).await;
    let p = public("pb-sig", &[stub.peer()], None).await;
    let good = simple_bundle_with(PUB, &seed_for(PUB), vec![level_receipt(PUB, "N", "42", TX)], Some("Levels"));
    let tampered = edit(&good, |m| {
        for (k, v) in m.iter_mut() {
            if k == "catalog" {
                *v = Value::Array(vec![]);
            }
        }
    });
    let r = p.post(tampered).await;
    p.refused(&r, "E_BUNDLE_SIGNATURE");
    p.nothing_of(PUB);
    // a name added after signing
    let extra = edit(&good, |m| {
        for (k, v) in m.iter_mut() {
            if k == "names" {
                if let Value::Array(a) = v {
                    a.insert(0, datumesh_core::json::parse(format!(r#"{{"fn_hash":"{}","name":"{PUB}/b4/aaa"}}"#, stored_fn().hash()).as_bytes()).unwrap());
                }
            }
        }
    });
    let r = p.post(extra).await;
    p.refused(&r, "E_BUNDLE_SIGNATURE");
    p.nothing_of(PUB);
    // not a bundle at all
    for body in [b"not json".to_vec(), b"{}".to_vec(), b"[]".to_vec(), Vec::new()] {
        let r = p.post(body).await;
        assert_eq!(r.status, 400, "{}", r.text());
        assert!(r.json()["error"].is_string(), "{}", r.text());
    }
    p.nothing_of(PUB);
}

/// The manifest's key must be in the host's key document, valid now: a key the document does not
/// hold, one that starts in the future, one a rotation ended; and a document that is not one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_key_must_be_in_the_key_document_and_valid() {
    let b = |seed: &[u8; 32]| simple_bundle_with(PUB, seed, vec![level_receipt(PUB, "N", "42", TX)], None);
    // a key the document does not hold
    let stub = Stub::start(PUB).await;
    let p = public("pb-key1", &[stub.peer()], Some("*")).await;
    let r = p.post(b(&[9; 32])).await;
    p.refused(&r, "E_KEY_DOC");
    p.nothing_of(PUB);
    // the key starts in the future
    let live = datumesh_core::SigningKey::from_seed(&[7; 32]);
    let d1 = datumesh_http::keydoc::with_key(PUB, None, &live, common::KEY_FROM).unwrap();
    let future = datumesh_http::keydoc::with_key(PUB, Some(&d1), &common::key_for(PUB), "2030-01-01T00:00:00.000000Z").unwrap();
    let stub = Stub::with_doc(PUB, future, &live).await;
    let p = public("pb-key2", &[stub.peer()], Some("*")).await;
    let r = p.post(b(&seed_for(PUB))).await;
    p.refused(&r, "E_KEY_DOC");
    p.nothing_of(PUB);
    // the key was rotated out before now: the old key's window closed
    let old = datumesh_core::SigningKey::from_seed(&[5; 32]);
    let new = datumesh_core::SigningKey::from_seed(&[6; 32]);
    let d1 = datumesh_http::keydoc::with_key(PUB, None, &old, common::KEY_FROM).unwrap();
    let d2 = datumesh_http::keydoc::with_key(PUB, Some(&d1), &new, "2026-06-01T00:00:00.000000Z").unwrap();
    let stub = Stub::with_doc(PUB, d2, &new).await;
    let p = public("pb-key3", &[stub.peer()], Some("*")).await;
    let r = p.post(b(&[5; 32])).await;
    p.refused(&r, "E_KEY_DOC");
    p.nothing_of(PUB);
    // the new key's bundle is fine: the rotation is the only difference
    let r = p.post(simple_bundle_with(PUB, &[6; 32], vec![], None)).await;
    assert_eq!(r.status, 201, "{}", r.text());
    // the answer at the host's address is not a key document
    let (origin, _t) = raw_origin(200, "this is not a key document").await;
    let p = public("pb-key4", &[(PUB.into(), origin)], Some("*")).await;
    let r = p.post(b(&seed_for(PUB))).await;
    p.refused(&r, "E_KEY_DOC");
    p.nothing_of(PUB);
}

/// No answer from the host: E_KEY_UNAVAILABLE (refused connection; and silence, within the 5 s the
/// contract gives, whatever the peer timeout of cross-mesh reads).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_host_that_does_not_answer() {
    let b = simple_bundle(PUB);
    let p = public("pb-down", &[(PUB.into(), closed_origin())], Some("*")).await;
    let r = p.post(b.clone()).await;
    p.refused(&r, "E_KEY_UNAVAILABLE");
    p.nothing_of(PUB);

    let (origin, _t) = silent_origin().await;
    let lite = Lite::new("pb-slow");
    let cfg = public_config(&[(PUB.into(), origin)], Some("*"), None);
    let m = common::readpath::build_served(lite, Some(cfg), |c| c.peer_timeout = Duration::from_secs(60)).await;
    let p = Public { m, log_dir: std::env::temp_dir() };
    let t = Instant::now();
    let r = p.post(b).await;
    let took = t.elapsed();
    p.refused(&r, "E_KEY_UNAVAILABLE");
    assert!(took >= Duration::from_secs(4) && took < Duration::from_secs(9), "5 s timeout, took {took:?}");
    p.nothing_of(PUB);
}

/// A name outside the host: E_NOT_OWNED, nothing written. It is the domain proof's check: it comes
/// before the allow list (this mesh allows no host but its own).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_name_outside_the_host() {
    let stub = Stub::start(PUB).await;
    let p = public("pb-owned", &[stub.peer()], None).await;
    let b = bundle::build(
        &ns_of(PUB),
        vec![stored_fn().canonical()],
        vec![(level_name(PUB), stored_fn().hash()), ("other.example/b4/level".into(), stored_fn().hash())],
        vec![],
        &seed_for(PUB),
    )
    .unwrap();
    let r = p.post(b.clone()).await;
    p.refused(&r, "E_NOT_OWNED");
    p.nothing_of(PUB);
    let p = public("pb-owned2", &[stub.peer()], Some("*")).await;
    let r = p.post(b).await;
    p.refused(&r, "E_NOT_OWNED");
    p.nothing_of(PUB);
    assert!(p.m.lite.store.resolve_name("other.example/b4/level", END).unwrap().is_none());
}

/// A receipt older than a version the mesh holds is backdating: E_BACKDATED, and nothing of the
/// refused bundle (not its other name either, not its receipt) is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_backdated_receipt() {
    let (p, _stub) = allowed(PUB).await;
    let first = simple_bundle_with(PUB, &seed_for(PUB), vec![level_receipt(PUB, "N", "1", TX)], None);
    assert_eq!(p.post(first).await.status, 201);
    let early = level_receipt(PUB, "N", "2", "2026-11-14T11:00:00.000000Z");
    let early_hash = Receipt::from_json(&early).unwrap().hash().to_string();
    let extra = format!("{}/extra", ns_of(PUB));
    let second = bundle::build(
        &ns_of(PUB),
        vec![stored_fn().canonical()],
        vec![(level_name(PUB), stored_fn().hash()), (extra.clone(), stored_fn().hash())],
        vec![early],
        &seed_for(PUB),
    )
    .unwrap();
    let r = p.post(second).await;
    assert_eq!(r.status, 422, "{}", r.text());
    p.refused(&r, "E_BACKDATED");
    assert!(p.m.lite.store.resolve_name(&extra, END).unwrap().is_none(), "the other name is bound");
    assert_eq!(p.get(&format!("/receipt/{early_hash}"), &[]).await.status, 404, "the receipt is stored");
    // the value the first bundle gave still answers
    let c = p.get(&level_cell_path(PUB, "N"), &[]).await.json();
    assert_eq!(c["receipt"]["value"], "1", "{c}");
}

/// A computed function without tests: E_TESTS_MISSING, with what the mesh asks for; nothing of the
/// bundle is written (the stored function in it is not bound either).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_computed_function_without_tests() {
    let (p, _stub) = allowed(PUB).await;
    let c = computed_calc(PUB);
    let r = p.post(computed_bundle_without_tests(PUB)).await;
    assert_eq!(r.status, 422, "{}", r.text());
    p.refused(&r, "E_TESTS_MISSING");
    let detail = r.json()["detail"].to_string();
    for want in [c.per_unit.as_str(), "zero(units)", "negative(units)", "unexercised branches"] {
        assert!(detail.contains(want), "{want}: {detail}");
    }
    for name in [&c.per_unit, &c.reading] {
        assert!(p.m.lite.store.resolve_name(name, END).unwrap().is_none(), "{name} is bound");
    }
}

/// More than 16 MiB: E_TOO_LARGE, before anything is parsed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bundle_over_16_mib() {
    let (p, _stub) = allowed(PUB).await;
    let mut body = simple_bundle(PUB);
    body.resize(16 * 1024 * 1024 + 1, b' ');
    let r = p.post(body).await;
    assert!([400, 413, 422].contains(&r.status), "{}", r.status);
    assert_eq!(r.json()["error"], "E_TOO_LARGE", "{}", r.text());
    p.nothing_of(PUB);
    // the same bundle at the limit's side is not too large
    let r = p.post(simple_bundle(PUB)).await;
    assert_eq!(r.status, 201, "{}", r.text());
}

// ---- rate limits --------------------------------------------------------------------------------

fn assert_rate_limited(r: &Reply4) {
    assert_eq!(r.status, 429, "{}", r.text());
    assert!(r.header("retry-after").is_some(), "no Retry-After");
    let j = r.json();
    assert!(j["outcome"] == "rate_limited" || j["error"] == "rate_limited", "{j}");
}

/// 10 bundles an hour per host (the clock is fixed, the hour never ends): the 11th is refused, and
/// another host is not affected.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ten_bundles_an_hour_per_host() {
    let (a, b) = (Stub::start(PUB).await, Stub::start("pub2.example").await);
    let p = public("pb-rate", &[a.peer(), b.peer()], Some("*")).await;
    for i in 0..10 {
        let r = p.post(simple_bundle(PUB)).await;
        assert_eq!(r.status, 201, "bundle {i}: {}", r.text());
    }
    assert_rate_limited(&p.post(simple_bundle(PUB)).await);
    let r = p.post(simple_bundle("pub2.example")).await;
    assert_eq!(r.status, 201, "{}", r.text());
}

/// 60 bundles an hour per mesh: six hosts at ten each, then a seventh host is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sixty_bundles_an_hour_per_mesh() {
    let hosts: Vec<String> = (1..=7).map(|i| format!("pub{i}.example")).collect();
    let mut stubs = Vec::new();
    for h in &hosts {
        stubs.push(Stub::start(h).await);
    }
    let peers: Vec<_> = stubs.iter().map(Stub::peer).collect();
    let p = public("pb-rate60", &peers, Some("*")).await;
    for h in &hosts[..6] {
        for i in 0..10 {
            let r = p.post(simple_bundle(h)).await;
            assert_eq!(r.status, 201, "{h} bundle {i}: {}", r.text());
        }
    }
    assert_rate_limited(&p.post(simple_bundle(&hosts[6])).await);
}

// ---- the operator log ---------------------------------------------------------------------------

/// Every attempt is a line of `bundles.log` with the host, the result and a time; no contents.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_attempt_is_logged_without_contents() {
    let stub = Stub::start(PUB).await;
    let p = public("pb-log", &[stub.peer()], Some(PUB)).await;
    let secret = "a-description-that-must-not-reach-the-log";
    let good = simple_bundle_with(PUB, &seed_for(PUB), vec![level_receipt(PUB, "N", "42", TX)], Some(secret));
    assert_eq!(p.post(good).await.status, 201);
    let bad = simple_bundle_with(PUB, &[9; 32], vec![], Some(secret));
    p.refused(&p.post(bad).await, "E_KEY_DOC");
    let log = bundle_log(&p.log_dir, |t| t.lines().count() >= 2).await;
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 2, "{log}");
    assert!(lines.iter().all(|l| l.contains(PUB)), "{log}");
    assert!(lines[1].contains("E_KEY_DOC"), "{log}");
    assert!(!lines[0].contains("E_"), "{log}");
    for l in &lines {
        assert!(l.contains("20"), "a time: {l}");
        assert!(!l.contains(secret) && !l.contains(&stored_fn().hash()), "contents in the log: {l}");
    }
}
