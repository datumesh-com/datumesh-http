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
//! 3. for a **stored** function, brings the cell's versions up to date: from the host's
//!    versions endpoint (9.8) when its mesh document lists one, every version the store lacks,
//!    each receipt fetched by hash and checked; otherwise the host's envelope for the cell
//!    now, then, walking back (as of just before each receipt's start), every earlier version
//!    the store lacks. Everything is fetched and checked by the store-less
//!    [`crate::client::Client`] (envelopes against the request, signatures against the
//!    signer's key document, fetched from the signer's own host); each receipt is then stored
//!    with `put_receipt` (which verifies it again against the key document stored for its
//!    signer) and `put_version`, oldest first.
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
//! Not in 0.1.x: slices over foreign functions (`E_INPUT_UNAVAILABLE`: the store would hold
//! only the members read so far), cells of a foreign name's earlier bindings that were never
//! imported while current, and `pending` answers from the other mesh (`E_INPUT_UNAVAILABLE`).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use datumesh_core::{
    format_uri, keys_from_response, Args, Cell, CellRef, Error as CoreError, ErrorCode, FunctionDescriptor, KeyDoc,
    MeshDoc, Receipt, Request, Scalar,
};
use datumesh_engine::{Budget, Clock, EvalError, FnBinding, Input, Inputs, Member, Runtime, SlicePattern};
use datumesh_store::Store;

use crate::client::{Client, ClientConfig};
use crate::config::Config;

/// A fetched version: the receipt bytes and the parsed receipt.
type Version = (Vec<u8>, Box<Receipt>);

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

/// A failure of cross-mesh resolution: the client's error (its code is the `error`
/// envelope's).
pub type ForeignError = crate::client::ClientError;

impl ForeignError {
    /// As an evaluation error (the engine reports it as `EngineError::Invalid`, which the
    /// mesh answers `error` with this code).
    pub(crate) fn eval(self) -> EvalError {
        EvalError::InvalidInput(CoreError::new(self.code, "$", format!("cross-mesh: {}", self.detail)))
    }
}

