//! The store-less client of other meshes (for `datumesh-local` and for this mesh's cross-mesh
//! reads): fetch a host's key document and mesh document, its bindings, a cell's envelope (by
//! call, URI or pin), slice pages, batches, receipts, descriptors, manifests, blobs and
//! versions, and **check** each with the core: the envelope against the request
//! (`check_answer`, `check_slice_answer`, `check_batch`), every receipt's signature against
//! its signer's key document (fetched from the signer's own host, never from the mesh that
//! served it), and content-addressed bytes against their hash. Nothing is stored; the import
//! into a store is [`crate::foreign::Foreign`]'s, a layer on top.
//!
//! **Blocking.** The client is synchronous (ureq, rustls with the Mozilla roots, no redirects,
//! a time limit per request, bodies bounded): this mesh's engine reads foreign cells from
//! inside an evaluation, which runs on a blocking thread, and `datumesh-local`'s CLI is
//! synchronous too. An async caller runs it on `tokio::task::spawn_blocking`, as the mesh's
//! own handlers do with the engine. [`Client`] is cheap to clone and shares its caches.
//!
//! **Caches.** Key documents (default 300 s; a key id the cached document lacks refetches it,
//! at most every 10 s) and mesh documents (300 s), in memory. Nothing else is cached here.
//!
//! **Allowlist.** With [`ClientConfig::allowed`] set, only those hosts are fetched from
//! (anything else is refused with `E_INPUT_UNAVAILABLE` before any request).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use datumesh_core::json::{self, Value};
use datumesh_core::{
    check_answer, check_batch, check_slice_answer, format_reference, keys_from_response, lexical, parse_reference,
    resolve_mesh, Answer, Batch, ErrorCode, FunctionDescriptor, KeyDoc, Keys, Manifest, MeshDoc, Receipt, Reference,
    Request, Retrieval, Slice, SliceRequest,
};

use crate::foreign::KeyCache;

/// The largest body read from another mesh.
const MAX_BODY: u64 = 64 << 20;
/// The largest key or mesh document read (SPEC 9.3's suggested bound).
const MAX_DOC: u64 = 256 << 10;

/// A failure of the client: the code of the `error` envelope a mesh would give for it, and why.
#[derive(Clone, Debug)]
pub struct ClientError {
    /// `E_INPUT_UNAVAILABLE` (the other mesh is down, answered otherwise, or is not allowed),
    /// `E_MESH_DOC`, `E_KEY_UNAVAILABLE` or `E_KEY_DOC` (resolving its host), `E_ENVELOPE` (an
    /// answer that fails the check), or what checking a receipt gave.
    pub code: ErrorCode,
    /// Why.
    pub detail: String,
}

impl ClientError {
    pub(crate) fn new(code: ErrorCode, detail: impl Into<String>) -> ClientError {
        ClientError { code, detail: detail.into() }
    }
    pub(crate) fn unavailable(detail: impl Into<String>) -> ClientError {
        ClientError::new(ErrorCode::InputUnavailable, detail)
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.detail)
    }
}

impl std::error::Error for ClientError {}

/// The client's configuration.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Origins to use instead of `https://<host>` for some hosts (tests, private networks).
    pub peers: BTreeMap<String, String>,
    /// When set, the only hosts fetched from.
    pub allowed: Option<Vec<String>>,
    /// The time limit of one request (default 10 s).
    pub timeout: Duration,
    /// Key documents' lifetime in the cache (default 300 s).
    pub key_ttl: Duration,
    /// The shortest interval between two fetches of one host's key document (default 10 s).
    pub key_refetch: Duration,
    /// Mesh documents' lifetime in the cache (default 300 s).
    pub mesh_ttl: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        ClientConfig {
            peers: BTreeMap::new(),
            allowed: None,
            timeout: Duration::from_secs(10),
            key_ttl: Duration::from_secs(300),
            key_refetch: Duration::from_secs(10),
            mesh_ttl: Duration::from_secs(300),
        }
    }
}

/// A binding another mesh states (SPEC 9.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteBinding {
    /// The function hash.
    pub fn_hash: String,
    /// When the binding began.
    pub since: String,
}

/// An answer fetched and checked: its HTTP status, its body as received, and the checked answer.
#[derive(Clone, Debug)]
pub struct Fetched {
    /// The HTTP status.
    pub status: u16,
    /// The body.
    pub body: Vec<u8>,
    /// The answer, checked against the request.
    pub answer: Answer,
}

