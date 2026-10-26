//! [`Mesh`]: the engine, its store and everything the endpoints share; the blocking
//! operations each endpoint is built on, usable as library calls.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use datumesh_core::json::{self, Value};
use datumesh_core::{
    keys_from_response, verify, ErrorCode, FunctionDescriptor, KeyDoc, Keys, Receipt, Request, SigningKey, Trust,
};
use datumesh_engine::{Answer, Clock, Engine, EngineError, Runtime, SystemClock};
use datumesh_formula::runtime::FormulaRuntime;
use datumesh_store::Store;

use crate::apikeys::Principal;
use crate::call::{self, Bindings, Bound, Rejection, Target};
use crate::config::{Config, ConfigError, Rule};
use crate::envelope::{self, Cache, Reply};
use crate::foreign::{host_of, CrossMesh, Foreign, ForeignError, KeyCache};
use crate::jobs::Jobs;
use crate::limit::Limiter;
use crate::metrics::Metrics;
use crate::pool::Pool;

/// Why a mesh could not be built.
#[derive(Debug)]
#[non_exhaustive]
pub enum BuildError {
    /// The configuration.
    Config(ConfigError),
    /// The engine's configuration (the key, its publication in the store, the runtimes).
    Engine(datumesh_engine::ConfigError),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::Config(e) => write!(f, "{e}"),
            BuildError::Engine(e) => write!(f, "engine: {e}"),
        }
    }
}

impl std::error::Error for BuildError {}

/// Builds a [`Mesh`].
pub struct MeshBuilder {
    config: Config,
    store: Arc<dyn Store + Send + Sync>,
    key: Option<SigningKey>,
    key_file: Option<PathBuf>,
    runtimes: Vec<Arc<dyn Runtime>>,
    formula: bool,
    clock: Arc<dyn Clock>,
    fuel: Option<u64>,
    licence: Option<String>,
    memo_capacity: Option<usize>,
}

impl std::fmt::Debug for MeshBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeshBuilder").field("host", &self.config.host).finish_non_exhaustive()
    }
}

impl MeshBuilder {
    /// The mesh's signing key.
    pub fn signing_key(mut self, key: SigningKey) -> Self {
        self.key = Some(key);
        self.key_file = None;
        self
    }

    /// The mesh's signing key, from a file of its 32-byte seed (32 bytes or 64 hex digits),
    /// which must have mode 0600.
    pub fn signing_key_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.key_file = Some(path.into());
        self.key = None;
        self
    }

    /// Adds a runtime (the formula runtime is added unless [`MeshBuilder::without_formula`]).
    pub fn runtime(mut self, runtime: Arc<dyn Runtime>) -> Self {
        self.runtimes.push(runtime);
        self
    }

    /// No formula runtime.
    pub fn without_formula(mut self) -> Self {
        self.formula = false;
        self
    }

    /// The clock (default: the system clock).
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// The engine's fuel per request.
    pub fn fuel(mut self, fuel: u64) -> Self {
        self.fuel = Some(fuel);
        self
    }

    /// The licence of the receipts the engine signs.
    pub fn licence(mut self, licence: impl Into<String>) -> Self {
        self.licence = Some(licence.into());
        self
    }

    /// The engine's memo capacity.
    pub fn memo_capacity(mut self, n: usize) -> Self {
        self.memo_capacity = Some(n);
        self
    }

    /// Checks the configuration, builds the engine (its key must be published in the store's
    /// namespace record for the host) and the mesh.
    pub fn build(self) -> Result<Mesh, BuildError> {
        self.config.check().map_err(BuildError::Config)?;
        let keys = Arc::new(KeyCache::default());
        let foreign = Arc::new(Foreign::new(&self.config, self.store.clone(), keys.clone(), self.clock.clone()));
        let mut b = Engine::builder(self.store.clone(), self.config.host.clone()).clock(self.clock.clone());
        b = match (self.key, &self.key_file) {
            (Some(k), _) => b.signing_key(k),
            (None, Some(p)) => {
                crate::config::read_secret_file(p).map_err(BuildError::Config)?;
                b.signing_key_file(p.clone())
            }
            (None, None) => b,
        };
        let mut runtimes = self.runtimes;
        if self.formula {
            runtimes.push(Arc::new(FormulaRuntime::new(self.store.clone())));
        }
        for rt in runtimes {
            b = b.runtime(Arc::new(CrossMesh::new(rt, foreign.clone())));
        }
        if let Some(f) = self.fuel {
            b = b.fuel(f);
        }
        if let Some(l) = self.licence {
            b = b.licence(l);
        }
        if let Some(n) = self.memo_capacity {
            b = b.memo_capacity(n);
        }
        let engine = b.build().map_err(BuildError::Engine)?;
        let config = self.config;
        let inner = Inner {
            pool: Arc::new(Pool::new(config.pool_size)),
            jobs: Jobs::new(config.ttl.job, config.max_jobs),
            limiter: Limiter::new(config.rate.clone()),
            metrics: Metrics::default(),
            engine: Arc::new(engine),
            store: self.store,
            clock: self.clock,
            keys,
            foreign,
            descriptors: Mutex::new(HashMap::new()),
            last_good: Mutex::new(HashMap::new()),
            slices: Mutex::new(Vec::new()),
            started: Instant::now(),
            config,
        };
        Ok(Mesh { inner: Arc::new(inner) })
    }
}

