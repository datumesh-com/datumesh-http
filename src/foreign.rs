//! Cross-mesh resolution (SPEC 9.6, 9.7), minimal form: names under hosts this mesh does not
//! own.
//!
//! # How the engine sees foreign names
//!
//! The engine reads only its store, and the store holds only what verified on the way in. So
//! foreign records are **imported into the store before the engine reads them**, through a
//! read-through layer around every runtime ([`CrossMesh`]): when a function reads a name under
//! a host this mesh does not own, the layer first
//!
//! 1. resolves that host (its key document and its mesh document, `resolve_mesh`, cached),
//! 2. brings the name's bindings up to date from the host's bindings resource (9.7): the
//!    binding answering now and, walking back by `since`, every earlier one the store lacks,
//!    each with its descriptor (fetched by hash, checked by `put_descriptor`), appended with
//!    `bind_name` in order;
//! 3. for a **stored** function, brings the cell's versions up to date: the host's envelope
//!    for the cell now, checked with `check_envelope_any` against the signer's key document
//!    (fetched from its host over TLS, never from the mesh that served the receipt); then,
//!    walking back (as of just before each receipt's start), every earlier version the store
//!    lacks; each receipt stored with `put_receipt` (which verifies it again against the key
//!    document stored for its signer) and `put_version`, oldest first.
//!
//! A foreign function that is not stored (a formula on the other mesh) is evaluated by this
//! mesh from its descriptor and its inputs, which are imported the same way: its value is
//! recomputed, never taken on trust.
//!
//! Versions and bindings are append-only in the store, in transaction order; importing the
//! full history back to the first version (or binding) the store lacks keeps them so, so the
//! store answers every asof as the other mesh would. A history that does not extend what the
//! store holds (the other mesh rewrote it) is refused with `E_INPUT_UNAVAILABLE`.
//!
//! Answers for asofs at or before the last confirmation never change; answers about "now"
//! are confirmed again after [`Ttls::current`](crate::config::Ttls) (bindings after
//! `Ttls::binding`). The memo is kept honest by [`Foreign::take_invalidated`]: every cell
//! whose versions grew is reported to the engine (`stored_version_added`) after the request.
//!
//! # Caches and lifetimes
//!
//! | What | Where | Lifetime |
//! |---|---|---|
//! | key documents | memory ([`KeyCache`]) and the store's namespace record | 300 s; a key id the cached document lacks refetches it at once (at most every 10 s) |
//! | mesh documents | memory | 300 s |
//! | a name's bindings | the store | confirmed again after 60 s |
//! | a cell's versions | the store | confirmed again after 60 s, for requests about now |
//! | descriptors, receipts | the store | forever: content-addressed |
//!
//! Not in 0.1.0: slices over foreign functions (`E_INPUT_UNAVAILABLE`: the store would hold
//! only the members read so far), cells of a foreign name's earlier bindings that were never
//! imported while current, and `pending` answers from the other mesh (`E_INPUT_UNAVAILABLE`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use datumesh_core::json::{self, Value};
use datumesh_core::{
    check_envelope_any, format_uri, keys_from_response, lexical, resolve_mesh, AnyEnvelope, Args, Cell, CellRef,
    Error as CoreError, ErrorCode, FunctionDescriptor, KeyDoc, Keys, MeshDoc, Receipt, Request, Retrieval, Scalar,
};
use datumesh_engine::{Budget, Clock, EvalError, FnBinding, Input, Inputs, Member, Runtime, SlicePattern};
use datumesh_store::Store;

use crate::config::Config;

/// The largest body read from another mesh.
const MAX_BODY: u64 = 16 << 20;
/// The largest key or mesh document read (SPEC 9.3's suggested bound).
const MAX_DOC: u64 = 256 << 10;
/// The most versions or bindings one walk imports.
const MAX_WALK: usize = 10_000;

/// The latest instant: the store's latest binding or version is the one answering then.
const END: &str = "9999-12-31T23:59:59.999999Z";

/// Key documents this mesh has fetched or been given, by host: each the result of
/// interpreting a retrieval (SPEC 9.3), a failure included.
#[derive(Default)]
pub struct KeyCache {
    #[allow(clippy::type_complexity)]
    docs: Mutex<HashMap<String, (Result<KeyDoc, CoreError>, Vec<u8>, Instant)>>,
}