/// Percent-encodes a value as SPEC 5.1 says (literal characters as themselves).
pub fn pct(s: &str) -> String {
    crate::client::pct(s)
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

/// The cross-mesh resolver of a mesh: the [`Client`]'s fetches and checks, then the import
/// into the store.
pub struct Foreign {
    store: Arc<dyn Store + Send + Sync>,
    client: Client,
    clock: Arc<dyn Clock>,
    owned: Vec<String>,
    enabled: bool,
    ttl_binding: Duration,
    ttl_current: Duration,
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
        let cc = ClientConfig {
            peers: config.peers.clone(),
            allowed: config.cross_mesh_hosts.clone(),
            timeout: config.peer_timeout,
            key_ttl: config.ttl.key_doc,
            key_refetch: config.ttl.key_refetch,
            mesh_ttl: config.ttl.mesh_doc,
        };
        Foreign {
            store,
            client: Client::with_keys(cc, keys),
            clock,
            owned: config.namespaces.clone(),
            enabled: config.cross_mesh,
            ttl_binding: config.ttl.binding,
            ttl_current: config.ttl.current,
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

    /// The client it fetches and checks with.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Whether `name` (a function name) is under a host this mesh does not own, and
    /// cross-mesh resolution is on (and allows that host).
    pub fn is_foreign(&self, name: &str) -> bool {
        let host = host_of(name);
        self.enabled && !self.owned.iter().any(|h| h == host) && self.client.allowed(host)
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

    fn counted<T>(&self, r: Result<T, ForeignError>) -> Result<T, ForeignError> {
        self.fetches.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if r.is_err() {
            self.failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        r
    }

    /// The key document of `host` (see [`Client::key_document`]).
    pub fn keys_for(&self, host: &str, refresh: bool) -> Result<KeyDoc, ForeignError> {
        self.counted(self.client.key_document(host, refresh))
    }

    /// The mesh document of `host`, resolved with its key document (SPEC 9.6), cached.
    pub fn mesh(&self, host: &str) -> Result<MeshDoc, ForeignError> {
        self.counted(self.client.resolve(host))
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
        let host = host_of(name);
        let local = self
            .store
            .resolve_binding(name, END)
            .map_err(|e| ForeignError::unavailable(format!("store: {e}")))?
            .map(|b| (b.fn_hash, b.tx));
        let mut chain = Vec::new();
        let mut at: Option<String> = None;
        while let Some(b) = self.counted(self.client.binding(host, name, at.as_deref()))? {
            let b = (b.fn_hash, b.since);
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
            self.import_descriptor(host, fn_hash)?;
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

    fn import_descriptor(&self, host: &str, fn_hash: &str) -> Result<(), ForeignError> {
        if self.store.descriptor(fn_hash).is_some() {
            return Ok(());
        }
        let body = self.counted(self.client.descriptor(host, fn_hash))?;
        let h = self
            .store
            .put_descriptor(&body)
            .map_err(|e| ForeignError::new(ErrorCode::FnHashMismatch, e.to_string()))?;
        if h != fn_hash {
            return Err(ForeignError::new(ErrorCode::FnHashMismatch, format!("{host} gave the descriptor {h}")));
        }
        Ok(())
    }

    /// The other mesh's answer for `cell` as of `asof` (now when `None`), checked as an
    /// envelope against the request and the signer's key document.
    fn fetch_cell(&self, cell: &Cell, asof: Option<&str>) -> Result<Fetched, ForeignError> {
        let host = host_of(&cell.ns);
        let reference = CellRef {
            ns: cell.ns.clone(),
            fn_name: cell.fn_name.clone(),
            args: cell.args.clone(),
            asof: asof.map(str::to_string),
        };
        let uri = format_uri(&reference);
        let request = Request::new(&cell.id(), asof).map_err(|_| ForeignError::unavailable("request"))?;
        let f = match self.counted(self.client.cell(host, &uri["cell://".len()..], &request)) {
            Ok(f) => f,
            Err(e) if e.code == ErrorCode::Envelope => return Err(ForeignError::unavailable(e.detail)),
            Err(e) => return Err(e),
        };
        match (f.answer.outcome.as_str(), f.answer.receipt) {
            ("ok" | "stale", Some(r)) => {
                let bytes = datumesh_core::json::parse(&f.body)
                    .ok()
                    .and_then(|v| v.get("receipt").map(datumesh_core::json::Value::to_jcs))
                    .ok_or_else(|| ForeignError::unavailable("receipt"))?;
                Ok(Fetched::Receipt(bytes, Box::new(r)))
            }
            ("not_covered", _) => Ok(Fetched::NotCovered),
            (o, _) => Err(ForeignError::unavailable(format!("{host}: {o}"))),
        }
    }

    /// The versions of `cell` its mesh lists (9.8) after `after_tx`, with their receipts, when
    /// its mesh document has the versions endpoint; `Ok(None)` when it has none.
    fn listed_versions(&self, cell: &Cell, after_tx: Option<&str>) -> Result<Option<Vec<Version>>, ForeignError> {
        let host = host_of(&cell.ns);
        let mesh = self.mesh(host)?;
        if !mesh.endpoints.contains_key("versions") {
            return Ok(None);
        }
        let reference =
            CellRef { ns: cell.ns.clone(), fn_name: cell.fn_name.clone(), args: cell.args.clone(), asof: None };
        let uri = format_uri(&reference);
        let mut rest = uri["cell://".len()..].to_string();
        if let Some(t) = after_tx
            .and_then(datumesh_formula::calendar::parse_instant)
            .and_then(|us| datumesh_formula::calendar::instant_text(us + 1))
        {
            rest.push(if rest.contains('?') { '&' } else { '?' });
            rest.push_str(&format!("from={t}"));
        }
        let mut page = self.counted(self.client.versions(host, &rest))?;
        let mut out = Vec::new();
        loop {
            if page.cell.get("fn_hash").and_then(datumesh_core::json::Value::as_str) != Some(cell.fn_hash.as_str()) {
                return Err(ForeignError::unavailable(format!("{host}: the versions of another function")));
            }
            for v in &page.versions {
                let bytes = self.counted(self.client.receipt(host, &v.receipt))?;
                let r = Receipt::from_json(&bytes).map_err(|e| ForeignError::new(e.code(), e.to_string()))?;
                if r.cell().id() != cell.id() || r.tx() != v.tx {
                    return Err(ForeignError::unavailable(format!("{host}: a version of another cell")));
                }
                out.push((bytes, Box::new(r)));
                if out.len() > MAX_WALK {
                    return Err(ForeignError::unavailable(format!("{host}: more than {MAX_WALK} versions")));
                }
            }
            match page.next.clone() {
                Some(n) => page = self.counted(self.client.versions_at(&n))?,
                None => break,
            }
        }
        Ok(Some(out))
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
                "{name}: a cell of an earlier binding is not resolved in 0.1.x"
            )));
        }
        let local = self.store.latest(&id).map_err(|e| ForeignError::unavailable(format!("store: {e}")))?;
        // Oldest first.
        let chain: Vec<(Vec<u8>, Box<Receipt>)> =
            match self.listed_versions(cell, local.as_ref().map(|v| v.tx.as_str()))? {
                Some(listed) => listed,
                None => {
                    let mut chain: Vec<(Vec<u8>, Box<Receipt>)> = Vec::new();
                    let mut at: Option<String> = None;
                    loop {
                        match self.fetch_cell(cell, at.as_deref())? {
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
                    chain.reverse();
                    chain
                }
            };
        if let (Some((_, oldest)), Some(l)) = (chain.first(), &local) {
            if oldest.tx() <= l.tx.as_str() {
                return Err(ForeignError::unavailable(format!("{name}: the other mesh's versions do not extend ours")));
            }
        }
        let added = !chain.is_empty();
        for (bytes, r) in &chain {
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
        self.store_key_document(host)
    }

    /// Before this mesh computes a function under a host it does not own (a formula of
    /// another mesh, evaluated here): the publisher's key document, as cached (fetched again
    /// after its TTL), put in the store, where the engine reads its declarations (SPEC 8.5,
    /// spec-0.4.3). No HTTP response from the host is `E_FOREIGN_UNAVAILABLE`, its detail
    /// first in the error's `detail` ([`ForeignError::foreign_detail`]). Nothing for a
    /// namespace this mesh owns, or with cross-mesh resolution off. Since 0.2.1.
    pub fn ensure_publisher(&self, ns: &str) -> Result<(), ForeignError> {
        if !self.is_foreign(ns) {
            return Ok(());
        }
        let host = host_of(ns);
        self.counted(self.client.publisher_key_document(host))?;
        self.store_key_document(host)
    }

    /// The cached key document of `host`, put in the store unless the store's is the same.
    fn store_key_document(&self, host: &str) -> Result<(), ForeignError> {
        let bytes = self
            .client
            .key_document_bytes(host)
            .ok_or_else(|| ForeignError::new(ErrorCode::KeyUnavailable, host.to_string()))?;
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
        // Since 0.2.1: the publisher's declarations, for licence inheritance (SPEC 8.5).
        self.foreign.ensure_publisher(&cell.ns).map_err(ForeignError::eval)?;
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

    fn read_blob(&mut self, sha256: &str, length: u64) -> Result<Vec<u8>, EvalError> {
        self.inner.read_blob(sha256, length)
    }
}
