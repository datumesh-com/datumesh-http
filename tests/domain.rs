//! Domain validation (step 1.5): a namespace host is valid when `https://<host>/.well-known/
//! datumesh.json` fetches and passes the key-document rules (core 0.3.7's checks, declarations
//! included). A valid host accepts writes; one never validated is refused; a host that stops
//! serving a valid document is suspended by its second consecutive failure (not the first),
//! and its new writes are refused with `datumesh_store::Error::NamespaceSuspended`; the next
//! success reinstates it; a rotation in the document replaces the stored one; and everything
//! already stored stays verifiable, suspended or not. The hosts are local test servers
//! (`ClientConfig::peers`), never a real domain.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use datumesh_core::json::Value;
use datumesh_core::{verify, Keys, Receipt, SigningKey};
use datumesh_http::client::{Client, ClientConfig};
use datumesh_http::domain::{Outcome, Validation, Validator, SUSPEND_AFTER};
use datumesh_store::{Error, PgStore};

const H: &str = "rates.example.org";
const OWN: &str = "mesh-own.example";
const FROM: &str = "2020-01-01T00:00:00.000000Z";

/// What a test host serves at `/.well-known/datumesh.json`.
#[derive(Clone)]
enum Serve {
    Doc(Vec<u8>),
    NotFound,
    /// The connection is closed without a response.
    Down,
}

/// A host's origin on 127.0.0.1: a thread answering every request from the current state.
struct Host {
    origin: String,
    state: Arc<Mutex<Serve>>,
}

impl Host {
    fn start(first: Serve) -> Host {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(first));
        let s = state.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut c) = conn else { continue };
                let mut buf = [0u8; 4096];
                let mut got = Vec::new();
                while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                    match c.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => got.extend_from_slice(&buf[..n]),
                    }
                }
                let reply = match s.lock().unwrap().clone() {
                    Serve::Doc(d) => {
                        let mut r = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            d.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(&d);
                        r
                    }
                    Serve::NotFound => b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec(),
                    Serve::Down => continue,
                };
                let _ = c.write_all(&reply);
            }
        });
        Host { origin, state }
    }

    fn set(&self, s: Serve) {
        *self.state.lock().unwrap() = s;
    }
}

struct Fixture {
    db: TestDb,
    store: PgStore,
    host: Host,
    key: SigningKey,
    validator: Validator,
}

fn validator(hosts: &[(&str, &Host)], own: &[&str]) -> Validator {
    let mut cfg = ClientConfig::default();
    for (h, host) in hosts {
        cfg.peers.insert(h.to_string(), host.origin.clone());
    }
    cfg.timeout = Duration::from_secs(3);
    Validator::new(Client::new(cfg), own.iter().map(|s| s.to_string()).collect())
}

fn fixture() -> Fixture {
    let db = TestDb::migrated();
    let store = db.store(4);
    let key = key_for(H);
    let host = Host::start(Serve::Doc(key_doc(H, &key)));
    let validator = validator(&[(H, &host)], &[OWN]);
    Fixture { db, store, host, key, validator }
}

/// Instants a week apart from 2026-11-01.
fn t(n: i64) -> String {
    plus("2026-11-01T00:00:00.000000Z", n * 7 * 86_400 * 1_000_000)
}

fn check(f: &Fixture, n: i64) -> Validation {
    f.validator.validate(&f.store, H, &t(n)).unwrap()
}

/// A JSON lines file for `publish_stored`: a stored function under H, and a cell of it at `tx`.
fn lines(tx: &str, value: &str) -> String {
    format!(
        r#"{{"function":"{H}/fx_rate","signature":{{"args":{{"quote":{{"type":"string","valid":false}}}},"order":["quote"],"returns":{{"type":"number","unit":"1"}}}},"bound":"{FROM}"}}
{{"cell":{{"fn":"{H}/fx_rate","args":{{"quote":{{"type":"string","value":"USD"}}}}}},"value":{{"type":"number","value":"{value}"}},"tx":"{tx}","asof":"{tx}","issued":"{tx}","licence":"CC0-1.0","summary":"A test rate."}}
"#
    )
}

