//! The mesh's configuration: [`Config`], from code or from the environment
//! ([`Config::from_env`]).
//!
//! Secrets never come from here directly: the signing key and the API keys are read from
//! files that must have mode 0600 ([`read_secret_file`]).

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use datumesh_core::json::{self, Value};
use datumesh_core::lexical;

use crate::apikeys::ApiKeys;

/// The default wall-clock budget of a request before it is answered `pending` (SPEC 5.5).
pub const DEFAULT_BUDGET: Duration = Duration::from_secs(1);
/// The default size of the blocking pool, which is the concurrency limit of evaluation and
/// store access (see the crate documentation for why).
pub const DEFAULT_POOL_SIZE: usize = 32;
/// The default bound on a `POST /cells` batch.
pub const DEFAULT_BATCH_MAX: usize = 1000;
/// The default number of members in one JSON page of `GET /slice`.
pub const DEFAULT_SLICE_PAGE: usize = 1000;
/// The bound on a JSON page of `GET /slice` (`limit=`).
pub const MAX_SLICE_PAGE: usize = 10_000;
/// The bound on the members of a slice served (SPEC 10.1: a manifest has at most 1,048,576).
pub const MAX_SLICE_MEMBERS: usize = 1_048_576;

/// Rate limits (token buckets), per minute. Refused requests are answered `rate_limited`
/// with `retry_after_seconds` and `Retry-After`; nothing is ever queued.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RateLimits {
    /// Whether limits apply at all (default true).
    pub enabled: bool,
    /// Requests per minute per client IP without an API key (default 60).
    pub per_ip: u32,
    /// The burst per IP: the bucket's capacity (default 120).
    pub ip_burst: u32,
    /// Requests per minute per API key, unless the key's own line says otherwise (default
    /// 600; the burst is the same number).
    pub per_key: u32,
    /// `POST /verify` per minute per IP, with or without a key (default 60; burst the same).
    pub verify_per_ip: u32,
    /// Cells per request token in a `POST /cells` batch (default 50: a batch of 1,000 cells
    /// counts as 20 requests).
    pub batch_cells_per_token: u32,
}

impl Default for RateLimits {
    fn default() -> Self {
        RateLimits {
            enabled: true,
            per_ip: 60,
            ip_burst: 120,
            per_key: 600,
            verify_per_ip: 60,
            batch_cells_per_token: 50,
        }
    }
}

/// Cache lifetimes. Content-addressed objects (descriptors, receipts, manifests, blobs) never
/// expire: their bytes are fixed by their hash.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ttls {
    /// A foreign host's key document (default 300 s). A receipt signed with a key id the
    /// cached document lacks refetches it at once (at most every `key_refetch`).
    pub key_doc: Duration,
    /// The shortest interval between two fetches of one host's key document (default 10 s).
    pub key_refetch: Duration,
    /// A foreign host's mesh document (default 300 s).
    pub mesh_doc: Duration,
    /// How long a foreign name's binding, confirmed with its mesh, stands for requests
    /// without an asof or with a later one (default 60 s).
    pub binding: Duration,
    /// How long a foreign cell's latest version, confirmed with its mesh, stands for requests
    /// without an asof or with a later one (default 60 s).
    pub current: Duration,
    /// `max-age` of answers to requests without an asof (default 10 s).
    pub latest_max_age: Duration,
    /// How long a completed job's result is kept after it is first served, and after it
    /// completes when it is never served, while the mesh runs (default and minimum 300 s, SPEC 5.5).
    pub job: Duration,
}

impl Default for Ttls {
    fn default() -> Self {
        Ttls {
            key_doc: Duration::from_secs(300),
            key_refetch: Duration::from_secs(10),
            mesh_doc: Duration::from_secs(300),
            binding: Duration::from_secs(60),
            current: Duration::from_secs(60),
            latest_max_age: Duration::from_secs(10),
            job: Duration::from_secs(300),
        }
    }
}

/// What a function's cells need (the licence and payment stubs of step 1.4).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Only these principals (API key owners) may receive the function's cells; anyone
    /// else is answered `unlicensed`.
    Principals(Vec<String>),
    /// The function's cells are sold: `payment_required` with this quote (a stub until
    /// prepaid balances, 3.3).
    Price {
        /// The amount (SPEC 3.1, greater than zero).
        amount: String,
        /// A currency code of `units.md`.
        currency: String,
    },
}

