//! `POST /bundles` (Build 2 Part 4, P1; CONTRACT-build2-part4-publish): a bundle of a publisher's
//! namespace is accepted when the mesh has proved the publisher's control of its host, the host may
//! publish here, and the bundle passes `import-signed`'s rules and the public test rule.
//!
//! The checks run in this order, nothing is written unless all pass:
//! 1. the body is at most 16 MiB and at most 50,000 receipts (`E_TOO_LARGE`, 413); the body is read to
//!    its end either way, so the client gets the answer;
//! 2. it parses as a bundle (`E_BUNDLE_FORMAT`, 400), and its host is under the rate limits (10 an
//!    hour per host, 60 an hour per mesh; `rate_limited`, 429 with `Retry-After`);
//! 3. the domain proof: the publisher's key document is fetched now (a mesh's own namespaces read the
//!    store; another host's `https://<host>/.well-known/datumesh.json` within 5 s:
//!    `E_KEY_UNAVAILABLE`, 422; a document that is not one, or holds no key of the manifest valid now:
//!    `E_KEY_DOC`, 422); every name is under the namespace (`E_NOT_OWNED`, 422); the manifest signature
//!    verifies (`E_BUNDLE_SIGNATURE`, 422);
//! 4. the host is allowed (`DATUMESH_PUBLISH_DOMAINS`; `E_DOMAIN_NOT_ALLOWED`, 403);
//! 5. the import rules (no backdating: `E_BACKDATED`, 422) and the public test rule
//!    (`E_TESTS_MISSING` / `E_TESTS_FAILING`, 422), then the writes.
//!
//! Every attempt is one line of `<DATUMESH_LOG_DIR>/bundles.log`: the time, the host, the result and
//! the counts, never the bundle's contents.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::time::{Duration, Instant};

use axum::body::Body;
use datumesh_core::json::{self, Value};
use http_body_util::BodyExt;

use crate::bundle::{self, BundleError};
use crate::envelope::{self, Cache, Reply};
use crate::mesh::Inner;

/// The most bytes of a bundle body.
pub(crate) const MAX_BODY: usize = 16 * 1024 * 1024;
/// The most receipts of a bundle.
pub(crate) const MAX_RECEIPTS: usize = 50_000;
/// The time the domain proof gives the publisher's key document.
const DOMAIN_TIMEOUT: Duration = Duration::from_secs(5);
/// The window of the rate limits.
const HOUR: Duration = Duration::from_secs(3600);
/// Attempts per host per hour, and for the mesh per hour.
const PER_HOST: usize = 10;
const PER_MESH: usize = 60;

/// The attempts at `POST /bundles` of the last hour, per host and for the mesh.
#[derive(Default)]
pub(crate) struct Limits {
    hosts: HashMap<String, VecDeque<Instant>>,
    mesh: VecDeque<Instant>,
}

impl Limits {
    /// Admits an attempt for `host`, or says after how many seconds a window of it frees a place.
    fn admit(&mut self, host: &str, now: Instant) -> Result<(), u64> {
        fn window(q: &mut VecDeque<Instant>, max: usize, now: Instant) -> Result<(), u64> {
            while q.front().is_some_and(|t| now.duration_since(*t) >= HOUR) {
                q.pop_front();
            }
            if q.len() >= max {
                let oldest = q.front().copied().unwrap_or(now);
                return Err(HOUR.saturating_sub(now.duration_since(oldest)).as_secs().max(1));
            }
            Ok(())
        }
        let Limits { hosts, mesh } = self;
        // A host whose attempts are all older than the window is forgotten (the map stays as small as
        // the last hour's attempts).
        hosts.retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) < HOUR));
        let q = hosts.entry(host.to_string()).or_default();
        window(q, PER_HOST, now)?;
        window(mesh, PER_MESH, now)?;
        q.push_back(now);
        mesh.push_back(now);
        Ok(())
    }
}

/// The body, read to its end: its bytes, or that it is larger than the limit (read and dropped), or
/// that it could not be read.
enum BodyRead {
    Bytes(Vec<u8>),
    TooLarge,
    Broken,
}

async fn read_capped(mut body: Body) -> BodyRead {
    let mut out = Vec::new();
    let mut too_large = false;
    while let Some(frame) = body.frame().await {
        let Ok(frame) = frame else { return BodyRead::Broken };
        if let Ok(data) = frame.into_data() {
            if too_large {
                continue;
            }
            if out.len() + data.len() > MAX_BODY {
                too_large = true;
                out = Vec::new();
            } else {
                out.extend_from_slice(&data);
            }
        }
    }
    if too_large {
        BodyRead::TooLarge
    } else {
        BodyRead::Bytes(out)
    }
}