/// The last `ok` receipt served for a cell, for `stale` answers.
struct LastGood {
    receipt: Vec<u8>,
    trust: Trust,
    at: Instant,
}

pub(crate) struct Inner {
    pub(crate) config: Config,
    pub(crate) store: Arc<dyn Store + Send + Sync>,
    pub(crate) engine: Arc<Engine>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) pool: Arc<Pool>,
    pub(crate) jobs: Jobs,
    pub(crate) limiter: Limiter,
    pub(crate) metrics: Metrics,
    pub(crate) keys: Arc<KeyCache>,
    pub(crate) foreign: Arc<Foreign>,
    descriptors: Mutex<HashMap<String, Arc<FunctionDescriptor>>>,
    last_good: Mutex<HashMap<String, LastGood>>,
    #[allow(clippy::type_complexity)]
    pub(crate) slices: Mutex<Vec<(String, String, Arc<Vec<datumesh_store::SliceValue>>)>>,
    pub(crate) started: Instant,
}

/// A Datumesh mesh: the engine behind the endpoints. Cheap to clone (shared).
#[derive(Clone)]
pub struct Mesh {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for Mesh {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mesh").field("host", &self.inner.config.host).finish_non_exhaustive()
    }
}

/// Bindings looked up once per name and asof within one batch.
pub(crate) struct Memo<'a> {
    inner: &'a Inner,
    seen: Mutex<HashMap<(String, String), Option<Bound>>>,
}

impl<'a> Memo<'a> {
    pub(crate) fn new(inner: &'a Inner) -> Memo<'a> {
        Memo { inner, seen: Mutex::new(HashMap::new()) }
    }
}

impl Bindings for Memo<'_> {
    fn binding(&self, fn_name: &str, asof: &str) -> Result<Option<Bound>, ErrorCode> {
        let k = (fn_name.to_string(), asof.to_string());
        if let Some(b) = self.seen.lock().unwrap_or_else(|p| p.into_inner()).get(&k) {
            return Ok(b.clone());
        }
        let b = self.inner.binding(fn_name, asof)?;
        self.seen.lock().unwrap_or_else(|p| p.into_inner()).insert(k, b.clone());
        Ok(b)
    }
}

impl Bindings for Inner {
    fn binding(&self, fn_name: &str, asof: &str) -> Result<Option<Bound>, ErrorCode> {
        let mut failed = None;
        if self.foreign.is_foreign(fn_name) {
            if let Err(e) = self.foreign.ensure_name(fn_name) {
                tracing::warn!(target: "datumesh_http", name = fn_name, detail = %e.detail, "cross-mesh binding");
                failed = Some(e.code);
            }
        }
        let b = self.store.resolve_binding(fn_name, asof).map_err(|e| {
            tracing::warn!(target: "datumesh_http", error = %e, "resolve_binding");
            ErrorCode::InputUnavailable
        })?;
        // The other mesh is unreachable: a binding imported before still names the cell
        // (whose answer is then stale or an error), else the name cannot be resolved.
        if let (Some(code), None) = (failed, &b) {
            return Err(code);
        }
        match b {
            None => Ok(None),
            Some(b) => {
                let d = self.descriptor(&b.fn_hash)?;
                Ok(Some(Bound { fn_hash: b.fn_hash, since: b.tx, descriptor: d }))
            }
        }
    }
}