/// Per-function rules, by fully qualified function name. Functions without one are free.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    /// The rules.
    pub rules: BTreeMap<String, Rule>,
}

impl Policy {
    /// The rule of a function, if any.
    pub fn rule(&self, fn_name: &str) -> Option<&Rule> {
        self.rules.get(fn_name)
    }

    /// From JSON: `{"<function>": {"principals": ["..."]} | {"price": {"amount", "currency"}}}`.
    pub fn from_json(bytes: &[u8]) -> Result<Policy, ConfigError> {
        let v = json::parse(bytes).map_err(|e| ConfigError::new("policy", e.to_string()))?;
        let mut rules = BTreeMap::new();
        for (name, r) in v.as_object().ok_or_else(|| ConfigError::new("policy", "not an object"))? {
            lexical::check_function_name(name, "$").map_err(|e| ConfigError::new("policy", e.to_string()))?;
            let rule = if let Some(Value::Array(ps)) = r.get("principals") {
                Rule::Principals(
                    ps.iter()
                        .map(|p| p.as_str().map(str::to_string).ok_or_else(|| ConfigError::new("policy", "principal")))
                        .collect::<Result<_, _>>()?,
                )
            } else if let Some(p) = r.get("price") {
                let amount =
                    p.get("amount").and_then(Value::as_str).ok_or_else(|| ConfigError::new("policy", "amount"))?;
                let currency =
                    p.get("currency").and_then(Value::as_str).ok_or_else(|| ConfigError::new("policy", "currency"))?;
                lexical::check_number(amount, "$").map_err(|e| ConfigError::new("policy", e.to_string()))?;
                if amount.starts_with('-') || amount == "0" || !lexical::is_currency(currency) {
                    return Err(ConfigError::new("policy", format!("price of {name}")));
                }
                Rule::Price { amount: amount.to_string(), currency: currency.to_string() }
            } else {
                return Err(ConfigError::new("policy", format!("rule of {name}")));
            };
            rules.insert(name.clone(), rule);
        }
        Ok(Policy { rules })
    }
}

/// The mesh's configuration.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct Config {
    /// The DNS name the mesh signs as (`computed_by`) and serves on: `https://<host>/...`
    /// (e.g. `mesh-test.datumesh.com`), unless [`Config::base_url`] says otherwise.
    pub host: String,
    /// Where the mesh is served when not at `https://<host>`, e.g. `http://127.0.0.1:8787`
    /// for a mesh on this machine (default: none): the base of every URL in the mesh
    /// document (`bindings` and the endpoints, [`crate::Mesh::mesh_document`]). `https://`
    /// with any host; `http://` only when the mesh's `host` is unrooted (its last label is
    /// `invalid`, SPEC 2.5, e.g. `local.invalid`) and only with `127.0.0.1`, `[::1]` or
    /// `localhost`; an optional port; no path, query, fragment or trailing slash
    /// ([`check_base_url`]; [`crate::MeshBuilder::build`] refuses another with
    /// [`crate::BuildError::BaseUrl`]).
    ///
    /// An `http://` mesh document is for local use only: it is not valid under spec-0.4.3
    /// (9.6 wants `https://` URLs on the host), and core 0.3.6's `resolve_mesh` refuses it.
    /// spec-0.4.4 will allow exactly this: for an unrooted host, mesh-document URLs and
    /// `poll` may be `http://` on a loopback address only (`127.0.0.1`, `[::1]`,
    /// `localhost`); rooted hosts stay `https://`-only. datumesh-core 0.3.8 will accept it.
    /// The key document and `host` do not change, nor do the URLs in envelopes and headers
    /// (a `pending` envelope's `poll`, a page's `next`, the `canonical` link), which stay
    /// [`Config::origin`]: SPEC 5.5 makes `poll` an `https://` URL, and a client maps
    /// `https://<host>/` onto where it reaches the mesh (its peers).
    pub base_url: Option<String>,
    /// The hosts whose names this mesh owns (default: `host`). A name under any other host
    /// is resolved across meshes.
    pub namespaces: Vec<String>,
    /// The wall-clock budget of a request before it is answered `pending` (default 1 s).
    pub budget: Duration,
    /// The blocking pool's size: the concurrency limit (default 32).
    pub pool_size: usize,
    /// The most cells in one `POST /cells` (default 1,000).
    pub batch_max: usize,
    /// The default JSON page of `GET /slice` (default 1,000; at most 10,000 with `limit=`).
    pub slice_page: usize,
    /// The most members one slice read serves (default 1,048,576).
    pub slice_max: usize,
    /// The depth of `GET /explain` by default (default 4; at most 16 with `depth=`).
    pub explain_depth: usize,
    /// The most jobs held at once, running or done (default 10,000).
    pub max_jobs: usize,
    /// Rate limits.
    pub rate: RateLimits,
    /// Cache lifetimes.
    pub ttl: Ttls,
    /// Take the client IP from the last `X-Forwarded-For` entry (behind Caddy) instead of
    /// the peer address (default false).
    pub trust_proxy: bool,
    /// Resolve names under hosts this mesh does not own (default true).
    pub cross_mesh: bool,
    /// When set, the only other hosts whose names are resolved across meshes; names under
    /// any other host are not served (`not_covered`). Default: every host.
    pub cross_mesh_hosts: Option<Vec<String>>,
    /// Origins to use instead of `https://<host>` for some hosts, e.g. `mesh-b.example` ->
    /// `http://127.0.0.1:8081` (tests and private networks only; the default is none).
    pub peers: BTreeMap<String, String>,
    /// The time limit of one request to another mesh (default 10 s; SPEC 9.3's bound).
    pub peer_timeout: Duration,
    /// Short names of the call syntax: `@alias` -> fully qualified name.
    pub aliases: BTreeMap<String, String>,
    /// Licence and payment rules.
    pub policy: Policy,
    /// Descriptions of the functions `GET /find` searches (name -> description).
    pub catalog: BTreeMap<String, String>,
    /// API keys (`Authorization: Bearer dm_...`).
    pub api_keys: ApiKeys,
}