/// What `datumesh-mesh publish-stored` does for H (the functions are bound, the cells signed
/// with H's key and stored with their versions): `Ok` or the error.
fn publish(f: &Fixture, key: &SigningKey, tx: &str, value: &str) -> Result<datumesh_http::publish::Published, String> {
    let dir = out_dir().join(format!("domain-{}-{}", std::process::id(), f.db.name));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{tx}.jsonl"));
    std::fs::write(&path, lines(tx, value)).unwrap();
    let r = datumesh_http::publish::publish_stored(&f.store, key, H, &path).map_err(|e| e.to_string());
    let _ = std::fs::remove_dir_all(&dir);
    r
}

#[test]
fn a_valid_host_accepts_writes() {
    let f = fixture();
    let v = check(&f, 0);
    assert_eq!(v.outcome, Outcome::Valid, "{v:?}");
    assert!(!v.suspended && !v.reinstated && !v.rotated);
    let ns = f.store.namespace(H).unwrap().unwrap();
    assert_eq!(ns.validated_at, t(0), "validated_at is recorded through put_namespace");
    assert_eq!(ns.keydoc, key_doc(H, &f.key));
    assert_eq!((ns.consecutive_failures, ns.suspended_at, ns.last_checked_at), (0, None, Some(t(0))));
    let p = publish(&f, &f.key, "2026-03-01T00:00:00.000000Z", "1.08").unwrap();
    assert_eq!((p.functions, p.receipts, p.versions), (1, 1, 1), "{p:?}");
}

#[test]
fn an_unvalidated_host_is_refused() {
    let f = fixture();
    // Never validated: the store has no key document for it, so nothing signed by it is kept.
    let e = publish(&f, &f.key, "2026-03-01T00:00:00.000000Z", "1.08").unwrap_err();
    assert!(e.contains("key document"), "{e}");
    assert!(f.store.namespace(H).unwrap().is_none());
    // A host that serves no document stays unvalidated: nothing is recorded, nothing suspended.
    f.host.set(Serve::NotFound);
    for n in 0..3 {
        let v = check(&f, n);
        assert!(matches!(v.outcome, Outcome::Invalid { .. }), "{v:?}");
        assert!(!v.suspended);
    }
    assert!(f.store.namespace(H).unwrap().is_none());
    assert!(publish(&f, &f.key, "2026-03-01T00:00:00.000000Z", "1.08").is_err());
}

#[test]
fn removal_suspends_on_the_second_failure_not_the_first() {
    assert_eq!(SUSPEND_AFTER, 2);
    let f = fixture();
    assert_eq!(check(&f, 0).outcome, Outcome::Valid);
    publish(&f, &f.key, "2026-03-01T00:00:00.000000Z", "1.08").unwrap();

    // The host stops serving its document: the first failure is not enough.
    f.host.set(Serve::NotFound);
    let v = check(&f, 1);
    match &v.outcome {
        Outcome::Invalid { code, .. } => assert_eq!(code.as_str(), "E_KEY_UNAVAILABLE"),
        o => panic!("{o:?}"),
    }
    assert_eq!((v.failures, v.suspended), (1, false));
    publish(&f, &f.key, "2026-03-02T00:00:00.000000Z", "1.09").expect("still writable after one failure");

    // The second consecutive failure suspends.
    let v = check(&f, 2);
    assert_eq!((v.failures, v.suspended), (2, true));
    assert!(v.suspended_now);
    let ns = f.store.namespace(H).unwrap().unwrap();
    assert_eq!(ns.suspended_at, Some(t(2)));
    // New writes are refused, with the typed error (the store's, whatever the path).
    match f.store.bind_name(&format!("{H}/other"), &"0".repeat(64), "2026-04-01T00:00:00.000000Z") {
        Err(Error::NamespaceSuspended { host, suspended_at }) => assert_eq!((host.as_str(), suspended_at.as_str()), (H, t(2).as_str())),
        r => panic!("{r:?}"),
    }
    let e = publish(&f, &f.key, "2026-03-03T00:00:00.000000Z", "1.10").unwrap_err();
    assert!(e.contains("suspended"), "{e}");
    // A third failure keeps the first suspension's time.
    let v = check(&f, 3);
    assert_eq!((v.failures, v.suspended, v.suspended_now), (3, true, false));
    assert_eq!(f.store.namespace(H).unwrap().unwrap().suspended_at, Some(t(2)));
}