impl std::fmt::Debug for KeyCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyCache").field("hosts", &self.docs.lock().map(|d| d.len()).unwrap_or(0)).finish()
    }
}

impl KeyCache {
    /// Adds (or replaces) `host`'s key document, checked as a 200 response for that host;
    /// a document that fails the check is not added.
    pub fn insert(&self, host: &str, doc: &[u8]) -> Result<(), CoreError> {
        let kd = keys_from_response(host, 200, Some(doc))?;
        self.docs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(host.to_string(), (Ok(kd), doc.to_vec(), Instant::now()));
        Ok(())
    }

    /// Records what retrieving `host`'s key document gave (status and body, as SPEC 9.3
    /// interprets them): a document, or the failure (`E_KEY_UNAVAILABLE`, `E_KEY_DOC`).
    pub fn insert_response(&self, host: &str, status: u16, body: Option<&[u8]>) {
        let r = keys_from_response(host, status, body);
        self.docs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(host.to_string(), (r, body.unwrap_or_default().to_vec(), Instant::now()));
    }

    /// `host`'s document, its bytes and when it was fetched.
    pub fn get(&self, host: &str) -> Option<(KeyDoc, Vec<u8>, Instant)> {
        match self.docs.lock().unwrap_or_else(|p| p.into_inner()).get(host) {
            Some((Ok(d), b, at)) => Some((d.clone(), b.clone(), *at)),
            _ => None,
        }
    }

    /// What is recorded for `host`: its document or the failure.
    pub fn result(&self, host: &str) -> Option<Result<KeyDoc, CoreError>> {
        self.docs.lock().unwrap_or_else(|p| p.into_inner()).get(host).map(|(r, _, _)| r.clone())
    }

    /// Forgets `host`'s document.
    pub fn remove(&self, host: &str) {
        self.docs.lock().unwrap_or_else(|p| p.into_inner()).remove(host);
    }
}

/// A failure of cross-mesh resolution: the code of the `error` envelope it gives, and why.
#[derive(Clone, Debug)]
pub struct ForeignError {
    /// `E_INPUT_UNAVAILABLE` (the other mesh is down or answered otherwise), `E_MESH_DOC`,
    /// `E_KEY_UNAVAILABLE` or `E_KEY_DOC` (resolving its host), or what checking its envelope
    /// gave.
    pub code: ErrorCode,
    /// Why.
    pub detail: String,
}

impl ForeignError {
    fn new(code: ErrorCode, detail: impl Into<String>) -> ForeignError {
        ForeignError { code, detail: detail.into() }
    }
    fn unavailable(detail: impl Into<String>) -> ForeignError {
        ForeignError::new(ErrorCode::InputUnavailable, detail)
    }
    /// As an evaluation error (the engine reports it as `EngineError::Invalid`, which the
    /// mesh answers `error` with this code).
    pub(crate) fn eval(self) -> EvalError {
        EvalError::InvalidInput(CoreError::new(self.code, "$", format!("cross-mesh: {}", self.detail)))
    }
}

/// Percent-encodes a value as SPEC 5.1 says (literal characters as themselves).
pub fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b':') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// One microsecond before `t`.
fn before(t: &str) -> Option<String> {
    let us = datumesh_formula::calendar::parse_instant(t)?;
    datumesh_formula::calendar::instant_text(us - 1)
}

/// The host of a function name or namespace: its first segment.
pub fn host_of(name: &str) -> &str {
    name.split('/').next().unwrap_or(name)
}

struct Confirmed {
    at: Instant,
    /// The mesh's clock reading then: answers as of it or earlier never change.
    instant: String,
}

/// The cross-mesh resolver of a mesh.
pub struct Foreign {
    store: Arc<dyn Store + Send + Sync>,
    keys: Arc<KeyCache>,
    clock: Arc<dyn Clock>,
    agent: ureq::Agent,
    owned: Vec<String>,
    enabled: bool,
    allowed: Option<Vec<String>>,
    peers: BTreeMap<String, String>,
    ttl_key: Duration,
    ttl_refetch: Duration,
    ttl_mesh: Duration,
    ttl_binding: Duration,
    ttl_current: Duration,
    meshes: Mutex<HashMap<String, (MeshDoc, Instant)>>,
    names: Mutex<HashMap<String, Confirmed>>,
    cells: Mutex<HashMap<String, Confirmed>>,
    invalidated: Mutex<Vec<String>>,
    /// Whether `invalidated` may be non-empty, and whether any cell was imported (checked
    /// without a lock on every answer).
    dirty: std::sync::atomic::AtomicBool,
    any: std::sync::atomic::AtomicBool,
    deps: Mutex<HashMap<String, Arc<Vec<Cell>>>>,
    /// Counters for /metrics.
    pub(crate) fetches: std::sync::atomic::AtomicU64,
    pub(crate) failures: std::sync::atomic::AtomicU64,
    pub(crate) imported: std::sync::atomic::AtomicU64,
}