/// One version a mesh lists (SPEC 9.8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionEntry {
    /// The version's `tx`.
    pub tx: String,
    /// Its receipt's `asof`.
    pub asof: String,
    /// Its receipt hash.
    pub receipt: String,
}

/// A page of the versions endpoint (SPEC 9.8).
#[derive(Clone, Debug)]
pub struct Versions {
    /// The cell (4.4).
    pub cell: Value,
    /// The versions, ascending by `tx`.
    pub versions: Vec<VersionEntry>,
    /// The following page's URL.
    pub next: Option<String>,
}

struct Inner {
    agent: ureq::Agent,
    cfg: ClientConfig,
    keys: Arc<KeyCache>,
    meshes: Mutex<HashMap<String, (MeshDoc, Instant)>>,
}

/// The client (see the module documentation).
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("allowed", &self.inner.cfg.allowed).finish_non_exhaustive()
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

fn host_of(name: &str) -> &str {
    name.split('/').next().unwrap_or(name)
}

impl Client {
    /// A client with this configuration.
    pub fn new(cfg: ClientConfig) -> Client {
        Client::with_keys(cfg, Arc::new(KeyCache::default()))
    }

    /// A client sharing a key cache (the mesh's, so `POST /verify` sees what it fetched).
    pub fn with_keys(cfg: ClientConfig, keys: Arc<KeyCache>) -> Client {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(cfg.timeout))
            .user_agent(concat!("datumesh-http/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Client { inner: Arc::new(Inner { agent, cfg, keys, meshes: Mutex::new(HashMap::new()) }) }
    }

    /// The key cache.
    pub fn keys(&self) -> &Arc<KeyCache> {
        &self.inner.keys
    }

    /// Whether `host` may be fetched from.
    pub fn allowed(&self, host: &str) -> bool {
        self.inner.cfg.allowed.as_ref().is_none_or(|a| a.iter().any(|h| h == host))
    }

    fn check_host(&self, host: &str) -> Result<(), ClientError> {
        if lexical::check_dns_name(host, "$").is_err() {
            return Err(ClientError::new(ErrorCode::KeyUnavailable, format!("{host:?} is not a DNS name")));
        }
        if !self.allowed(host) {
            return Err(ClientError::unavailable(format!("{host} is not an allowed host")));
        }
        Ok(())
    }

    /// `GET url` (an `https://<host>/...` URL; the origin replaced for a configured peer):
    /// status and body, at most `limit` bytes.
    pub fn get(&self, url: &str, limit: u64) -> Result<(u16, Vec<u8>), String> {
        self.request(url, None, limit)
    }

    fn request(&self, url: &str, post: Option<&[u8]>, limit: u64) -> Result<(u16, Vec<u8>), String> {
        let mut target = url.to_string();
        if let Some(rest) = url.strip_prefix("https://") {
            let host = rest.split(['/', '?']).next().unwrap_or("");
            if !self.allowed(host) {
                return Err(format!("{host} is not an allowed host"));
            }
            if let Some(origin) = self.inner.cfg.peers.get(host) {
                target = format!("{origin}{}", &rest[host.len()..]);
            }
        }
        let resp = match post {
            None => self.inner.agent.get(&target).call(),
            Some(b) => self.inner.agent.post(&target).header("content-type", "application/json").send(b),
        };
        let mut resp = resp.map_err(|e| format!("{url}: {e}"))?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| format!("{url}: {e}"))?;
        Ok((status, body))
    }

    /// The key document of `host` (SPEC 9.3), from the cache while it is fresh, else fetched;
    /// `refresh` fetches it again unless it was fetched within the refetch interval.
    pub fn key_document(&self, host: &str, refresh: bool) -> Result<KeyDoc, ClientError> {
        if let Some((doc, _, at)) = self.inner.keys.get(host) {
            let age = at.elapsed();
            if (!refresh && age < self.inner.cfg.key_ttl) || (refresh && age < self.inner.cfg.key_refetch) {
                return Ok(doc);
            }
        }
        self.check_host(host)?;
        let url = format!("https://{host}/.well-known/datumesh.json");
        let (status, body) = self.get(&url, MAX_DOC).map_err(|e| ClientError::new(ErrorCode::KeyUnavailable, e))?;
        let doc = keys_from_response(host, status, if status == 200 { Some(&body) } else { None })
            .map_err(|e| ClientError::new(e.code(), e.to_string()))?;
        self.inner.keys.insert(host, &body).map_err(|e| ClientError::new(e.code(), e.to_string()))?;
        Ok(doc)
    }