impl Config {
    /// The defaults for a mesh signing as `host`.
    pub fn new(host: impl Into<String>) -> Config {
        let host = host.into();
        Config {
            namespaces: vec![host.clone()],
            host,
            base_url: None,
            budget: DEFAULT_BUDGET,
            pool_size: DEFAULT_POOL_SIZE,
            batch_max: DEFAULT_BATCH_MAX,
            slice_page: DEFAULT_SLICE_PAGE,
            slice_max: MAX_SLICE_MEMBERS,
            explain_depth: 4,
            max_jobs: 10_000,
            rate: RateLimits::default(),
            ttl: Ttls::default(),
            trust_proxy: false,
            cross_mesh: true,
            cross_mesh_hosts: None,
            peers: BTreeMap::new(),
            peer_timeout: Duration::from_secs(10),
            aliases: BTreeMap::new(),
            policy: Policy::default(),
            catalog: BTreeMap::new(),
            api_keys: ApiKeys::default(),
        }
    }

    /// The configuration with this base URL ([`Config::base_url`]); it is checked when the
    /// mesh is built.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Config {
        self.base_url = Some(url.into());
        self
    }

    /// The configuration from the environment (`DATUMESH_*`; the README lists them). The
    /// files named (`DATUMESH_API_KEYS_FILE`, and the others) are read here; the API keys
    /// file must have mode 0600.
    pub fn from_env() -> Result<Config, ConfigError> {
        Config::from_vars(|k| std::env::var(k).ok())
    }

    /// The configuration from variables `get` answers, as [`Config::from_env`] reads them.
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Config, ConfigError> {
        let host = get("DATUMESH_HOST").ok_or_else(|| ConfigError::new("DATUMESH_HOST", "not set"))?;
        let mut c = Config::new(host);
        if let Some(v) = get("DATUMESH_BASE_URL").filter(|v| !v.is_empty()) {
            c.base_url = Some(v);
        }
        if let Some(v) = get("DATUMESH_NAMESPACES") {
            c.namespaces = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        }
        let ms = |k: &str, d: Duration| -> Result<Duration, ConfigError> {
            match get(k) {
                Some(v) => v.parse::<u64>().map(Duration::from_millis).map_err(|e| ConfigError::new(k, e.to_string())),
                None => Ok(d),
            }
        };
        let secs = |k: &str, d: Duration| -> Result<Duration, ConfigError> {
            match get(k) {
                Some(v) => v.parse::<u64>().map(Duration::from_secs).map_err(|e| ConfigError::new(k, e.to_string())),
                None => Ok(d),
            }
        };
        let num = |k: &str, d: usize| -> Result<usize, ConfigError> {
            match get(k) {
                Some(v) => v.parse::<usize>().map_err(|e| ConfigError::new(k, e.to_string())),
                None => Ok(d),
            }
        };
        let flag = |k: &str, d: bool| -> Result<bool, ConfigError> {
            match get(k).as_deref() {
                None => Ok(d),
                Some("1" | "true" | "yes") => Ok(true),
                Some("0" | "false" | "no") => Ok(false),
                Some(v) => Err(ConfigError::new(k, format!("not a boolean: {v}"))),
            }
        };
        c.budget = ms("DATUMESH_BUDGET_MS", c.budget)?;
        c.pool_size = num("DATUMESH_POOL_SIZE", c.pool_size)?;
        c.batch_max = num("DATUMESH_BATCH_MAX", c.batch_max)?;
        c.slice_page = num("DATUMESH_SLICE_PAGE", c.slice_page)?;
        c.slice_max = num("DATUMESH_SLICE_MAX", c.slice_max)?;
        c.explain_depth = num("DATUMESH_EXPLAIN_DEPTH", c.explain_depth)?;
        c.max_jobs = num("DATUMESH_MAX_JOBS", c.max_jobs)?;
        c.trust_proxy = flag("DATUMESH_TRUST_PROXY", c.trust_proxy)?;
        c.cross_mesh = flag("DATUMESH_CROSS_MESH", c.cross_mesh)?;
        c.rate.enabled = flag("DATUMESH_RATE_LIMITS", c.rate.enabled)?;
        c.rate.per_ip = num("DATUMESH_RATE_PER_IP", c.rate.per_ip as usize)? as u32;
        c.rate.ip_burst = num("DATUMESH_RATE_IP_BURST", c.rate.ip_burst as usize)? as u32;
        c.rate.per_key = num("DATUMESH_RATE_PER_KEY", c.rate.per_key as usize)? as u32;
        c.rate.verify_per_ip = num("DATUMESH_RATE_VERIFY_PER_IP", c.rate.verify_per_ip as usize)? as u32;
        c.ttl.key_doc = secs("DATUMESH_TTL_KEY_DOC", c.ttl.key_doc)?;
        c.ttl.mesh_doc = secs("DATUMESH_TTL_MESH_DOC", c.ttl.mesh_doc)?;
        c.ttl.binding = secs("DATUMESH_TTL_BINDING", c.ttl.binding)?;
        c.ttl.current = secs("DATUMESH_TTL_CURRENT", c.ttl.current)?;
        c.ttl.latest_max_age = secs("DATUMESH_LATEST_MAX_AGE", c.ttl.latest_max_age)?;
        c.ttl.job = secs("DATUMESH_JOB_TTL", c.ttl.job)?;
        c.peer_timeout = ms("DATUMESH_PEER_TIMEOUT_MS", c.peer_timeout)?;
        if let Some(v) = get("DATUMESH_CROSS_MESH_HOSTS") {
            c.cross_mesh_hosts = Some(v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
        }
        if let Some(v) = get("DATUMESH_PEERS") {
            for pair in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                let (h, o) =
                    pair.split_once('=').ok_or_else(|| ConfigError::new("DATUMESH_PEERS", "host=origin pairs"))?;
                c.peers.insert(h.trim().to_string(), o.trim().trim_end_matches('/').to_string());
            }
        }
        if let Some(p) = get("DATUMESH_ALIASES_FILE") {
            c.aliases = string_map(&read_file(&p)?, "DATUMESH_ALIASES_FILE")?;
        }
        if let Some(p) = get("DATUMESH_CATALOG_FILE") {
            c.catalog = string_map(&read_file(&p)?, "DATUMESH_CATALOG_FILE")?;
        }
        if let Some(p) = get("DATUMESH_POLICY_FILE") {
            c.policy = Policy::from_json(&read_file(&p)?)?;
        }
        if let Some(p) = get("DATUMESH_API_KEYS_FILE") {
            c.api_keys = ApiKeys::parse(&read_secret_file(Path::new(&p))?)?;
        }
        c.check()?;
        Ok(c)
    }

    /// Checks the configuration: names are DNS names and function names, the base URL is
    /// one ([`check_base_url`]), numbers in range.
    pub fn check(&self) -> Result<(), ConfigError> {
        lexical::check_dns_name(&self.host, "$").map_err(|e| ConfigError::new("host", e.to_string()))?;
        if let Some(u) = &self.base_url {
            check_base_url(&self.host, u).map_err(|e| ConfigError::new("base URL", format!("{u}: {e}")))?;
        }
        for h in &self.namespaces {
            lexical::check_dns_name(h, "$").map_err(|e| ConfigError::new("namespaces", e.to_string()))?;
        }
        for (h, o) in &self.peers {
            lexical::check_dns_name(h, "$").map_err(|e| ConfigError::new("peers", e.to_string()))?;
            if !(o.starts_with("http://") || o.starts_with("https://")) {
                return Err(ConfigError::new("peers", format!("{o}: not an http(s) origin")));
            }
        }
        for (a, n) in &self.aliases {
            lexical::check_function_name(n, "$").map_err(|e| ConfigError::new("aliases", format!("{a}: {e}")))?;
        }
        if self.pool_size == 0 || self.batch_max == 0 || self.slice_page == 0 || self.slice_max == 0 {
            return Err(ConfigError::new("sizes", "the pool, batch and slice bounds must be positive"));
        }
        if self.ttl.job < Duration::from_secs(300) {
            return Err(ConfigError::new("job TTL", "at least 300 s (SPEC 5.5)"));
        }
        if self.budget.is_zero() {
            return Err(ConfigError::new("budget", "must be positive"));
        }
        Ok(())
    }

    /// Whether this mesh owns the names under `host`.
    pub fn owns(&self, host: &str) -> bool {
        self.namespaces.iter().any(|h| h == host)
    }

    /// `https://<host>`: where this mesh is served, the base of the URLs in its envelopes.
    pub fn origin(&self) -> String {
        format!("https://{}", self.host)
    }

    /// The base of the URLs in the mesh document: the base URL when set, else
    /// [`Config::origin`].
    pub fn mesh_document_base(&self) -> String {
        self.base_url.clone().unwrap_or_else(|| self.origin())
    }
}