impl std::fmt::Debug for Foreign {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Foreign").field("owned", &self.owned).field("enabled", &self.enabled).finish_non_exhaustive()
    }
}

enum Fetched {
    Receipt(Vec<u8>, Box<Receipt>),
    NotCovered,
}

impl Foreign {
    pub(crate) fn new(
        config: &Config,
        store: Arc<dyn Store + Send + Sync>,
        keys: Arc<KeyCache>,
        clock: Arc<dyn Clock>,
    ) -> Foreign {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(config.peer_timeout))
            .user_agent(concat!("datumesh-mesh/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Foreign {
            store,
            keys,
            clock,
            agent,
            owned: config.namespaces.clone(),
            enabled: config.cross_mesh,
            allowed: config.cross_mesh_hosts.clone(),
            peers: config.peers.clone(),
            ttl_key: config.ttl.key_doc,
            ttl_refetch: config.ttl.key_refetch,
            ttl_mesh: config.ttl.mesh_doc,
            ttl_binding: config.ttl.binding,
            ttl_current: config.ttl.current,
            meshes: Mutex::new(HashMap::new()),
            names: Mutex::new(HashMap::new()),
            cells: Mutex::new(HashMap::new()),
            invalidated: Mutex::new(Vec::new()),
            dirty: Default::default(),
            any: Default::default(),
            deps: Mutex::new(HashMap::new()),
            fetches: Default::default(),
            failures: Default::default(),
            imported: Default::default(),
        }
    }

    /// Whether `name` (a function name) is under a host this mesh does not own, and
    /// cross-mesh resolution is on.
    pub fn is_foreign(&self, name: &str) -> bool {
        let host = host_of(name);
        self.enabled
            && !self.owned.iter().any(|h| h == host)
            && self.allowed.as_ref().is_none_or(|a| a.iter().any(|h| h == host))
    }

    /// Whether any foreign cell was ever imported (lineage checks are skipped otherwise).
    pub fn any_cells(&self) -> bool {
        self.any.load(std::sync::atomic::Ordering::Acquire)
    }