#[test]
fn a_document_that_fails_the_rules_or_a_host_that_is_down_counts_as_a_failure() {
    let f = fixture();
    assert_eq!(check(&f, 0).outcome, Outcome::Valid);
    // Not a key document; the document of another host (E_KEY_DOC); no answer at all.
    f.host.set(Serve::Doc(br#"{"proto":"datumesh/1"}"#.to_vec()));
    let v = check(&f, 1);
    assert!(matches!(&v.outcome, Outcome::Invalid { code, .. } if code.as_str() == "E_KEY_DOC"), "{v:?}");
    f.host.set(Serve::Doc(key_doc("calc.example.net", &key_for("calc.example.net"))));
    let v = check(&f, 2);
    assert!(matches!(&v.outcome, Outcome::Invalid { code, .. } if code.as_str() == "E_KEY_DOC"), "{v:?}");
    assert!(v.suspended);
    f.host.set(Serve::Down);
    let v = check(&f, 3);
    assert!(matches!(v.outcome, Outcome::Invalid { .. }), "{v:?}");
    assert_eq!(v.failures, 3);
}

#[test]
fn a_failure_between_successes_does_not_suspend() {
    let f = fixture();
    assert_eq!(check(&f, 0).outcome, Outcome::Valid);
    f.host.set(Serve::NotFound);
    assert_eq!(check(&f, 1).failures, 1);
    f.host.set(Serve::Doc(key_doc(H, &f.key)));
    assert_eq!(check(&f, 2).outcome, Outcome::Valid);
    f.host.set(Serve::NotFound);
    let v = check(&f, 3);
    assert_eq!((v.failures, v.suspended), (1, false), "failures must be consecutive");
}

#[test]
fn the_next_success_reinstates() {
    let f = fixture();
    assert_eq!(check(&f, 0).outcome, Outcome::Valid);
    f.host.set(Serve::NotFound);
    check(&f, 1);
    assert!(check(&f, 2).suspended);
    assert!(publish(&f, &f.key, "2026-03-01T00:00:00.000000Z", "1.08").is_err());
    // The host serves its document again.
    f.host.set(Serve::Doc(key_doc(H, &f.key)));
    let v = check(&f, 3);
    assert_eq!(v.outcome, Outcome::Valid);
    assert!(v.reinstated && !v.suspended, "{v:?}");
    let ns = f.store.namespace(H).unwrap().unwrap();
    assert_eq!((ns.suspended_at, ns.consecutive_failures), (None, 0));
    publish(&f, &f.key, "2026-03-01T00:00:00.000000Z", "1.08").expect("writes again");
}

#[test]
fn a_rotation_is_picked_up() {
    let f = fixture();
    assert_eq!(check(&f, 0).outcome, Outcome::Valid);
    // The host adds a key (the old one ending then): SPEC 9.5.
    let k2 = SigningKey::from_seed(&[77; 32]);
    let rotated_at = "2026-06-01T00:00:00.000000Z";
    let doc2 = datumesh_http::keydoc::with_key(H, Some(&key_doc(H, &f.key)), &k2, rotated_at).unwrap();
    f.host.set(Serve::Doc(doc2.clone()));
    let v = check(&f, 1);
    assert_eq!(v.outcome, Outcome::Valid);
    assert!(v.rotated, "{v:?}");
    let ns = f.store.namespace(H).unwrap().unwrap();
    assert_eq!(ns.keydoc, doc2, "the stored key document is replaced");
    assert_eq!(ns.validated_at, t(1));
    // A receipt signed with the new key is now stored; the old key's receipts still are.
    publish(&f, &f.key, "2026-03-01T00:00:00.000000Z", "1.08").expect("old key, before the rotation");
    // The new key's receipts need the key's own start: its function bound under the same name.
    let p = publish(&f, &k2, "2026-07-01T00:00:00.000000Z", "1.20").unwrap();
    assert_eq!(p.versions, 1, "{p:?}");
    // The same document again is not a rotation.
    let v = check(&f, 2);
    assert!(v.outcome == Outcome::Valid && !v.rotated, "{v:?}");
    // The rotation closed the first key with `valid_to` (that is what the stored document
    // now says), and a later rotation closing the second key is picked up the same way.
    assert!(String::from_utf8_lossy(&ns.keydoc).contains(&format!(r#""valid_to":"{rotated_at}""#)));
    let k3 = SigningKey::from_seed(&[78; 32]);
    let doc3 = datumesh_http::keydoc::with_key(H, Some(&doc2), &k3, "2026-09-01T00:00:00.000000Z").unwrap();
    f.host.set(Serve::Doc(doc3.clone()));
    let v = check(&f, 3);
    assert!(v.outcome == Outcome::Valid && v.rotated, "{v:?}");
    assert_eq!(f.store.namespace(H).unwrap().unwrap().keydoc, doc3);
}

#[test]
fn existing_receipts_of_a_suspended_namespace_still_verify() {
    let f = fixture();
    assert_eq!(check(&f, 0).outcome, Outcome::Valid);
    publish(&f, &f.key, "2026-03-01T00:00:00.000000Z", "1.08").unwrap();
    let cell_id = {
        let d = f.store.resolve_name(&format!("{H}/fx_rate"), "2026-03-01T00:00:00.000000Z").unwrap().unwrap();
        let cell = datumesh_core::Cell {
            ns: H.into(),
            fn_name: "fx_rate".into(),
            fn_hash: d,
            args: args(&[("quote", s("USD"))]),
        };
        cell.id()
    };
    let version = f.store.latest(&cell_id).unwrap().unwrap();
    let receipt = f.store.receipt(&version.receipt_hash).unwrap().into_owned();

    f.host.set(Serve::NotFound);
    check(&f, 1);
    assert!(check(&f, 2).suspended);

    // Reads: the version, the receipt, the key document; verification against the stored key
    // document; and putting the very receipt again stays idempotent.
    assert_eq!(f.store.latest(&cell_id).unwrap().unwrap().receipt_hash, version.receipt_hash);
    let keydoc = f.store.namespace(H).unwrap().unwrap().keydoc;
    let mut keys = Keys::new();
    keys.insert_response(H, 200, Some(&keydoc));
    let out = verify(&receipt, &keys, Some(&f.store), None);
    assert!(out.ok, "{:?}", out.error());
    assert_eq!(Receipt::from_json(&receipt).unwrap().hash(), version.receipt_hash);
    assert_eq!(f.store.put_receipt(&receipt).unwrap(), version.receipt_hash);
    assert!(publish(&f, &f.key, "2026-03-01T00:00:00.000000Z", "1.08").is_ok_and(|p| p.versions == 0), "re-running a file changes nothing");
}

#[test]
fn validate_all_checks_every_namespace_but_the_meshs_own() {
    let f = fixture();
    let other = Host::start(Serve::Doc(key_doc("calc.example.net", &key_for("calc.example.net"))));
    let own = Host::start(Serve::NotFound);
    let v = validator(&[(H, &f.host), ("calc.example.net", &other), (OWN, &own)], &[OWN]);
    f.store.put_namespace(H, &key_doc(H, &f.key), "2026-01-01T00:00:00.000000Z").unwrap();
    f.store.put_namespace("calc.example.net", &key_doc("calc.example.net", &key_for("calc.example.net")), "2026-01-01T00:00:00.000000Z").unwrap();
    f.store.put_namespace(OWN, &key_doc(OWN, &key_for(OWN)), "2026-01-01T00:00:00.000000Z").unwrap();
    f.host.set(Serve::NotFound);
    for n in 1..=3 {
        let all = v.validate_all(&f.store, &t(n)).unwrap();
        let by = |h: &str| all.iter().find(|x| x.host == h).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(by("calc.example.net").outcome, Outcome::Valid);
        assert_eq!(by(H).suspended, n >= 2, "{n}");
        // The mesh's own host is never validated, so never suspended.
        assert_eq!(by(OWN).outcome, Outcome::Skipped);
    }
    assert_eq!(f.store.namespace(OWN).unwrap().unwrap().suspended_at, None);
}

/// The key document the mesh serves for its own host, and a cell it answers, do not depend
/// on its namespace being suspended: suspension blocks new writes only.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_suspended_namespace_still_serves_its_key_document_and_cells() {
    let a = Node::start(OWN, |_| {}).await;
    a.stored(&format!("{OWN}/price"), r#"{"args":{"item":{"type":"string","valid":false}},"order":["item"],"returns":{"type":"number","unit":"EUR"}}"#);
    a.put_value(&format!("{OWN}/price"), &args(&[("item", s("bolt"))]), n("100"), "2026-03-01T00:00:00.000000Z");
    let doc_before = a.get("/.well-known/datumesh.json").await;
    assert_eq!(doc_before.status, 200);
    bl(|| {
        assert!(a.store.record_namespace_failure(OWN, &now(), 1).unwrap().unwrap().suspended_at.is_some());
    });
    let doc = a.get("/.well-known/datumesh.json").await;
    assert_eq!((doc.status, doc.body.clone()), (200, doc_before.body));
    let r = a.get(&format!("/cell/{OWN}/price?item=bolt")).await;
    assert_eq!(r.outcome(), "ok", "{}", r.text());
    let receipt = r.json().get("receipt").unwrap().to_jcs();
    let mut keys = Keys::new();
    keys.insert_response(OWN, 200, Some(&doc.body));
    assert!(verify(&receipt, &keys, None, None).ok);
    let _ = Value::Null;
}

/// The `validate` command: a host, then `--all`, against a database and a test host.
#[test]
fn the_validate_command() {
    let f = fixture();
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_datumesh-mesh"))
            .args(args)
            .env("DATABASE_URL", f.db.url())
            .env("DATUMESH_HOST", OWN)
            .env("DATUMESH_PEERS", format!("{H}={}", f.host.origin))
            .env("DATUMESH_PEER_TIMEOUT_MS", "3000")
            .output()
            .unwrap()
    };
    let out = run(&["validate", H]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let line = String::from_utf8(out.stdout).unwrap();
    let v = datumesh_core::json::parse(line.trim().as_bytes()).unwrap();
    assert_eq!(v.get("host").and_then(Value::as_str), Some(H));
    assert_eq!(v.get("outcome").and_then(Value::as_str), Some("valid"));
    assert!(f.store.namespace(H).unwrap().is_some());

    f.host.set(Serve::NotFound);
    let out = run(&["validate", "--all"]);
    assert!(!out.status.success(), "a failed validation exits non-zero");
    let v = datumesh_core::json::parse(String::from_utf8(out.stdout).unwrap().lines().next().unwrap().as_bytes()).unwrap();
    assert_eq!(v.get("outcome").and_then(Value::as_str), Some("invalid"));
    assert_eq!(v.get("code").and_then(Value::as_str), Some("E_KEY_UNAVAILABLE"));
    let out = run(&["validate", "--all"]);
    assert!(!out.status.success());
    assert!(f.store.namespace(H).unwrap().unwrap().suspended_at.is_some(), "the second failed run suspends");
    f.host.set(Serve::Doc(key_doc(H, &f.key)));
    let out = run(&["validate", H]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(f.store.namespace(H).unwrap().unwrap().suspended_at.is_none(), "reinstated");
    // No host and no --all: a usage error.
    assert!(!run(&["validate"]).status.success());
}