/// Why a base URL ([`Config::base_url`]) is refused.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseUrlError {
    /// Neither `https://` nor `http://` (lower case).
    Scheme,
    /// `http://` for a mesh whose host is rooted (not under `.invalid`, SPEC 2.5): rooted
    /// hosts are served over `https://` only, whatever the base URL's host.
    HttpRootedHost,
    /// `http://` (for an unrooted host) with a host other than `127.0.0.1`, `[::1]` and
    /// `localhost`.
    HttpNotLoopback,
    /// No host, userinfo, or a host that is neither a DNS name (lower case), an IPv4 address
    /// nor an IPv6 address in brackets.
    Host,
    /// A port that is not a decimal number from 1 to 65535 (`:` with nothing after it too).
    Port,
    /// A path after the host and port.
    Path,
    /// A slash alone after the host and port.
    TrailingSlash,
    /// A query.
    Query,
    /// A fragment.
    Fragment,
}

impl fmt::Display for BaseUrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BaseUrlError::Scheme => "not an https:// or http:// URL",
            BaseUrlError::HttpRootedHost => "http:// is for an unrooted host (under .invalid) only",
            BaseUrlError::HttpNotLoopback => "http:// is for 127.0.0.1, [::1] and localhost only",
            BaseUrlError::Host => "not a host name, IPv4 address or bracketed IPv6 address",
            BaseUrlError::Port => "not a port from 1 to 65535",
            BaseUrlError::Path => "a path (the base URL is a scheme, a host and a port)",
            BaseUrlError::TrailingSlash => "a trailing slash",
            BaseUrlError::Query => "a query",
            BaseUrlError::Fragment => "a fragment",
        })
    }
}