    /// The cells whose versions grew since the last call: the caller reports each to the
    /// engine (`stored_version_added`).
    pub fn take_invalidated(&self) -> Vec<String> {
        if !self.dirty.swap(false, std::sync::atomic::Ordering::AcqRel) {
            return Vec::new();
        }
        std::mem::take(&mut *self.invalidated.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// `GET url`, with the origin replaced for a configured peer: status and body.
    fn get(&self, url: &str, limit: u64) -> Result<(u16, Vec<u8>), String> {
        let mut target = url.to_string();
        if let Some(rest) = url.strip_prefix("https://") {
            let host = rest.split(['/', '?']).next().unwrap_or("");
            if let Some(origin) = self.peers.get(host) {
                target = format!("{origin}{}", &rest[host.len()..]);
            }
        }
        self.fetches.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let r = (|| {
            let mut resp = self.agent.get(&target).call().map_err(|e| format!("{url}: {e}"))?;
            let status = resp.status().as_u16();
            let body = resp.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| format!("{url}: {e}"))?;
            Ok((status, body))
        })();
        if r.is_err() {
            self.failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        r
    }

    /// The key document of `host`, from the cache while it is fresh, else fetched (SPEC 9.3).
    /// `refresh` fetches it again unless it was fetched within the refetch interval.
    pub fn keys_for(&self, host: &str, refresh: bool) -> Result<KeyDoc, ForeignError> {
        if let Some((doc, _, at)) = self.keys.get(host) {
            let age = at.elapsed();
            if (!refresh && age < self.ttl_key) || (refresh && age < self.ttl_refetch) {
                return Ok(doc);
            }
        }
        if lexical::check_dns_name(host, "$").is_err() {
            return Err(ForeignError::new(ErrorCode::KeyUnavailable, format!("{host:?} is not a DNS name")));
        }
        let url = format!("https://{host}/.well-known/datumesh.json");
        let (status, body) = self.get(&url, MAX_DOC).map_err(|e| ForeignError::new(ErrorCode::KeyUnavailable, e))?;
        let doc = keys_from_response(host, status, if status == 200 { Some(&body) } else { None })
            .map_err(|e| ForeignError::new(e.code(), e.to_string()))?;
        self.keys.insert(host, &body).map_err(|e| ForeignError::new(e.code(), e.to_string()))?;
        Ok(doc)
    }

    /// The mesh document of `host`, resolved with its key document (SPEC 9.6), cached.
    pub fn mesh(&self, host: &str) -> Result<MeshDoc, ForeignError> {
        if let Some((m, at)) = self.meshes.lock().unwrap_or_else(|p| p.into_inner()).get(host) {
            if at.elapsed() < self.ttl_mesh {
                return Ok(m.clone());
            }
        }
        if lexical::check_dns_name(host, "$").is_err() {
            return Err(ForeignError::new(ErrorCode::KeyUnavailable, format!("{host:?} is not a DNS name")));
        }
        let keys = self.get(&format!("https://{host}/.well-known/datumesh.json"), MAX_DOC);
        let mesh = match &keys {
            Ok((200, _)) => self.get(&format!("https://{host}/.well-known/datumesh-mesh.json"), MAX_DOC),
            _ => Err(String::new()),
        };
        fn ret(r: &Result<(u16, Vec<u8>), String>) -> Retrieval<'_> {
            match r {
                Ok((s, b)) => Retrieval::Response { status: *s, body: Some(b.as_slice()) },
                Err(_) => Retrieval::Failed,
            }
        }
        let resolved = resolve_mesh(host, ret(&keys), ret(&mesh)).map_err(|e| {
            let detail = match (&keys, &mesh) {
                (Err(e), _) | (_, Err(e)) if !e.is_empty() => e.clone(),
                _ => e.to_string(),
            };
            ForeignError::new(e.code(), detail)
        })?;
        if let Ok((_, body)) = &keys {
            let _ = self.keys.insert(host, body);
        }
        let m = resolved.mesh;
        self.meshes.lock().unwrap_or_else(|p| p.into_inner()).insert(host.to_string(), (m.clone(), Instant::now()));
        Ok(m)
    }

    /// The binding of `name` the host's bindings resource gives as of `asof` (now when
    /// `None`): `(fn_hash, since)`.
    fn remote_binding(
        &self,
        mesh: &MeshDoc,
        name: &str,
        asof: Option<&str>,
    ) -> Result<Option<(String, String)>, ForeignError> {
        let mut url = format!("{}?name={}", mesh.bindings, pct(name));
        if let Some(a) = asof {
            url.push_str(&format!("&asof={}", pct(a)));
        }
        let (status, body) = self.get(&url, MAX_DOC).map_err(ForeignError::unavailable)?;
        if status != 200 {
            return Err(ForeignError::unavailable(format!("{url}: status {status}")));
        }
        let v = json::parse(&body).map_err(|e| ForeignError::unavailable(format!("{url}: {e}")))?;
        let Some(b) = v.get("bindings").and_then(|b| b.get(name)) else {
            if v.get("bindings").and_then(Value::as_object).is_none() {
                return Err(ForeignError::unavailable(format!("{url}: no bindings object")));
            }
            return Ok(None);
        };
        let h = b.get("fn_hash").and_then(Value::as_str).unwrap_or("");
        let since = b.get("since").and_then(Value::as_str).unwrap_or("");
        if lexical::check_hex(h, 64, "$").is_err() || lexical::check_instant(since, "$").is_err() {
            return Err(ForeignError::unavailable(format!("{url}: malformed binding")));
        }
        if let Some(a) = asof {
            if since > a {
                return Err(ForeignError::unavailable(format!("{url}: a binding since after the asof")));
            }
        }
        Ok(Some((h.to_string(), since.to_string())))
    }

    /// Brings the bindings of a foreign `name` up to date (when not confirmed within the TTL).
    pub fn ensure_name(&self, name: &str) -> Result<(), ForeignError> {
        if !self.is_foreign(name) {
            return Ok(());
        }
        if let Some(c) = self.names.lock().unwrap_or_else(|p| p.into_inner()).get(name) {
            if c.at.elapsed() < self.ttl_binding {
                return Ok(());
            }
        }
        let now = self.clock.now();
        let mesh = self.mesh(host_of(name))?;
        let local = self
            .store
            .resolve_binding(name, END)
            .map_err(|e| ForeignError::unavailable(format!("store: {e}")))?
            .map(|b| (b.fn_hash, b.tx));
        let mut chain = Vec::new();
        let mut at: Option<String> = None;
        while let Some(b) = self.remote_binding(&mesh, name, at.as_deref())? {
            if Some(&b) == local.as_ref() {
                break;
            }
            if chain.len() >= MAX_WALK {
                return Err(ForeignError::unavailable(format!("{name}: more than {MAX_WALK} bindings")));
            }
            at = Some(before(&b.1).ok_or_else(|| ForeignError::unavailable("a binding at the first instant"))?);
            let stop = at.as_deref() < Some("0001-01-01T00:00:00.000000Z");
            chain.push(b);
            if stop {
                break;
            }
        }
        if let (Some(oldest), Some(l)) = (chain.last(), &local) {
            if oldest.1 <= l.1 {
                return Err(ForeignError::unavailable(format!("{name}: the other mesh's bindings do not extend ours")));
            }
        }
        for (fn_hash, since) in chain.iter().rev() {
            self.import_descriptor(&mesh, fn_hash)?;
            self.store
                .bind_name(name, fn_hash, since)
                .map_err(|e| ForeignError::unavailable(format!("bind {name}: {e}")))?;
        }
        self.names
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(name.to_string(), Confirmed { at: Instant::now(), instant: now });
        Ok(())
    }

    fn import_descriptor(&self, mesh: &MeshDoc, fn_hash: &str) -> Result<(), ForeignError> {
        if self.store.descriptor(fn_hash).is_some() {
            return Ok(());
        }
        let url = mesh
            .url("descriptor", &[("hash", fn_hash)])
            .ok_or_else(|| ForeignError::new(ErrorCode::MeshDoc, "no descriptor endpoint"))?;
        let (status, body) = self.get(&url, MAX_BODY).map_err(ForeignError::unavailable)?;
        if status != 200 {
            return Err(ForeignError::new(ErrorCode::FnUnavailable, format!("{url}: status {status}")));
        }
        let h = self
            .store
            .put_descriptor(&body)
            .map_err(|e| ForeignError::new(ErrorCode::FnHashMismatch, e.to_string()))?;
        if h != fn_hash {
            return Err(ForeignError::new(ErrorCode::FnHashMismatch, format!("{url} gave the descriptor {h}")));
        }
        Ok(())
    }

    /// The other mesh's answer for `cell` as of `asof` (now when `None`), checked as an
    /// envelope against the request and the signer's key document.
    fn fetch_cell(&self, mesh: &MeshDoc, cell: &Cell, asof: Option<&str>) -> Result<Fetched, ForeignError> {
        let reference = CellRef {
            ns: cell.ns.clone(),
            fn_name: cell.fn_name.clone(),
            args: cell.args.clone(),
            asof: asof.map(str::to_string),
        };
        let uri = format_uri(&reference);
        let url = mesh
            .url("cell", &[("cell", &uri["cell://".len()..])])
            .ok_or_else(|| ForeignError::new(ErrorCode::MeshDoc, "no cell endpoint"))?;
        let (status, body) = self.get(&url, MAX_BODY).map_err(ForeignError::unavailable)?;
        let request = Request::new(&cell.id(), asof).map_err(|_| ForeignError::unavailable("request"))?;
        // The signer is named in the body: its key document comes from its own host.
        let signer = json::parse(&body).ok().and_then(|v| {
            v.get("receipt")?.get("computed_by")?.as_str().map(|s| s.split('#').next().unwrap_or("").to_string())
        });
        let mut refreshed = false;
        loop {
            let mut keys = Keys::new();
            if let Some(h) = &signer {
                keys.insert(self.keys_for(h, refreshed)?);
            }
            match check_envelope_any(status, &body, &keys, &request) {
                Ok(AnyEnvelope::Envelope(env, _)) => {
                    return match (env.outcome.as_str(), env.receipt) {
                        ("ok" | "stale", Some(r)) => {
                            // The receipt's own bytes, as served (put_receipt canonicalises).
                            let bytes = json::parse(&body)
                                .ok()
                                .and_then(|v| v.get("receipt").map(Value::to_jcs))
                                .ok_or_else(|| ForeignError::unavailable("receipt"))?;
                            Ok(Fetched::Receipt(bytes, Box::new(r)))
                        }
                        ("not_covered", _) => Ok(Fetched::NotCovered),
                        (o, _) => Err(ForeignError::unavailable(format!("{url}: {o}"))),
                    };
                }
                Ok(other) => return Err(ForeignError::unavailable(format!("{url}: {}", other.outcome_str()))),
                Err(e) if !refreshed && matches!(e.code(), ErrorCode::KeyUnknown | ErrorCode::KeyNotValid) => {
                    refreshed = true;
                }
                Err(e) => {
                    let code = match e.code() {
                        ErrorCode::Envelope => ErrorCode::InputUnavailable,
                        c => c,
                    };
                    return Err(ForeignError::new(code, format!("{url}: {e}")));
                }
            }
        }
    }

    /// Brings the versions of a foreign stored cell up to date (when not confirmed within the
    /// TTL). `Ok(true)` when versions were added.
    pub fn ensure_cell(&self, cell: &Cell) -> Result<bool, ForeignError> {
        let id = cell.id();
        if let Some(c) = self.cells.lock().unwrap_or_else(|p| p.into_inner()).get(&id) {
            if c.at.elapsed() < self.ttl_current {
                return Ok(false);
            }
        }
        let now = self.clock.now();
        let name = cell.function_name();
        let current = self
            .store
            .resolve_binding(&name, &now)
            .map_err(|e| ForeignError::unavailable(format!("store: {e}")))?
            .map(|b| b.fn_hash);
        if current.as_deref() != Some(cell.fn_hash.as_str()) {
            if self.cells.lock().unwrap_or_else(|p| p.into_inner()).contains_key(&id) {
                // A cell of an earlier binding, imported while current: its versions are final.
                return Ok(false);
            }
            return Err(ForeignError::unavailable(format!(
                "{name}: a cell of an earlier binding is not resolved in 0.1.0"
            )));
        }
        let mesh = self.mesh(host_of(&name))?;
        let local = self.store.latest(&id).map_err(|e| ForeignError::unavailable(format!("store: {e}")))?;
        let mut chain: Vec<(Vec<u8>, Box<Receipt>)> = Vec::new();
        let mut at: Option<String> = None;
        loop {
            match self.fetch_cell(&mesh, cell, at.as_deref())? {
                Fetched::NotCovered => break,
                Fetched::Receipt(bytes, r) => {
                    if local.as_ref().is_some_and(|v| v.receipt_hash == r.hash()) {
                        break;
                    }
                    if chain.iter().any(|(_, c)| c.hash() == r.hash()) || chain.len() >= MAX_WALK {
                        return Err(ForeignError::unavailable(format!(
                            "{name}: the other mesh's history does not end"
                        )));
                    }
                    let start = std::cmp::max(r.tx(), r.asof()).to_string();
                    let next = before(&start);
                    chain.push((bytes, r));
                    match next {
                        Some(t) => at = Some(t),
                        None => break,
                    }
                }
            }
        }
        if let (Some((_, oldest)), Some(l)) = (chain.last(), &local) {
            if oldest.tx() <= l.tx.as_str() {
                return Err(ForeignError::unavailable(format!("{name}: the other mesh's versions do not extend ours")));
            }
        }
        let added = !chain.is_empty();
        for (bytes, r) in chain.iter().rev() {
            self.ensure_namespace(r.signer_host())?;
            let h =
                self.store.put_receipt(bytes).map_err(|e| ForeignError::unavailable(format!("put_receipt: {e}")))?;
            self.store
                .put_version(&id, r.tx(), &h)
                .map_err(|e| ForeignError::unavailable(format!("put_version: {e}")))?;
            self.imported.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if added {
            self.invalidated.lock().unwrap_or_else(|p| p.into_inner()).push(id.clone());
            self.dirty.store(true, std::sync::atomic::Ordering::Release);
        }
        self.cells.lock().unwrap_or_else(|p| p.into_inner()).insert(id, Confirmed { at: Instant::now(), instant: now });
        self.any.store(true, std::sync::atomic::Ordering::Release);
        Ok(added)
    }

    /// Puts the signer host's key document (as cached, fetched from that host) in the store,
    /// unless the store's is the same. Never for a host this mesh owns.
    fn ensure_namespace(&self, host: &str) -> Result<(), ForeignError> {
        if self.owned.iter().any(|h| h == host) {
            return Ok(());
        }
        self.keys_for(host, false)?;
        let (_, bytes, _) =
            self.keys.get(host).ok_or_else(|| ForeignError::new(ErrorCode::KeyUnavailable, host.to_string()))?;
        let stored = self.store.namespace(host).map_err(|e| ForeignError::unavailable(format!("store: {e}")))?;
        if stored.as_ref().map(|n| n.keydoc.as_slice()) != Some(bytes.as_slice()) {
            self.store
                .put_namespace(host, &bytes, &self.clock.now())
                .map_err(|e| ForeignError::new(ErrorCode::KeyDoc, format!("put_namespace {host}: {e}")))?;
        }
        Ok(())
    }

    /// The foreign stored cells a receipt rests on, through its lineage in the store
    /// (bounded; cached by receipt hash).
    fn foreign_deps(&self, receipt_hash: &str) -> Arc<Vec<Cell>> {
        if let Some(d) = self.deps.lock().unwrap_or_else(|p| p.into_inner()).get(receipt_hash) {
            return d.clone();
        }
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut todo = vec![receipt_hash.to_string()];
        while let Some(h) = todo.pop() {
            if !seen.insert(h.clone()) || seen.len() > 100_000 {
                continue;
            }
            if h.starts_with("slice:") {
                if let Some(m) = self.store.manifest(&h) {
                    if let Ok(m) = datumesh_core::Manifest::from_json(&m) {
                        todo.extend(m.members().iter().cloned());
                    }
                }
                continue;
            }
            let Some(bytes) = self.store.receipt(&h) else {
                continue;
            };
            let Ok(r) = Receipt::from_json(&bytes) else { continue };
            if self.is_foreign(&r.cell().ns) && r.inputs().is_empty() {
                out.push(r.cell().clone());
            } else {
                todo.extend(r.inputs().iter().cloned());
            }
        }
        let d = Arc::new(out);
        let mut deps = self.deps.lock().unwrap_or_else(|p| p.into_inner());
        if deps.len() > 100_000 {
            deps.clear();
        }
        deps.insert(receipt_hash.to_string(), d.clone());
        d
    }

    /// Confirms the foreign cells an answer rests on, for an answer as of `asof`: those not
    /// confirmed within the TTL and confirmed before `asof` are fetched again. `Ok(true)` when
    /// versions were added (the caller answers again).
    pub fn refresh_deps(&self, receipt_hash: &str, asof: &str) -> Result<bool, ForeignError> {
        if !self.any_cells() {
            return Ok(false);
        }
        let mut added = false;
        for cell in self.foreign_deps(receipt_hash).iter() {
            let stale = match self.cells.lock().unwrap_or_else(|p| p.into_inner()).get(&cell.id()) {
                Some(c) => c.at.elapsed() >= self.ttl_current && asof > c.instant.as_str(),
                None => true,
            };
            if stale {
                added |= self.ensure_cell(cell)?;
            }
        }
        Ok(added)
    }

    /// For a request about a foreign name as of `asof` (now when `None`): its bindings and,
    /// for a stored function's cell, its versions, brought up to date when needed.
    pub fn prepare(&self, cell: &Cell, stored: bool, asof: Option<&str>) -> Result<(), ForeignError> {
        if !self.is_foreign(&cell.function_name()) || !stored {
            return Ok(());
        }
        let fresh = match self.cells.lock().unwrap_or_else(|p| p.into_inner()).get(&cell.id()) {
            Some(c) => c.at.elapsed() < self.ttl_current || asof.is_some_and(|a| a <= c.instant.as_str()),
            None => false,
        };
        if !fresh {
            self.ensure_cell(cell)?;
        }
        Ok(())
    }
}

/// A runtime with cross-mesh reads in front of it (see the module documentation).
pub struct CrossMesh {
    inner: Arc<dyn Runtime>,
    foreign: Arc<Foreign>,
}

impl CrossMesh {
    /// `inner`, reading foreign names through `foreign`.
    pub fn new(inner: Arc<dyn Runtime>, foreign: Arc<Foreign>) -> CrossMesh {
        CrossMesh { inner, foreign }
    }
}

impl Runtime for CrossMesh {
    fn lang(&self) -> &str {
        self.inner.lang()
    }

    fn evaluate(
        &self,
        descriptor: &FunctionDescriptor,
        cell: &Cell,
        inputs: &mut dyn Inputs,
        budget: &mut Budget,
    ) -> Result<Scalar, EvalError> {
        for name in &descriptor.inputs {
            self.foreign.ensure_name(name).map_err(ForeignError::eval)?;
        }
        let mut w = Through { inner: inputs, foreign: &self.foreign };
        self.inner.evaluate(descriptor, cell, &mut w, budget)
    }

    fn asof_dependent(&self, descriptor: &FunctionDescriptor) -> bool {
        self.inner.asof_dependent(descriptor)
    }
}

/// The inputs of one evaluation, foreign reads imported first.
struct Through<'a> {
    inner: &'a mut dyn Inputs,
    foreign: &'a Foreign,
}

impl Through<'_> {
    fn pre(&mut self, name: &str, args: &Args, at: Option<&str>) -> Result<(), EvalError> {
        if !self.foreign.is_foreign(name) {
            return Ok(());
        }
        self.foreign.ensure_name(name).map_err(ForeignError::eval)?;
        let Some(b) = self.inner.binding_peek(name, at)? else {
            return Ok(());
        };
        if b.descriptor.lang != "stored" {
            return Ok(());
        }
        let (ns, f) = name.rsplit_once('/').unwrap_or((name, ""));
        let cell = Cell { ns: ns.to_string(), fn_name: f.to_string(), fn_hash: b.fn_hash.clone(), args: args.clone() };
        self.foreign.ensure_cell(&cell).map_err(ForeignError::eval)?;
        Ok(())
    }
}