/// The engine's error as an envelope code (the engine's table, "For the HTTP layer").
fn engine_code(e: &EngineError) -> (bool, ErrorCode) {
    match e {
        EngineError::Invalid(e) if envelope::is_request_code(e.code()) => (true, e.code()),
        EngineError::Invalid(e) if envelope::is_error_code(e.code()) => (false, e.code()),
        EngineError::NameUnresolved { .. } => (false, ErrorCode::FnUnavailable),
        EngineError::UndeclaredInput { .. } => (false, ErrorCode::InputUndeclared),
        EngineError::Sign(e) if envelope::is_error_code(e.code()) => (false, e.code()),
        EngineError::Store(_) => (false, ErrorCode::InputUnavailable),
        _ => (false, ErrorCode::EvalUnavailable),
    }
}

impl Inner {
    pub(crate) fn now(&self) -> String {
        self.clock.now()
    }

    /// A descriptor by hash (cached: content-addressed).
    pub(crate) fn descriptor(&self, fn_hash: &str) -> Result<Arc<FunctionDescriptor>, ErrorCode> {
        if let Some(d) = self.descriptors.lock().unwrap_or_else(|p| p.into_inner()).get(fn_hash) {
            return Ok(d.clone());
        }
        let bytes = self.store.descriptor(fn_hash).ok_or(ErrorCode::FnUnavailable)?;
        let d = Arc::new(FunctionDescriptor::from_json(&bytes).map_err(|_| ErrorCode::FnUnavailable)?);
        let mut m = self.descriptors.lock().unwrap_or_else(|p| p.into_inner());
        if m.len() > 10_000 {
            m.clear();
        }
        m.insert(fn_hash.to_string(), d.clone());
        Ok(d)
    }

    /// The reply to a request the call parser rejected.
    pub(crate) fn rejection(&self, r: Rejection, asof: Option<&str>) -> Reply {
        match r {
            Rejection::BadRequest(c) => envelope::bad_request_reply(c),
            Rejection::NotCovered => {
                let _ = asof;
                Reply::json(404, envelope::not_covered(None), Cache::Public(self.config.ttl.latest_max_age.as_secs()))
            }
            Rejection::Error(c) => envelope::error_reply(c),
        }
    }

    /// The licence and payment rules for a target and principal: a refusal, and whether the
    /// answer depends on who asks.
    fn policy(&self, target: &Target, principal: Option<&Principal>) -> (Option<Reply>, bool) {
        match self.config.policy.rule(&target.fn_name()) {
            None => (None, false),
            Some(Rule::Principals(ps)) => {
                if principal.is_some_and(|p| ps.contains(&p.id)) {
                    (None, true)
                } else {
                    (Some(Reply::json(403, envelope::unlicensed(), Cache::Private)), true)
                }
            }
            Some(Rule::Price { amount, currency }) => {
                let authorise = format!("{}/pay/{}", self.config.origin(), target.cell.id());
                (Some(Reply::json(402, envelope::payment_required(amount, currency, &authorise), Cache::Private)), true)
            }
        }
    }

    /// Answers a `/cell` request (blocking; no budget: see [`Mesh::cell`]). `on_target` is
    /// told the request description as soon as it is known (for `pending`).
    pub(crate) fn answer(
        &self,
        rest: &str,
        principal: Option<&Principal>,
        bindings: &dyn Bindings,
        now: &str,
        on_target: &mut dyn FnMut(&Request),
    ) -> Reply {
        let target = match call::resolve(rest, now, &self.config.aliases, bindings, false) {
            Ok(t) => t,
            Err(r) => return self.rejection(r, None),
        };
        if let Ok(req) = Request::new(&target.cell.id(), target.asof()) {
            on_target(&req);
        }
        let (refused, private) = self.policy(&target, principal);
        if let Some(r) = refused {
            return r;
        }
        self.answer_target(&target, now, private)
    }

