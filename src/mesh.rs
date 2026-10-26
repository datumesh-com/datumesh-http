//! [`Mesh`]: the engine, its store and everything the endpoints share; the blocking
//! operations each endpoint is built on, usable as library calls.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use datumesh_core::json::{self, Value};
use datumesh_core::{
    keys_from_response, verify, ErrorCode, FunctionDescriptor, KeyDoc, Keys, Receipt, SigningKey, Trust,
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
            infos: (0..32).map(|_| Mutex::new(HashMap::new())).collect(),
            latest: Latest::new(),
            targets: Targets::new(),
            slices: Mutex::new(Vec::new()),
            started: Instant::now(),
            config,
        };
        Ok(Mesh { inner: Arc::new(inner) })
    }
}

/// The last `ok` receipt served for a cell, for `stale` answers.
struct LastGood {
    info: Arc<ReceiptInfo>,
    at: Instant,
}

/// What the mesh needs of a receipt it serves, kept by its bytes (content-addressed: never
/// stale): its hash, `asof`, trust and its `ok` envelope.
pub(crate) struct ReceiptInfo {
    pub(crate) hash: String,
    pub(crate) asof: String,
    pub(crate) receipt: Vec<u8>,
    pub(crate) trust: Trust,
    pub(crate) ok: crate::envelope::Bytes,
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
    infos: Vec<Mutex<HashMap<u64, Arc<ReceiptInfo>>>>,
    latest: Latest,
    targets: Targets,
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
    seen: std::borrow::Cow<'a, Seen>,
}

/// The bindings a batch has looked up (shared by its chunks).
#[derive(Default)]
pub(crate) struct Seen(#[allow(clippy::type_complexity)] RwLock<HashMap<String, Vec<(String, Option<Arc<Bound>>)>>>);

impl Clone for Seen {
    fn clone(&self) -> Seen {
        Seen(RwLock::new(self.0.read().unwrap_or_else(|p| p.into_inner()).clone()))
    }
}

impl<'a> Memo<'a> {
    pub(crate) fn new(inner: &'a Inner) -> Memo<'a> {
        Memo { inner, seen: std::borrow::Cow::Owned(Seen::default()) }
    }

    pub(crate) fn shared(inner: &'a Inner, seen: &'a Seen) -> Memo<'a> {
        Memo { inner, seen: std::borrow::Cow::Borrowed(seen) }
    }
}

impl Bindings for Memo<'_> {
    fn cached(&self, fn_hash: &str, rest: &str) -> Option<Arc<Target>> {
        self.inner.targets.get(fn_hash, rest)
    }

    fn remember(&self, rest: &str, target: &Arc<Target>) {
        self.inner.targets.put(rest, target);
    }

    fn binding(&self, fn_name: &str, asof: &str) -> Result<Option<Arc<Bound>>, ErrorCode> {
        if let Some(v) = self.seen.0.read().unwrap_or_else(|p| p.into_inner()).get(fn_name) {
            if let Some((_, b)) = v.iter().find(|(a, _)| a == asof) {
                return Ok(b.clone());
            }
        }
        let b = Bindings::binding(self.inner, fn_name, asof)?;
        let mut m = self.seen.0.write().unwrap_or_else(|p| p.into_inner());
        let v = m.entry(fn_name.to_string()).or_default();
        if v.len() < 64 {
            v.push((asof.to_string(), b.clone()));
        }
        Ok(b)
    }
}

impl Bindings for Inner {
    fn cached(&self, fn_hash: &str, rest: &str) -> Option<Arc<Target>> {
        self.targets.get(fn_hash, rest)
    }

    fn remember(&self, rest: &str, target: &Arc<Target>) {
        self.targets.put(rest, target);
    }

    fn binding(&self, fn_name: &str, asof: &str) -> Result<Option<Arc<Bound>>, ErrorCode> {
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
                Ok(Some(Arc::new(Bound::new(b.fn_hash, b.tx, d))))
            }
        }
    }
}

/// Latest answers by key: the receipt, the cell id and the asof answered at.
type LatestShard = RwLock<HashMap<String, (Arc<ReceiptInfo>, String, String)>>;

/// The latest answers of local stored cells, as of when they were answered: such an answer
/// stands for every later asof until a version of the cell is added, which the mesh is told
/// of ([`Mesh::stored_version_added`]), as the engine's memo is. It saves the engine's work
/// (the cell id, the memo) on the hottest path: warm gets of stored cells.
struct Latest {
    shards: Vec<LatestShard>,
    by_cell: Mutex<HashMap<String, Vec<String>>>,
}