impl Inputs for Through<'_> {
    fn read(&mut self, fn_name: &str, args: &Args) -> Result<Scalar, EvalError> {
        self.pre(fn_name, args, None)?;
        self.inner.read(fn_name, args)
    }

    fn read_cell(&mut self, fn_name: &str, args: &Args) -> Result<Input, EvalError> {
        self.pre(fn_name, args, None)?;
        self.inner.read_cell(fn_name, args)
    }

    fn read_asof(&mut self, fn_name: &str, args: &Args, t: &str) -> Result<Input, EvalError> {
        self.pre(fn_name, args, Some(t))?;
        self.inner.read_asof(fn_name, args, t)
    }

    fn read_slice(&mut self, fn_name: &str, pattern: &SlicePattern) -> Result<Vec<Member>, EvalError> {
        if self.foreign.is_foreign(fn_name) {
            self.foreign.ensure_name(fn_name).map_err(ForeignError::eval)?;
            if let Some(b) = self.inner.binding_peek(fn_name, None)? {
                if b.descriptor.lang == "stored" {
                    return Err(ForeignError::unavailable(format!(
                        "{fn_name}: slices of foreign stored functions are not read in 0.1.0"
                    ))
                    .eval());
                }
            }
        }
        self.inner.read_slice(fn_name, pattern)
    }

    fn read_versions(&mut self, fn_name: &str, args: &Args, from: &str, to: &str) -> Result<Vec<Input>, EvalError> {
        self.pre(fn_name, args, None)?;
        self.inner.read_versions(fn_name, args, from, to)
    }

    fn binding(&mut self, fn_name: &str, at: Option<&str>) -> Result<Option<FnBinding>, EvalError> {
        self.foreign.ensure_name(fn_name).map_err(ForeignError::eval)?;
        self.inner.binding(fn_name, at)
    }

    fn asof(&mut self) -> String {
        self.inner.asof()
    }

    fn binding_peek(&mut self, fn_name: &str, at: Option<&str>) -> Result<Option<FnBinding>, EvalError> {
        self.foreign.ensure_name(fn_name).map_err(ForeignError::eval)?;
        self.inner.binding_peek(fn_name, at)
    }

    fn asof_compare(&mut self, t: &str) -> Result<std::cmp::Ordering, EvalError> {
        self.inner.asof_compare(t)
    }
}