impl std::error::Error for BaseUrlError {}

/// Checks a base URL ([`Config::base_url`]) for a mesh signing as `host`: `https://` with any
/// host, or, when `host` is unrooted ([`is_unrooted`]), `http://` with `127.0.0.1`, `[::1]`
/// or `localhost`; then an optional `:<port>` (1 to 65535) and nothing else: no path, query,
/// fragment, trailing slash or userinfo. The form is checked first, then the scheme's rule:
/// `http://` for a rooted host is [`BaseUrlError::HttpRootedHost`] before anything about the
/// URL's host.
pub fn check_base_url(host: &str, url: &str) -> Result<(), BaseUrlError> {
    let mesh_host = host;
    let (http, rest) = if let Some(r) = url.strip_prefix("https://") {
        (false, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (true, r)
    } else {
        return Err(BaseUrlError::Scheme);
    };
    if rest.contains('#') {
        return Err(BaseUrlError::Fragment);
    }
    if rest.contains('?') {
        return Err(BaseUrlError::Query);
    }
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    match path {
        "" => {}
        "/" => return Err(BaseUrlError::TrailingSlash),
        _ => return Err(BaseUrlError::Path),
    }
    if authority.contains('@') {
        return Err(BaseUrlError::Host);
    }
    let (host, port) = if let Some(r) = authority.strip_prefix('[') {
        let (inside, after) = r.split_once(']').ok_or(BaseUrlError::Host)?;
        inside.parse::<std::net::Ipv6Addr>().map_err(|_| BaseUrlError::Host)?;
        let port = match after {
            "" => None,
            p => Some(p.strip_prefix(':').ok_or(BaseUrlError::Host)?),
        };
        (&authority[..inside.len() + 2], port)
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        };
        if host.parse::<std::net::Ipv4Addr>().is_err() && !is_dns_name(host) {
            return Err(BaseUrlError::Host);
        }
        (host, port)
    };
    if let Some(p) = port {
        let ok = !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit());
        if !ok || !(1..=65535).contains(&p.parse::<u32>().unwrap_or(0)) {
            return Err(BaseUrlError::Port);
        }
    }
    if http && !is_unrooted(mesh_host) {
        return Err(BaseUrlError::HttpRootedHost);
    }
    if http && !matches!(host, "127.0.0.1" | "[::1]" | "localhost") {
        return Err(BaseUrlError::HttpNotLoopback);
    }
    Ok(())
}