impl Latest {
    fn new() -> Latest {
        Latest { shards: (0..32).map(|_| RwLock::new(HashMap::new())).collect(), by_cell: Mutex::new(HashMap::new()) }
    }

    fn shard(&self, key: &str) -> &LatestShard {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        &self.shards[(h.finish() % self.shards.len() as u64) as usize]
    }

    /// The answer for `key` as of `a`, if one answered as of `a` or earlier is held.
    fn get(&self, key: &str, a: &str) -> Option<Arc<ReceiptInfo>> {
        let m = self.shard(key).read().unwrap_or_else(|p| p.into_inner());
        let (info, _, at) = m.get(key)?;
        (a >= at.as_str()).then(|| info.clone())
    }

    fn put(&self, key: String, cell_id: String, info: Arc<ReceiptInfo>, at: String) {
        {
            let mut b = self.by_cell.lock().unwrap_or_else(|p| p.into_inner());
            if b.len() > 200_000 {
                b.clear();
                for s in &self.shards {
                    s.write().unwrap_or_else(|p| p.into_inner()).clear();
                }
            }
            b.entry(cell_id.clone()).or_default().push(key.clone());
        }
        self.shard(&key).write().unwrap_or_else(|p| p.into_inner()).insert(key, (info, cell_id, at));
    }

    fn forget(&self, cell_id: &str) {
        let keys = self.by_cell.lock().unwrap_or_else(|p| p.into_inner()).remove(cell_id).unwrap_or_default();
        for k in keys {
            self.shard(&k).write().unwrap_or_else(|p| p.into_inner()).remove(&k);
        }
    }

    fn clear(&self) {
        self.by_cell.lock().unwrap_or_else(|p| p.into_inner()).clear();
        for s in &self.shards {
            s.write().unwrap_or_else(|p| p.into_inner()).clear();
        }
    }
}

/// Cell URI rests already parsed, by function hash (parsing depends on the signature
/// alone): resolving a hot cell costs a binding lookup and no parse.
struct Targets {
    #[allow(clippy::type_complexity)]
    shards: Vec<RwLock<HashMap<String, HashMap<String, Arc<Target>>>>>,
}

impl Targets {
    fn new() -> Targets {
        Targets { shards: (0..32).map(|_| RwLock::new(HashMap::new())).collect() }
    }

    #[allow(clippy::type_complexity)]
    fn shard(&self, rest: &str) -> &RwLock<HashMap<String, HashMap<String, Arc<Target>>>> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        rest.hash(&mut h);
        &self.shards[(h.finish() % self.shards.len() as u64) as usize]
    }

    fn get(&self, fn_hash: &str, rest: &str) -> Option<Arc<Target>> {
        self.shard(rest).read().unwrap_or_else(|p| p.into_inner()).get(rest)?.get(fn_hash).cloned()
    }

    fn put(&self, rest: &str, t: &Arc<Target>) {
        let mut m = self.shard(rest).write().unwrap_or_else(|p| p.into_inner());
        if m.len() >= 4_096 {
            m.clear();
        }
        let v = m.entry(rest.to_string()).or_default();
        if v.len() >= 8 {
            v.clear();
        }
        v.insert(t.bound.fn_hash.clone(), t.clone());
    }
}