    /// Answers a resolved target as of its asof (or `now`).
    pub(crate) fn answer_target(&self, target: &Target, now: &str, private: bool) -> Reply {
        let a = target.asof().unwrap_or(now).to_string();
        let stored = target.bound.descriptor.lang == "stored";
        if let Err(e) = self.foreign.prepare(&target.cell, stored, target.asof()) {
            return self.unavailable(target, e);
        }
        let mut attempt = 0;
        loop {
            let r = self.engine.get_cell(&target.cell, Some(&a));
            let inval = self.foreign.take_invalidated();
            for id in &inval {
                self.engine.stored_version_added(id);
            }
            if !inval.is_empty() && attempt < 2 {
                attempt += 1;
                continue;
            }
            return match r {
                Ok(Answer::Receipt(bytes)) => {
                    let Ok(receipt) = Receipt::from_json(&bytes) else {
                        return envelope::error_reply(ErrorCode::EvalUnavailable);
                    };
                    if attempt < 2 {
                        match self.foreign.refresh_deps(receipt.hash(), &a) {
                            Ok(true) => {
                                for id in self.foreign.take_invalidated() {
                                    self.engine.stored_version_added(&id);
                                }
                                attempt += 1;
                                continue;
                            }
                            Ok(false) => {}
                            Err(e) => return self.unavailable(target, e),
                        }
                    }
                    let trust = envelope::trust_of(&receipt, &self.config.host);
                    if target.asof().is_none() {
                        let mut lg = self.last_good.lock().unwrap_or_else(|p| p.into_inner());
                        if lg.len() > 100_000 {
                            lg.clear();
                        }
                        lg.insert(target.cell.id(), LastGood { receipt: bytes.clone(), trust, at: Instant::now() });
                    }
                    let mut reply = Reply::json(200, envelope::ok(&bytes, trust), Cache::Immutable);
                    self.cache_rules(&mut reply, target, &a, now, private);
                    reply
                }
                Ok(Answer::NotCovered { fault }) => {
                    let mut reply = Reply::json(404, envelope::not_covered(fault), Cache::Public(0));
                    self.cache_rules(&mut reply, target, &a, now, private);
                    if target.asof().is_some() {
                        reply.cache = if private {
                            Cache::Private
                        } else {
                            Cache::Public(self.config.ttl.latest_max_age.as_secs())
                        };
                    }
                    reply
                }
                Ok(_) => envelope::error_reply(ErrorCode::EvalUnavailable),
                Err(e) => {
                    let (bad, code) = engine_code(&e);
                    tracing::info!(target: "datumesh_http", error = %e, code = code.as_str(), "engine");
                    if bad {
                        envelope::bad_request_reply(code)
                    } else if code == ErrorCode::InputUnavailable {
                        self.unavailable(target, ForeignError { code, detail: e.to_string() })
                    } else {
                        envelope::error_reply(code)
                    }
                }
            };
        }
    }

    fn cache_rules(&self, reply: &mut Reply, target: &Target, a: &str, now: &str, private: bool) {
        let latest = self.config.ttl.latest_max_age.as_secs();
        reply.cache = if private {
            Cache::Private
        } else {
            match target.asof() {
                Some(x) if x <= now => Cache::Immutable,
                Some(_) => Cache::Public(latest),
                None => Cache::Public(latest),
            }
        };
        if target.asof().is_none() {
            reply.canonical = Some(format!("{}/cell/{}", self.config.origin(), target.rest_asof(a)));
        }
    }

    /// The answer when something the mesh depends on failed: `stale` with the last receipt
    /// it served for the cell, when it has one that answers the request; else `error`.
    fn unavailable(&self, target: &Target, e: ForeignError) -> Reply {
        tracing::info!(target: "datumesh_http", code = e.code.as_str(), detail = %e.detail, "unavailable");
        let lg = self.last_good.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(g) = lg.get(&target.cell.id()) {
            let answers = match (target.asof(), Receipt::from_json(&g.receipt)) {
                (None, Ok(_)) => true,
                (Some(a), Ok(r)) => r.asof() <= a,
                _ => false,
            };
            if answers {
                let age = g.at.elapsed().as_secs();
                return Reply::json(200, envelope::stale(&g.receipt, g.trust, age), Cache::NoStore);
            }
        }
        envelope::error_reply(e.code)
    }