/// `POST /bundles`: reads the body (on the request's task), then runs the checks on the pool.
pub(crate) async fn post(inner: std::sync::Arc<Inner>, body: Body) -> Reply {
    let bytes = match read_capped(body).await {
        BodyRead::Bytes(b) => b,
        BodyRead::TooLarge => {
            let r = refusal(413, "E_TOO_LARGE", "a bundle is at most 16 MiB".into());
            log(&inner, "-", "E_TOO_LARGE", 0, 0);
            return r;
        }
        BodyRead::Broken => {
            log(&inner, "-", "E_BUNDLE_FORMAT", 0, 0);
            return refusal(400, "E_BUNDLE_FORMAT", "the body could not be read".into());
        }
    };
    let i2 = inner.clone();
    match inner.pool.clone().run(move || attempt(&i2, &bytes)).await {
        Some(r) => r,
        None => envelope::error_reply(datumesh_core::ErrorCode::EvalUnavailable),
    }
}

/// What an attempt is known to be, for the log.
#[derive(Default)]
struct Seen {
    host: String,
    functions: usize,
    receipts: usize,
}

/// One attempt, logged whatever its result.
fn attempt(inner: &Inner, bytes: &[u8]) -> Reply {
    let mut seen = Seen { host: "-".into(), ..Seen::default() };
    let (reply, result) = check(inner, bytes, &mut seen);
    log(inner, &seen.host, &result, seen.functions, seen.receipts);
    reply
}

fn refusal(status: u16, code: &str, detail: String) -> Reply {
    let body =
        Value::Object(vec![("error".into(), Value::String(code.into())), ("detail".into(), Value::String(detail))]);
    Reply::json(status, body.to_jcs(), Cache::NoStore)
}

fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

/// The checks of an attempt, in the order of the module documentation: the reply and the result
/// (its error code, or `created`).
fn check(inner: &Inner, bytes: &[u8], seen: &mut Seen) -> (Reply, String) {
    let fail = |status: u16, code: &str, detail: String| (refusal(status, code, detail), code.to_string());
    let doc = match json::parse(bytes) {
        Ok(d @ Value::Object(_)) => d,
        Ok(_) => return fail(400, "E_BUNDLE_FORMAT", "a bundle is a JSON object".into()),
        Err(e) => return fail(400, "E_BUNDLE_FORMAT", format!("not JSON: {e}")),
    };
    let Some(namespace) = text(&doc, "namespace") else {
        return fail(400, "E_BUNDLE_FORMAT", "a bundle has a namespace".into());
    };
    let host = namespace.split('/').next().unwrap_or("").to_string();
    if namespace.split('/').count() < 2 || host.is_empty() {
        return fail(400, "E_BUNDLE_FORMAT", "the namespace has a host and a name".into());
    }
    seen.host = host.clone();
    let names: Vec<(String, String)> = match doc.get("names") {
        Some(Value::Array(a)) => {
            a.iter().filter_map(|n| Some((text(n, "name")?.to_string(), text(n, "fn_hash")?.to_string()))).collect()
        }
        _ => Vec::new(),
    };
    seen.functions = names.len();
    seen.receipts = match doc.get("receipts") {
        Some(Value::Array(a)) => a.len(),
        _ => 0,
    };
    if seen.receipts > MAX_RECEIPTS {
        return fail(413, "E_TOO_LARGE", format!("a bundle has at most {MAX_RECEIPTS} receipts"));
    }
    if let Err(wait) = inner.publish_limits.lock().unwrap_or_else(|p| p.into_inner()).admit(&host, Instant::now()) {
        return (envelope::rate_limited_reply(wait), "rate_limited".into());
    }

    // The domain proof: the publisher's key document, as it is now.
    let own = inner.config.owns(&host);
    let keydoc: Vec<u8> = if own {
        match inner.store.namespace(&host) {
            Ok(Some(n)) => n.keydoc,
            _ => return fail(422, "E_KEY_DOC", format!("the store holds no key document of {host}")),
        }
    } else {
        match inner.foreign.client().key_document_live(&host, DOMAIN_TIMEOUT) {
            Ok(b) => b,
            Err(e) => return fail(422, "E_KEY_UNAVAILABLE", e.to_string()),
        }
    };
    match bundle::prove(&*inner.store, &inner.config.host, &*inner.clock, bytes, &keydoc) {
        Ok(()) => {}
        Err(e) => return refused(&e),
    }

    // The allow list (the mesh's own namespaces need none).
    if !own && !inner.config.publish_domains.allows(&host) {
        return fail(403, "E_DOMAIN_NOT_ALLOWED", format!("{host} may not publish to this mesh"));
    }

    // The import: nothing is written unless every check passes.
    let owned = vec![host.clone()];
    let imported = match bundle::import_published(
        &*inner.store,
        &inner.config.host,
        &owned,
        &*inner.clock,
        bytes,
        Some(&keydoc),
    ) {
        Ok(i) => i,
        Err(e) => return refused(&e),
    };
    // The names and cells just imported are confirmed as of now (the publisher's bundle was checked
    // against its live key document): see `Foreign::confirm_imported`.
    let cells: Vec<(String, String)> = match doc.get("receipts") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|r| datumesh_core::Receipt::from_json(&r.to_jcs()).ok())
            .map(|r| (r.cell().function_name(), r.cell().id()))
            .collect(),
        _ => Vec::new(),
    };
    let just: Vec<String> = names.iter().map(|(n, _)| n.clone()).collect();
    inner.foreign.confirm_imported(&just, &cells, &inner.now());
    let origin = inner.config.origin();
    let functions: Vec<Value> = names
        .iter()
        .map(|(name, fh)| {
            Value::Object(vec![
                ("name".into(), Value::String(name.clone())),
                ("fn_hash".into(), Value::String(fh.clone())),
                ("page".into(), Value::String(format!("{origin}/fn/{name}"))),
            ])
        })
        .collect();
    let body = Value::Object(vec![
        ("imported".into(), json::parse(&imported.to_jcs()).unwrap_or(Value::Null)),
        ("names".into(), Value::Array(names.iter().map(|(n, _)| Value::String(n.clone())).collect())),
        ("functions".into(), Value::Array(functions)),
    ]);
    (Reply::json(201, body.to_jcs(), Cache::NoStore), "created".into())
}