    /// The bytes of `host`'s key document as last fetched.
    pub fn key_document_bytes(&self, host: &str) -> Option<Vec<u8>> {
        self.inner.keys.get(host).map(|(_, b, _)| b)
    }

    /// The mesh document of `host`, resolved with its key document (SPEC 9.6), cached.
    pub fn resolve(&self, host: &str) -> Result<MeshDoc, ClientError> {
        if let Some((m, at)) = self.inner.meshes.lock().unwrap_or_else(|p| p.into_inner()).get(host) {
            if at.elapsed() < self.inner.cfg.mesh_ttl {
                return Ok(m.clone());
            }
        }
        self.check_host(host)?;
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
            ClientError::new(e.code(), detail)
        })?;
        if let Ok((_, body)) = &keys {
            let _ = self.inner.keys.insert(host, body);
        }
        let m = resolved.mesh;
        self.inner
            .meshes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(host.to_string(), (m.clone(), Instant::now()));
        Ok(m)
    }

    fn endpoint(&self, host: &str, name: &str, values: &[(&str, &str)]) -> Result<String, ClientError> {
        self.resolve(host)?
            .url(name, values)
            .ok_or_else(|| ClientError::new(ErrorCode::MeshDoc, format!("{host} has no {name} endpoint")))
    }

    /// The binding of `name` that `host`'s bindings resource states as of `asof` (now when
    /// `None`), SPEC 9.7.
    pub fn binding(&self, host: &str, name: &str, asof: Option<&str>) -> Result<Option<RemoteBinding>, ClientError> {
        let mesh = self.resolve(host)?;
        let mut url = format!("{}?name={}", mesh.bindings, pct(name));
        if let Some(a) = asof {
            url.push_str(&format!("&asof={}", pct(a)));
        }
        let (status, body) = self.get(&url, MAX_DOC).map_err(ClientError::unavailable)?;
        if status != 200 {
            return Err(ClientError::unavailable(format!("{url}: status {status}")));
        }
        let v = json::parse(&body).map_err(|e| ClientError::unavailable(format!("{url}: {e}")))?;
        let Some(b) = v.get("bindings").and_then(|b| b.get(name)) else {
            if v.get("bindings").and_then(Value::as_object).is_none() {
                return Err(ClientError::unavailable(format!("{url}: no bindings object")));
            }
            return Ok(None);
        };
        let h = b.get("fn_hash").and_then(Value::as_str).unwrap_or("");
        let since = b.get("since").and_then(Value::as_str).unwrap_or("");
        if lexical::check_hex(h, 64, "$").is_err() || lexical::check_instant(since, "$").is_err() {
            return Err(ClientError::unavailable(format!("{url}: malformed binding")));
        }
        if asof.is_some_and(|a| since > a) {
            return Err(ClientError::unavailable(format!("{url}: a binding since after the asof")));
        }
        Ok(Some(RemoteBinding { fn_hash: h.to_string(), since: since.to_string() }))
    }

    /// The descriptor `fn_hash` names, from `host` (its hash checked).
    pub fn descriptor(&self, host: &str, fn_hash: &str) -> Result<Vec<u8>, ClientError> {
        let url = self.endpoint(host, "descriptor", &[("hash", fn_hash)])?;
        let (status, body) = self.get(&url, MAX_BODY).map_err(ClientError::unavailable)?;
        if status != 200 {
            return Err(ClientError::new(ErrorCode::FnUnavailable, format!("{url}: status {status}")));
        }
        let d = FunctionDescriptor::from_json(&body).map_err(|e| ClientError::new(e.code(), e.to_string()))?;
        if d.hash() != fn_hash {
            return Err(ClientError::new(ErrorCode::FnHashMismatch, format!("{url} gave the descriptor {}", d.hash())));
        }
        Ok(body)
    }

    /// The keys of the signers named in a body (a receipt, an envelope, a page or a batch):
    /// each signer's key document, from its own host.
    pub fn keys_for_body(&self, body: &[u8]) -> Keys {
        let mut hosts = BTreeSet::new();
        fn walk(v: &Value, hosts: &mut BTreeSet<String>, depth: usize) {
            if depth > 8 {
                return;
            }
            match v {
                Value::Object(m) => {
                    for (k, x) in m {
                        if k == "computed_by" {
                            if let Some(s) = x.as_str() {
                                hosts.insert(s.split('#').next().unwrap_or("").to_string());
                            }
                        } else {
                            walk(x, hosts, depth + 1);
                        }
                    }
                }
                Value::Array(a) => a.iter().for_each(|x| walk(x, hosts, depth + 1)),
                _ => {}
            }
        }
        if let Ok(v) = json::parse(body) {
            walk(&v, &mut hosts, 0);
        }
        let mut keys = Keys::new();
        for h in hosts {
            match self.key_document(&h, false) {
                Ok(d) => keys.insert(d),
                Err(e) => keys.insert_result(&h, Err(datumesh_core::Error::new(e.code, "$", e.detail))),
            }
        }
        keys
    }

    /// Runs `check` with the signers' keys, fetching them again once when a key id is unknown
    /// (a rotation).
    fn checked<T>(&self, body: &[u8], check: impl Fn(&Keys) -> datumesh_core::Result<T>) -> Result<T, ClientError> {
        let keys = self.keys_for_body(body);
        match check(&keys) {
            Err(e) if matches!(e.code(), ErrorCode::KeyUnknown | ErrorCode::KeyNotValid) => {
                let mut hosts = BTreeSet::new();
                if let Ok(v) = json::parse(body) {
                    collect_signers(&v, &mut hosts);
                }
                for h in hosts {
                    let _ = self.key_document(&h, true);
                }
                check(&self.keys_for_body(body)).map_err(|e| ClientError::new(e.code(), e.to_string()))
            }
            r => r.map_err(|e| ClientError::new(e.code(), e.to_string())),
        }
    }

    /// A receipt by its hash from `host`: its hash checked and its signature verified (SPEC
    /// 10.1 steps 1-6) against its signer's key document.
    pub fn receipt(&self, host: &str, hash: &str) -> Result<Vec<u8>, ClientError> {
        let url = self.endpoint(host, "receipt", &[("hash", hash)])?;
        let (status, body) = self.get(&url, MAX_BODY).map_err(ClientError::unavailable)?;
        if status != 200 {
            return Err(ClientError::unavailable(format!("{url}: status {status}")));
        }
        let h = self.checked(&body, |keys| {
            let r = datumesh_core::verify(&body, keys, None, None);
            match r.error() {
                Some(e) => Err(e),
                None => Ok(r.hash.unwrap_or_default()),
            }
        })?;
        if h != hash {
            return Err(ClientError::unavailable(format!("{url} gave the receipt {h}")));
        }
        Ok(body)
    }

    /// A slice manifest from `host` (its entry checked).
    pub fn manifest(&self, host: &str, slice_id: &str, members_hash: &str) -> Result<Vec<u8>, ClientError> {
        let url = self.endpoint(host, "manifest", &[("slice_id", slice_id), ("members_hash", members_hash)])?;
        let (status, body) = self.get(&url, MAX_BODY).map_err(ClientError::unavailable)?;
        if status != 200 {
            return Err(ClientError::unavailable(format!("{url}: status {status}")));
        }
        let m = Manifest::from_json(&body).map_err(|e| ClientError::new(e.code(), e.to_string()))?;
        if m.entry() != format!("slice:{slice_id}:{members_hash}") {
            return Err(ClientError::unavailable(format!("{url} gave the manifest {}", m.entry())));
        }
        Ok(body)
    }

    /// Blob content by SHA-256 from `host` (its hash checked).
    pub fn blob(&self, host: &str, sha256: &str) -> Result<Vec<u8>, ClientError> {
        let url = self.endpoint(host, "blob", &[("sha256", sha256)])?;
        let (status, body) = self.get(&url, MAX_BODY).map_err(ClientError::unavailable)?;
        if status != 200 {
            return Err(ClientError::unavailable(format!("{url}: status {status}")));
        }
        if datumesh_core::sha256_hex(&body) != sha256 {
            return Err(ClientError::new(ErrorCode::BlobMismatch, format!("{url}: content of another hash")));
        }
        Ok(body)
    }

    /// The envelope for `rest` (a cell URI's rest or a call, as the cell endpoint takes it)
    /// from `host`, checked against `request` (5.5; the host asked is added to it).
    pub fn cell(&self, host: &str, rest: &str, request: &Request) -> Result<Fetched, ClientError> {
        let url = self.endpoint(host, "cell", &[("cell", rest)])?;
        let (status, body) = self.get(&url, MAX_BODY).map_err(ClientError::unavailable)?;
        let req =
            request.clone().with_mesh(host).map_err(|e| ClientError::new(ErrorCode::Envelope, format!("{e:?}")))?;
        let answer = self.checked(&body, |keys| check_answer(status, &body, keys, &req))?;
        Ok(Fetched { status, body, answer })
    }

    /// The envelope of a reference (pinned or not) from `host`: its URI written by the core,
    /// the request it describes made from it (the cell id needs the function hash: the pin,
    /// else the binding `host` states as of the reference's asof).
    pub fn reference(&self, host: &str, reference: &Reference) -> Result<Fetched, ClientError> {
        let fn_hash = match &reference.fn_hash {
            Some(h) => h.clone(),
            None => {
                let name = format!("{}/{}", reference.ns, reference.fn_name);
                self.binding(host, &name, reference.asof.as_deref())?
                    .ok_or_else(|| ClientError::new(ErrorCode::NameUnbound, name.clone()))?
                    .fn_hash
            }
        };
        let cell = reference.resolve(&fn_hash).map_err(|e| ClientError::new(e.code(), e.to_string()))?;
        let mut request = Request::new(&cell.id(), reference.asof.as_deref())
            .map_err(|e| ClientError::new(ErrorCode::Envelope, format!("{e:?}")))?;
        if let Some(p) = &reference.fn_hash {
            request = request.with_pin(p).map_err(|e| ClientError::new(ErrorCode::Envelope, format!("{e:?}")))?;
        }
        let uri = format_reference(reference);
        self.cell(host, &uri["cell://".len()..], &request)
    }

    /// The slice a slice URI's rest denotes on `host`: the descriptor of its pin, or of the
    /// name's binding (as of its asof), parsed against.
    pub fn slice_of(&self, host: &str, rest: &str) -> Result<(Slice, Reference), ClientError> {
        let uri = format!("cell://{rest}");
        let path = rest.split('?').next().unwrap_or(rest);
        let query = rest.split_once('?').map(|(_, q)| q).unwrap_or("");
        let pin = query.split('&').find_map(|p| p.strip_prefix("fn_hash=")).map(str::to_string);
        let asof = query.split('&').next_back().and_then(|p| p.strip_prefix("asof=")).map(str::to_string);
        let fn_hash = match pin {
            Some(p) => p,
            None => {
                self.binding(host, path, asof.as_deref())?
                    .ok_or_else(|| ClientError::new(ErrorCode::NameUnbound, path.to_string()))?
                    .fn_hash
            }
        };
        let d = FunctionDescriptor::from_json(&self.descriptor(host, &fn_hash)?)
            .map_err(|e| ClientError::new(e.code(), e.to_string()))?;
        let r =
            parse_reference(&uri, &d.signature.to_value()).map_err(|e| ClientError::new(e.code(), e.to_string()))?;
        let cell = r.resolve(&fn_hash).map_err(|e| ClientError::new(e.code(), e.to_string()))?;
        let slice = Slice::from_value(&cell.to_value()).map_err(|e| ClientError::new(e.code(), e.to_string()))?;
        Ok((slice, r))
    }

    /// A page of a slice from `host` (5.6): the first page of `rest` (a slice URI's rest), or,
    /// with `next`, the page that URL names (which must be on `host`), checked against the
    /// slice `rest` denotes and, for a following page, the cursor `after`.
    pub fn slice_page(
        &self,
        host: &str,
        rest: &str,
        next: Option<&str>,
        after: Option<BTreeMap<String, datumesh_core::Scalar>>,
    ) -> Result<Fetched, ClientError> {
        let (slice, r) = self.slice_of(host, rest)?;
        let url = match next {
            Some(n) => {
                if !n.starts_with(&format!("https://{host}/")) {
                    return Err(ClientError::new(ErrorCode::Envelope, format!("{n} is not on {host}")));
                }
                n.to_string()
            }
            None => self.endpoint(host, "slice", &[("cell", rest)])?,
        };
        let (status, body) = self.get(&url, MAX_BODY).map_err(ClientError::unavailable)?;
        let mut req = SliceRequest::new(slice.id(), r.asof.as_deref())
            .and_then(|q| q.with_mesh(host))
            .map_err(|e| ClientError::new(ErrorCode::Envelope, format!("{e:?}")))?;
        if let Some(p) = &r.fn_hash {
            req = req.with_pin(p).map_err(|e| ClientError::new(ErrorCode::Envelope, format!("{e:?}")))?;
        }
        if let Some(a) = after {
            req = req.with_after(a);
        }
        let answer = self.checked(&body, |keys| check_slice_answer(status, &body, keys, &req))?;
        Ok(Fetched { status, body, answer })
    }

    /// The first page of a slice (see [`Client::slice_page`]).
    pub fn slice(&self, host: &str, rest: &str, next: Option<&str>) -> Result<Fetched, ClientError> {
        self.slice_page(host, rest, next, None)
    }

    /// A batch (5.7) to `host`: the requests' rests and the request each describes, checked as
    /// a whole and item by item.
    pub fn batch(&self, host: &str, items: &[String], requests: &[Request]) -> Result<Batch, ClientError> {
        let url = self.endpoint(host, "cells", &[])?;
        let body = Value::Array(items.iter().map(|i| Value::String(i.clone())).collect()).to_jcs();
        let (status, resp) = self.request(&url, Some(&body), MAX_BODY).map_err(ClientError::unavailable)?;
        let reqs: Vec<Request> =
            requests.iter().map(|r| r.clone().with_mesh(host).unwrap_or_else(|_| r.clone())).collect();
        self.checked(&resp, |keys| check_batch(status, &resp, keys, &reqs))
    }

    /// The versions of a stored cell from `host` (9.8): `rest` is its URI's rest without asof.
    pub fn versions(&self, host: &str, rest: &str) -> Result<Versions, ClientError> {
        let url = self.endpoint(host, "versions", &[("cell", rest)])?;
        self.versions_at(&url)
    }

    /// A page of the versions endpoint at `url` (a `next`).
    pub fn versions_at(&self, url: &str) -> Result<Versions, ClientError> {
        let (status, body) = self.get(url, MAX_BODY).map_err(ClientError::unavailable)?;
        let v = json::parse(&body).map_err(|e| ClientError::unavailable(format!("{url}: {e}")))?;
        if status != 200 {
            let code = v
                .get("error")
                .and_then(Value::as_str)
                .and_then(ErrorCode::from_code)
                .unwrap_or(ErrorCode::InputUnavailable);
            return Err(ClientError::new(code, format!("{url}: status {status}")));
        }
        let cell = v.get("cell").cloned().ok_or_else(|| ClientError::unavailable(format!("{url}: no cell")))?;
        let mut versions = Vec::new();
        if let Some(Value::Array(vs)) = v.get("versions") {
            for x in vs {
                let s = |k: &str| x.get(k).and_then(Value::as_str).map(str::to_string);
                match (s("tx"), s("asof"), s("receipt")) {
                    (Some(tx), Some(asof), Some(receipt)) => versions.push(VersionEntry { tx, asof, receipt }),
                    _ => return Err(ClientError::unavailable(format!("{url}: a malformed version"))),
                }
            }
        }
        Ok(Versions { cell, versions, next: v.get("next").and_then(Value::as_str).map(str::to_string) })
    }

    /// A receipt fetched by the client, parsed.
    pub fn parse_receipt(bytes: &[u8]) -> Result<Receipt, ClientError> {
        Receipt::from_json(bytes).map_err(|e| ClientError::new(e.code(), e.to_string()))
    }
}

fn collect_signers(v: &Value, hosts: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if k == "computed_by" {
                    if let Some(s) = x.as_str() {
                        hosts.insert(s.split('#').next().unwrap_or("").to_string());
                    }
                } else {
                    collect_signers(x, hosts);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_signers(x, hosts)),
        _ => {}
    }
}

/// The host of a function name.
pub fn name_host(name: &str) -> &str {
    host_of(name)
}