    /// The keys to verify `receipt` with: the documents of its signer and of the signers of
    /// its inputs and slice members held in the store, from the cache or the store's
    /// namespace records.
    pub(crate) fn keys_for_receipt(&self, receipt: &[u8]) -> Keys {
        let mut hosts = std::collections::BTreeSet::new();
        if let Ok(v) = json::parse(receipt) {
            if let Some(h) = v.get("computed_by").and_then(Value::as_str) {
                hosts.insert(h.split('#').next().unwrap_or("").to_string());
            }
            if let Some(Value::Array(ins)) = v.get("inputs") {
                let mut members = Vec::new();
                for i in ins.iter().filter_map(Value::as_str).take(10_000) {
                    if i.starts_with("slice:") {
                        if let Some(m) =
                            self.store.manifest(i).and_then(|m| datumesh_core::Manifest::from_json(&m).ok())
                        {
                            members.extend(m.members().iter().take(10_000).cloned());
                        }
                    } else {
                        members.push(i.to_string());
                    }
                }
                for h in members.iter().take(20_000) {
                    if let Some(r) = self.store.receipt(h).and_then(|b| Receipt::from_json(&b).ok()) {
                        hosts.insert(r.signer_host().to_string());
                    }
                }
            }
        }
        let mut keys = Keys::new();
        for h in hosts {
            if let Some(r) = self.keys.result(&h) {
                keys.insert_result(&h, r);
            } else if let Some(doc) = self.key_doc(&h) {
                keys.insert(doc);
            }
        }
        keys
    }

    /// A host's key document: the cache's, else the store's namespace record.
    fn key_doc(&self, host: &str) -> Option<KeyDoc> {
        if let Some((d, _, _)) = self.keys.get(host) {
            return Some(d);
        }
        let ns = self.store.namespace(host).ok()??;
        keys_from_response(host, 200, Some(&ns.keydoc)).ok()
    }
}

/// What `POST /verify` reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    /// The result as JSON (`ok`, `level`, `code`, `hash`, `computed_by`, `path`, `detail`).
    pub body: Vec<u8>,
}

impl Mesh {
    /// A builder for a mesh on `store` with `config`.
    pub fn builder(config: Config, store: Arc<dyn Store + Send + Sync>) -> MeshBuilder {
        MeshBuilder {
            config,
            store,
            key: None,
            key_file: None,
            runtimes: Vec::new(),
            formula: true,
            clock: Arc::new(SystemClock),
            fuel: None,
            licence: None,
            memo_capacity: None,
        }
    }

    /// The configuration.
    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    /// The engine.
    pub fn engine(&self) -> &Arc<Engine> {
        &self.inner.engine
    }

    /// The cross-mesh resolver.
    pub fn foreign(&self) -> &Foreign {
        &self.inner.foreign
    }

    /// The store.
    pub fn store(&self) -> &Arc<dyn Store + Send + Sync> {
        &self.inner.store
    }

    /// The key documents the mesh holds for other hosts (fetched, or inserted here): what
    /// `POST /verify` verifies with, besides the store's namespace records.
    pub fn keys(&self) -> &KeyCache {
        &self.inner.keys
    }

    /// Tells the engine that a stored version was added for `cell_id` outside the mesh.
    pub fn stored_version_added(&self, cell_id: &str) {
        self.inner.engine.stored_version_added(cell_id);
        self.inner.slices.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }

    /// The envelope for `rest` (a call or a cell URI's rest, query included, as received at
    /// `/cell/`), for `principal`. Blocking, and without a budget: what a `/cell` request
    /// gets when it finishes within its budget, or its job's result.
    pub fn cell(&self, rest: &str, principal: Option<&Principal>) -> Reply {
        let now = self.inner.now();
        self.inner.answer(rest, principal, &*self.inner, &now, &mut |_| {})
    }