/// The refusal of an import or proof error: its status and code (`E_` names of Build 2 Part 4).
fn refused(e: &BundleError) -> (Reply, String) {
    let (status, code, detail) = match e {
        BundleError::Format(m) => (400, "E_BUNDLE_FORMAT", m.clone()),
        BundleError::NotOwned(m) => (422, "E_NOT_OWNED", m.clone()),
        BundleError::Manifest(m) if m.starts_with("the signature does not verify") => {
            (422, "E_BUNDLE_SIGNATURE", m.clone())
        }
        BundleError::Manifest(m) => (422, "E_KEY_DOC", m.clone()),
        BundleError::Descriptor(m) => (422, "E_BUNDLE_DESCRIPTOR", m.clone()),
        BundleError::Receipt(m) => (422, "E_BUNDLE_RECEIPT", m.clone()),
        BundleError::TxInFuture { receipt, tx, now } => {
            (422, "E_TX_IN_FUTURE", format!("receipt {receipt}: tx {tx} is later than the mesh clock {now}"))
        }
        BundleError::Bind(m) if m.contains("at or after the clock") => (422, "E_BACKDATED", m.clone()),
        BundleError::Bind(m) => (422, "E_BIND", m.clone()),
        BundleError::Store(m) if m.contains("at or after") => (422, "E_BACKDATED", m.clone()),
        BundleError::Tests(m) => {
            let code = match m.split_once(':').map(|(c, _)| c) {
                Some("E_TESTS_FAILING") => "E_TESTS_FAILING",
                Some("E_TESTS_MISSING") => "E_TESTS_MISSING",
                _ => "E_BUNDLE_TESTS",
            };
            (422, code, m.clone())
        }
        other => (500, "error", other.to_string()),
    };
    (refusal(status, code, detail), code.to_string())
}

/// One line of the operator's log (`DATUMESH_LOG_DIR`, `bundles.log`).
fn log(inner: &Inner, host: &str, result: &str, functions: usize, receipts: usize) {
    let Some(dir) = &inner.config.log_dir else { return };
    let line = format!("{} host={host} result={result} functions={functions} receipts={receipts}\n", inner.now());
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("bundles.log"))
        .and_then(|mut f| f.write_all(line.as_bytes()));
    if let Err(e) = written {
        tracing::warn!(target: "datumesh_http", error = %e, "bundles.log: the attempt is not logged");
    }
}