/// The key of a cell's last good answer: the function hash and the URI (without asof: the
/// last good answer is kept for requests about now only).
fn last_good_key(t: &Target) -> String {
    t.cell_key().to_string()
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

    /// The `ok` reply with a receipt, remembered for `stale` answers.
    fn ok_reply(
        &self,
        target: &Target,
        info: Arc<ReceiptInfo>,
        a: &str,
        now: &str,
        private: bool,
        link: bool,
    ) -> Reply {
        let mut reply = Reply::json(200, info.ok.clone(), Cache::Immutable);
        if target.asof().is_none() {
            let mut lg = self.last_good.lock().unwrap_or_else(|p| p.into_inner());
            if lg.len() > 100_000 {
                lg.clear();
            }
            lg.insert(last_good_key(target), LastGood { info, at: Instant::now() });
        }
        self.cache_rules(&mut reply, target, a, now, private, link);
        reply
    }

    /// Remembers the answer as of now `a` for a local stored cell, when it is the cell's
    /// latest version (a version with a later `tx` would answer later asofs).
    fn remember_latest(&self, target: &Target, info: &Arc<ReceiptInfo>, a: &str) {
        match self.store.latest(target.cell_id()) {
            Ok(Some(v)) if v.receipt_hash == info.hash => {
                self.latest.put(
                    target.cell_key().to_string(),
                    target.cell_id().to_string(),
                    info.clone(),
                    a.to_string(),
                );
            }
            _ => {}
        }
    }

    /// A stored version was added for `cell_id`: the engine's memo, the latest answers and
    /// the slice cache forget what rests on it.
    pub(crate) fn version_added(&self, cell_id: &str) {
        self.engine.stored_version_added(cell_id);
        self.latest.forget(cell_id);
        self.slices.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }

    /// The receipt's hash, asof, trust and `ok` envelope (cached by its bytes).
    fn receipt_info(&self, bytes: &[u8]) -> Option<Arc<ReceiptInfo>> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut h);
        let key = h.finish();
        let shard = &self.infos[(key % self.infos.len() as u64) as usize];
        if let Some(i) = shard.lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
            if i.receipt == bytes {
                return Some(i.clone());
            }
        }
        let r = Receipt::from_json(bytes).ok()?;
        let trust = envelope::trust_of(&r, &self.config.host);
        let info = Arc::new(ReceiptInfo {
            hash: r.hash().to_string(),
            asof: r.asof().to_string(),
            receipt: bytes.to_vec(),
            trust,
            ok: envelope::ok(bytes, trust).into(),
        });
        let mut m = shard.lock().unwrap_or_else(|p| p.into_inner());
        if m.len() >= 4_096 {
            m.clear();
        }
        m.insert(key, info.clone());
        Some(info)
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
        if self.config.policy.rules.is_empty() {
            return (None, false);
        }
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
                let authorise = format!("{}/pay/{}", self.config.origin(), target.cell_id());
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
        on_target: &mut dyn FnMut(&Target),
    ) -> Reply {
        self.answer_with(rest, principal, bindings, now, on_target, true)
    }

    /// As [`Inner::answer`]; without the canonical link when `link` is false (a batch's
    /// envelopes have no headers).
    pub(crate) fn answer_with(
        &self,
        rest: &str,
        principal: Option<&Principal>,
        bindings: &dyn Bindings,
        now: &str,
        on_target: &mut dyn FnMut(&Target),
        link: bool,
    ) -> Reply {
        let target = match call::resolve(rest, now, &self.config.aliases, bindings, false) {
            Ok(t) => t,
            Err(r) => return self.rejection(r, None),
        };
        on_target(&target);
        let (refused, private) = self.policy(&target, principal);
        if let Some(r) = refused {
            return r;
        }
        self.answer_target(&target, now, private, link)
    }

    /// Answers a resolved target as of its asof (or `now`).
    pub(crate) fn answer_target(&self, target: &Target, now: &str, private: bool, link: bool) -> Reply {
        let a = target.asof().unwrap_or(now);
        let stored = target.bound.descriptor.lang == "stored";
        let local = stored && !self.foreign.is_foreign(&target.cell.ns);
        if local {
            if let Some(info) = self.latest.get(target.cell_key(), a) {
                let mut reply = Reply::json(200, info.ok.clone(), Cache::Immutable);
                self.cache_rules(&mut reply, target, a, now, private, link);
                return reply;
            }
        }
        if let Err(e) = self.foreign.prepare(&target.cell, stored, target.asof()) {
            return self.unavailable(target, e);
        }
        let mut attempt = 0;
        loop {
            let r = self.engine.get_cell(&target.cell, Some(a));
            let inval = self.foreign.take_invalidated();
            for id in &inval {
                self.version_added(id);
            }
            if !inval.is_empty() && attempt < 2 {
                attempt += 1;
                continue;
            }
            return match r {
                Ok(Answer::Receipt(bytes)) => {
                    let Some(info) = self.receipt_info(&bytes) else {
                        return envelope::error_reply(ErrorCode::EvalUnavailable);
                    };
                    if attempt < 2 {
                        match self.foreign.refresh_deps(&info.hash, a) {
                            Ok(true) => {
                                for id in self.foreign.take_invalidated() {
                                    self.version_added(&id);
                                }
                                attempt += 1;
                                continue;
                            }
                            Ok(false) => {}
                            Err(e) => return self.unavailable(target, e),
                        }
                    }
                    if local && target.asof().is_none() {
                        self.remember_latest(target, &info, a);
                    }
                    self.ok_reply(target, info, a, now, private, link)
                }
                Ok(Answer::NotCovered { fault }) => {
                    let mut reply = Reply::json(404, envelope::not_covered(fault), Cache::Public(0));
                    self.cache_rules(&mut reply, target, a, now, private, link);
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

    fn cache_rules(&self, reply: &mut Reply, target: &Target, a: &str, now: &str, private: bool, link: bool) {
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
        if link && target.asof().is_none() {
            reply.canonical = Some(format!("{}/cell/{}", self.config.origin(), target.rest_asof(a)));
        }
    }

    /// The answer when something the mesh depends on failed: `stale` with the last receipt
    /// it served for the cell, when it has one that answers the request; else `error`.
    fn unavailable(&self, target: &Target, e: ForeignError) -> Reply {
        tracing::info!(target: "datumesh_http", code = e.code.as_str(), detail = %e.detail, "unavailable");
        let lg = self.last_good.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(g) = lg.get(&last_good_key(target)) {
            if target.asof().is_none_or(|a| g.info.asof.as_str() <= a) {
                let age = g.at.elapsed().as_secs();
                return Reply::json(200, envelope::stale(&g.info.receipt, g.info.trust, age), Cache::NoStore);
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

fn verified(r: datumesh_core::VerifyResult) -> Verified {
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

    /// Forgets every memoised answer (the engine's memo and the mesh's), for a process that
    /// wrote versions without telling the mesh.
    pub fn clear_memo(&self) {
        self.inner.engine.clear_memo();
        self.inner.latest.clear();
        self.inner.slices.lock().unwrap_or_else(|p| p.into_inner()).clear();
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
        self.inner.version_added(cell_id);
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
    pub fn resolve(&self, rest: &str, slice: bool) -> Result<Arc<Target>, Rejection> {
        let now = self.inner.now();
        call::resolve(rest, &now, &self.inner.config.aliases, &*self.inner, slice)
    }

    /// Verifies a receipt (SPEC 10.1) with the mesh's keys; at level `recomputed` with the
    /// engine as the resolver unless `signed_only`; `blob` is content for step 7.7.
    pub fn verify(&self, receipt: &[u8], signed_only: bool, blob: Option<&[u8]>) -> Verified {
        let keys = self.inner.keys_for_receipt(receipt);
        let resolver: Option<&dyn datumesh_core::Resolver> =
            if signed_only { None } else { Some(&*self.inner.engine as &dyn datumesh_core::Resolver) };
        verified(verify(receipt, &keys, resolver, blob))
    }

    /// Verifies a receipt with the mesh's keys and what the caller offers for step 7: a
    /// `recompute` object as SPEC 11 describes it (`descriptor`, `inputs`, `manifests`,
    /// `lineage`, `value`), the core's `MapResolver` over it. Without one, steps 1-6 only.
    pub fn verify_offered(&self, receipt: &[u8], recompute: Option<&Value>, blob: Option<&[u8]>) -> Verified {
        let mut keys = self.inner.keys_for_receipt(receipt);
        // The signers of the receipts offered for step 7 too.
        let mut hosts = std::collections::BTreeSet::new();
        for (_, r) in recompute.and_then(|rc| rc.get("inputs")).and_then(Value::as_object).unwrap_or(&[]) {
            if let Some(h) = r
                .as_str()
                .and_then(|t| json::parse(t.as_bytes()).ok())
                .and_then(|v| v.get("computed_by").and_then(Value::as_str).map(str::to_string))
            {
                hosts.insert(h.split('#').next().unwrap_or("").to_string());
            }
        }
        for h in hosts {
            if let Some(r) = self.inner.keys.result(&h) {
                keys.insert_result(&h, r);
            } else if let Some(doc) = self.inner.key_doc(&h) {
                keys.insert(doc);
            }
        }
        let Some(rc) = recompute else {
            return verified(verify(receipt, &keys, None, blob));
        };
        let mut m = datumesh_core::MapResolver::new();
        let cell = json::parse(receipt).ok().and_then(|r| r.get("cell").cloned());
        if let (Some(d), Some(h)) =
            (rc.get("descriptor"), cell.as_ref().and_then(|c| c.get("fn_hash")).and_then(Value::as_str))
        {
            m.insert_descriptor(h, d.to_jcs());
        }
        for (h, r) in rc.get("inputs").and_then(Value::as_object).unwrap_or(&[]) {
            m.insert_receipt(h.as_str(), r.as_str().unwrap_or("").as_bytes());
        }
        for (e, t) in rc.get("manifests").and_then(Value::as_object).unwrap_or(&[]) {
            m.insert_manifest(e.as_str(), t.as_str().unwrap_or("").as_bytes());
        }
        let value = rc.get("value").and_then(|v| datumesh_core::Scalar::from_json(&v.to_jcs()).ok());
        if let (Some(value), Some(c)) = (value, cell.as_ref().and_then(|c| datumesh_core::Cell::from_value(c).ok())) {
            m.insert_value(c.id(), value);
            if let Some(Value::Array(l)) = rc.get("lineage") {
                m.insert_lineage(c.id(), l.iter().filter_map(Value::as_str));
            }
        }
        verified(verify(receipt, &keys, Some(&m), blob))
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