    /// The envelopes of a batch, in order (blocking, no budget).
    pub fn cells(&self, rests: &[String], principal: Option<&Principal>) -> Vec<Reply> {
        let now = self.inner.now();
        let memo = Memo::new(&self.inner);
        rests.iter().map(|r| self.inner.answer(r, principal, &memo, &now, &mut |_| {})).collect()
    }

    /// Resolves a request to its cell, as `/cell` does (or to its slice, with `slice`).
    pub fn resolve(&self, rest: &str, slice: bool) -> Result<Target, Rejection> {
        let now = self.inner.now();
        call::resolve(rest, &now, &self.inner.config.aliases, &*self.inner, slice)
    }

    /// Verifies a receipt (SPEC 10.1) with the mesh's keys; at level `recomputed` with the
    /// engine as the resolver unless `signed_only`; `blob` is content for step 7.7.
    pub fn verify(&self, receipt: &[u8], signed_only: bool, blob: Option<&[u8]>) -> Verified {
        let keys = self.inner.keys_for_receipt(receipt);
        let resolver: Option<&dyn datumesh_core::Resolver> =
            if signed_only { None } else { Some(&*self.inner.engine as &dyn datumesh_core::Resolver) };
        let r = verify(receipt, &keys, resolver, blob);
        let s = |v: Option<&str>| v.map(|x| Value::String(x.to_string())).unwrap_or(Value::Null);
        let body = Value::Object(vec![
            ("blob_checked".into(), Value::Bool(r.blob_checked)),
            ("code".into(), s(r.code.map(|c| c.as_str()))),
            ("computed_by".into(), s(r.computed_by.as_deref())),
            ("detail".into(), Value::String(r.detail.clone())),
            ("hash".into(), s(r.hash.as_deref())),
            ("level".into(), s(r.level.map(|l| l.as_str()))),
            ("ok".into(), Value::Bool(r.ok)),
            ("path".into(), Value::String(r.path.clone())),
        ]);
        Verified { body: body.to_jcs() }
    }

    /// The mesh document (SPEC 9.6) this mesh serves.
    pub fn mesh_document(&self) -> Vec<u8> {
        let o = self.inner.config.origin();
        let ep: BTreeMap<&str, String> = [
            ("blob", format!("{o}/blob/{{sha256}}")),
            ("cell", format!("{o}/cell/{{cell}}")),
            ("cells", format!("{o}/cells")),
            ("descriptor", format!("{o}/descriptor/{{hash}}")),
            ("explain", format!("{o}/explain/{{cell}}")),
            ("find", format!("{o}/find")),
            ("jobs", format!("{o}/jobs/{{job}}")),
            ("manifest", format!("{o}/manifest/{{slice_id}}/{{members_hash}}")),
            ("receipt", format!("{o}/receipt/{{hash}}")),
            ("slice", format!("{o}/slice/{{cell}}")),
            ("verify", format!("{o}/verify")),
        ]
        .into_iter()
        .collect();
        Value::Object(vec![
            ("bindings".into(), Value::String(format!("{o}/bindings"))),
            (
                "endpoints".into(),
                Value::Object(ep.into_iter().map(|(k, v)| (k.to_string(), Value::String(v))).collect()),
            ),
            ("host".into(), Value::String(self.inner.config.host.clone())),
            ("proto".into(), Value::String("datumesh/1".into())),
        ])
        .to_jcs()
    }

    /// The key document this mesh serves: the store's namespace record for its host.
    pub fn key_document(&self) -> Option<Vec<u8>> {
        self.inner.store.namespace(&self.inner.config.host).ok().flatten().map(|n| n.keydoc)
    }

    /// The Prometheus text of `/metrics`.
    pub fn metrics(&self) -> String {
        crate::http::metrics_text(&self.inner)
    }

    /// The number of requests counted for a route template (e.g. `/cell/{cell}`).
    pub fn requests_of(&self, route: &str) -> u64 {
        self.inner.metrics.requests_of(route)
    }

    /// The host of a function name (its first segment).
    pub fn host_of(name: &str) -> &str {
        host_of(name)
    }

    /// How long the mesh has run.
    pub fn uptime(&self) -> Duration {
        self.inner.started.elapsed()
    }
}