/// Whether a host is unrooted (SPEC 2.5, spec-0.4.3): its last label is `invalid`
/// (`local.invalid`, `agent-7.local.invalid`). Every other host is rooted.
pub fn is_unrooted(host: &str) -> bool {
    host.rsplit('.').next() == Some("invalid")
}

/// A DNS name in lower case: labels of 1 to 63 of `a-z`, `0-9` and `-`, not starting or
/// ending with `-`, at most 253 characters.
fn is_dns_name(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

fn string_map(bytes: &[u8], what: &str) -> Result<BTreeMap<String, String>, ConfigError> {
    let v = json::parse(bytes).map_err(|e| ConfigError::new(what, e.to_string()))?;
    let mut out = BTreeMap::new();
    for (k, x) in v.as_object().ok_or_else(|| ConfigError::new(what, "not an object"))? {
        let s = x.as_str().ok_or_else(|| ConfigError::new(what, format!("{k}: not a string")))?;
        out.insert(k.clone(), s.to_string());
    }
    Ok(out)
}

fn read_file(path: &str) -> Result<Vec<u8>, ConfigError> {
    std::fs::read(path).map_err(|e| ConfigError::new(path, e.to_string()))
}

/// Reads a file holding a secret, refusing it unless only its owner may read or write it
/// (mode 0600 or stricter, on Unix).
pub fn read_secret_file(path: &Path) -> Result<Vec<u8>, ConfigError> {
    let what = path.display().to_string();
    let meta = std::fs::metadata(path).map_err(|e| ConfigError::new(&what, e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(ConfigError::new(&what, format!("mode {mode:o}: a secret file must be 0600")));
        }
    }
    #[cfg(not(unix))]
    let _ = meta;
    std::fs::read(path).map_err(|e| ConfigError::new(&what, e.to_string()))
}

/// A configuration error: what and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    what: String,
    detail: String,
}

impl ConfigError {
    pub(crate) fn new(what: &str, detail: impl Into<String>) -> ConfigError {
        ConfigError { what: what.to_string(), detail: detail.into() }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "configuration: {}: {}", self.what, self.detail)
    }
}

impl std::error::Error for ConfigError {}
