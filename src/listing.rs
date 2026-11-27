//! Listing another mesh (CONTRACT-http-listing, http 0.6.0): the functions of a mesh this mesh does not host, as
//! descriptions only, with no cells stored.
//!
//! **Stub of the test commit.** The types and signatures are the API the tests pin; every function body is `todo!()`.
//! The implementer writes the bodies (see `BRIEF-listing-060-implement.md`), and changes neither the signatures nor
//! the tests.
//!
//! A run for one listed host is two steps, so that the store side is tested without a network:
//!
//! * [`fetch`]: read-only HTTP against the host's origin (L2): the key document (SPEC 9: host, keys, validity), the mesh
//!   document (agreeing on `host` and `proto`), the host's own catalogue cells `<host>/catalog/subject(...)` (each verified
//!   against the key document, its keys and `source(k)` names taken, the cell dropped), and for each name under `<host>/`,
//!   `describe` and `/descriptor/<hash>` (the bytes must hash to the function hash), and its label and description from
//!   `/fn/<name>?format=json` (describe carries neither). Never a cell, slice or receipt of a
//!   listed function. Within [`Limits`]; past one, what is verified so far is kept and the cut recorded.
//! * [`apply`]: writes the result into the store (L3, L8): the present state only, nothing when nothing changed.
//!
//! [`run_host`] is the two together; the mesh's listing thread calls it for each host of
//! [`Config::listed_meshes`](crate::Config::listed_meshes) every [`Config::listing_interval`](crate::Config::listing_interval).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use datumesh_core::json::{self, Value};
use datumesh_core::{KeyDocument, Keys};
use datumesh_store::{ListedHost, Listing, ListingWrite, Store, SubjectKeys};

use crate::envelope::{Cache, Reply};

/// The default interval between two runs of one host (`DATUMESH_LISTING_INTERVAL_SECS`, six hours).
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(21_600);

/// The most hosts `DATUMESH_LISTED_MESHES` may name.
pub const MAX_HOSTS: usize = 64;

/// How long a host may be unreachable before its listings drop out (L8), counted from the first failed run after the last
/// success. At exactly seven days it drops out; at six days 23 hours it does not.
pub const UNREACHABLE_DROP_AFTER: Duration = Duration::from_secs(7 * 86_400);

/// The limits of one run of one host (L2.5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most names listed (default 2,000). Names are taken in byte order of the name.
    pub names: usize,
    /// The most bytes of response bodies read, every request of the run counted (default 32 MB = 32 << 20).
    pub bytes: u64,
    /// The longest a run takes, from its first request (default 120 s). A request in flight at the limit is abandoned.
    pub time: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { names: 2_000, bytes: 32 << 20, time: Duration::from_secs(120) }
    }
}

/// Which limit cut a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// [`Limits::names`].
    Names,
    /// [`Limits::bytes`].
    Bytes,
    /// [`Limits::time`].
    Time,
}

/// One verified name of a listed host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedName {
    /// The full name, under `<host>/`.
    pub name: String,
    /// Its function hash, as `describe` gives it.
    pub fn_hash: String,
    /// The subjects the host's catalogue puts it in, ascending and unique.
    pub subjects: Vec<SubjectKeys>,
    /// The descriptor's bytes from `/descriptor/<fn_hash>`.
    pub descriptor: Vec<u8>,
    /// Its label in the host's catalogue (from the host's function page as JSON, `/fn/<name>?format=json`; may be empty).
    pub label: String,
    /// Its description (from the same page).
    pub description: String,
}

/// What one fetch of a host gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetch {
    /// The key document and the mesh document passed, and the catalogue was read (perhaps cut).
    Fetched {
        /// The id of the key of the key document that verified the host's catalogue cells.
        key_id: String,
        /// The names that verified, by name.
        names: Vec<FetchedName>,
        /// The names reached and refused (a catalogue cell, describe or descriptor that does not verify), by name.
        refused: Vec<String>,
        /// The limit that cut the run, if one did.
        cut: Option<Cut>,
    },
    /// No answer: a connection error, a timeout or a 5xx, of any request of the run (L8: not a withdrawal).
    Unreachable {
        /// What failed.
        detail: String,
    },
    /// The key document answered and failed the SPEC 9 checks (another host, no key valid at the run's time, not a key
    /// document, a 4xx).
    KeyDocFailed {
        /// Why.
        detail: String,
    },
    /// The mesh document answered and failed: it disagrees with the key document on `host` or `proto`, or is not one.
    MeshDocFailed {
        /// Why.
        detail: String,
    },
}

/// The outcome of a run, as `/metrics` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Fetched, no limit reached.
    Ok,
    /// [`Fetch::KeyDocFailed`].
    KeyDocFailed,
    /// [`Fetch::MeshDocFailed`].
    MeshDocFailed,
    /// Fetched, cut by a limit.
    Cut,
    /// [`Fetch::Unreachable`].
    Unreachable,
}

impl Outcome {
    /// `ok`, `key_document_failed`, `mesh_document_failed`, `cut` or `unreachable`: the `outcome` label of
    /// `datumesh_listing_outcome`.
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::KeyDocFailed => "key_document_failed",
            Outcome::MeshDocFailed => "mesh_document_failed",
            Outcome::Cut => "cut",
            Outcome::Unreachable => "unreachable",
        }
    }
}

/// What [`apply`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Applied {
    /// The run's outcome.
    pub outcome: Outcome,
    /// The cut, when the outcome is [`Outcome::Cut`].
    pub cut: Option<Cut>,
    /// The names newly listed, by name.
    pub added: Vec<String>,
    /// The names whose listing changed (hash, descriptor, subjects, label or description), by name.
    pub changed: Vec<String>,
    /// The names that dropped out, by name.
    pub dropped: Vec<String>,
    /// The names of the host listed after the run.
    pub listed: usize,
    /// The rows the run wrote or deleted: listings put (inserted or changed), listings deleted, and the host's row when
    /// it was written. 0 for a run that changed nothing.
    pub writes: u64,
}

impl Applied {
    /// Whether the run added, changed or dropped a listing (a catalogue job run after it has something to write).
    pub fn changed_any(&self) -> bool {
        !(self.added.is_empty() && self.changed.is_empty() && self.dropped.is_empty())
    }
}

/// A failure of [`apply`] (the store's), which leaves the store as it was before the failing write.
#[derive(Debug)]
pub struct ListingError {
    detail: String,
}

impl std::fmt::Display for ListingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "listing: {}", self.detail)
    }
}

impl std::error::Error for ListingError {}

/// Where `host` is reached: its origin in [`Config::peers`](crate::Config::peers) when it has one, else
/// `https://<host>`.
pub fn origin_of(config: &crate::Config, host: &str) -> String {
    match config.peers.get(host) {
        Some(origin) => origin.clone(),
        None => format!("https://{host}"),
    }
}

/// Why a request of a run stopped it.
enum Stop {
    /// No answer, or a 5xx (L8).
    Unreachable(String),
    /// The byte limit was passed.
    Bytes,
}

/// One run's requests: each one's timeout is the time left, its body counted against the bytes left.
struct Run {
    agent: ureq::Agent,
    started: Instant,
    limits: Limits,
    bytes: u64,
}

impl Run {
    fn new(limits: &Limits) -> Run {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent(concat!("datumesh-http/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Run { agent, started: Instant::now(), limits: limits.clone(), bytes: 0 }
    }

    /// The time left of the run, or `None` when it is spent.
    fn left(&self) -> Option<Duration> {
        self.limits.time.checked_sub(self.started.elapsed()).filter(|d| !d.is_zero())
    }

    /// `GET url`: the status and the body, or why it stopped the run.
    fn get(&mut self, url: &str) -> Result<(u16, Vec<u8>), Stop> {
        let Some(left) = self.left() else { return Err(Stop::Unreachable("the run's time is spent".into())) };
        let resp = self
            .agent
            .get(url)
            .config()
            .timeout_global(Some(left))
            .build()
            .call()
            .map_err(|e| Stop::Unreachable(format!("{url}: {e}")))?;
        let status = resp.status().as_u16();
        if status >= 500 {
            return Err(Stop::Unreachable(format!("{url}: status {status}")));
        }
        let room = self.limits.bytes.saturating_sub(self.bytes);
        let body = resp.into_body().with_config().limit(room.saturating_add(1)).read_to_vec().map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(_) => Stop::Bytes,
            e => Stop::Unreachable(format!("{url}: {e}")),
        })?;
        self.bytes += body.len() as u64;
        if self.bytes > self.limits.bytes {
            return Err(Stop::Bytes);
        }
        Ok((status, body))
    }
}

/// The key of `doc` valid at `now` (`valid_from <= now < valid_to`), if any.
fn key_valid_at(doc: &KeyDocument, now: &str) -> Option<String> {
    doc.keys
        .keys
        .iter()
        .find(|k| k.valid_from.as_str() <= now && k.valid_to.as_deref().is_none_or(|t| now < t))
        .map(|k| k.id.clone())
}

/// The function name's subjects and source names, from the host's signed catalogue cells (L2.3): each member's receipt
/// is verified against the host's key document; a member that does not verify is skipped. Pages are followed.
fn catalogue(
    run: &mut Run,
    host: &str,
    origin: &str,
    doc: &KeyDocument,
) -> Result<BTreeMap<String, BTreeSet<SubjectKeys>>, Stop> {
    let mut keys = Keys::new();
    keys.insert_document(doc.clone());
    let mut out: BTreeMap<String, BTreeSet<SubjectKeys>> = BTreeMap::new();
    let mut url = format!("{origin}/slice/{host}/catalog/subject/source?domain=*&measure=*&rank=*&region=*&thing=*");
    loop {
        let (status, body) = run.get(&url)?;
        if status == 404 {
            break;
        }
        if status != 200 {
            return Err(Stop::Unreachable(format!("{url}: status {status}")));
        }
        let page = json::parse(&body).map_err(|_| Stop::Unreachable(format!("{url}: not JSON")))?;
        if let Some(Value::Array(members)) = page.get("members") {
            for m in members {
                let Some(receipt) = m.get("receipt") else { continue };
                let r = datumesh_core::verify(&receipt.to_jcs(), &keys, None, None);
                if r.error().is_some() {
                    continue;
                }
                let Some(cell) = receipt.get("cell") else { continue };
                let arg = |k: &str| -> Option<String> {
                    cell.get("args")?.get(k)?.get("value")?.as_str().map(str::to_string)
                };
                let (Some(d), Some(t), Some(me), Some(rg)) =
                    (arg("domain"), arg("thing"), arg("measure"), arg("region"))
                else {
                    continue;
                };
                let value = receipt.get("value");
                let name = value.and_then(|v| v.as_str().or_else(|| v.get("value").and_then(Value::as_str)));
                let Some(name) = name else { continue };
                if name.starts_with(&format!("{host}/")) {
                    out.entry(name.to_string()).or_default().insert(SubjectKeys::new(&d, &t, &me, &rg));
                }
            }
        }
        match page.get("next").and_then(Value::as_str) {
            Some(next) => {
                url = match next.strip_prefix("https://") {
                    Some(r) => format!("{origin}{}", &r[r.find('/').unwrap_or(r.len())..]),
                    None if next.starts_with('/') => format!("{origin}{next}"),
                    None => next.to_string(),
                };
            }
            None => break,
        }
    }
    Ok(out)
}

/// Fetches `host` at `origin` (L2), read-only, within `limits`; `now` (an instant) is the time the key document's keys
/// must be valid at.
pub fn fetch(host: &str, origin: &str, limits: &Limits, now: &str) -> Fetch {
    let mut run = Run::new(limits);
    let unreachable = |s: Stop| match s {
        Stop::Unreachable(detail) => Fetch::Unreachable { detail },
        Stop::Bytes => Fetch::Unreachable { detail: "the byte limit, before the key document".into() },
    };
    // 1. the key document
    let (status, body) = match run.get(&format!("{origin}/.well-known/datumesh.json")) {
        Ok(x) => x,
        Err(s) => return unreachable(s),
    };
    let doc = match datumesh_core::key_document_from_response(host, status, Some(&body)) {
        Ok(d) if status == 200 => d,
        Ok(_) => return Fetch::KeyDocFailed { detail: format!("status {status}") },
        Err(e) => return Fetch::KeyDocFailed { detail: e.to_string() },
    };
    let Some(key_id) = key_valid_at(&doc, now) else {
        return Fetch::KeyDocFailed { detail: format!("no key of {host} is valid at {now}") };
    };
    let proto = json::parse(&body).ok().and_then(|v| v.get("proto").and_then(Value::as_str).map(str::to_string));
    // 2. the mesh document, agreeing on host and proto
    let (status, mbody) = match run.get(&format!("{origin}/.well-known/datumesh-mesh.json")) {
        Ok(x) => x,
        Err(s) => return unreachable(s),
    };
    if status != 200 {
        return Fetch::MeshDocFailed { detail: format!("status {status}") };
    }
    let Ok(m) = json::parse(&mbody) else { return Fetch::MeshDocFailed { detail: "not JSON".into() } };
    if m.get("host").and_then(Value::as_str) != Some(host) || m.get("proto").and_then(Value::as_str) != proto.as_deref()
    {
        return Fetch::MeshDocFailed { detail: "disagrees with the key document on host or proto".into() };
    }
    // 3. the host's own catalogue, verified and not stored
    let subjects = match catalogue(&mut run, host, origin, &doc) {
        Ok(x) => x,
        Err(s) => return unreachable(s),
    };
    let mut cut = None;
    let mut all: Vec<(String, Vec<SubjectKeys>)> =
        subjects.into_iter().map(|(n, s)| (n, s.into_iter().collect())).collect();
    if all.len() > limits.names {
        all.truncate(limits.names);
        cut = Some(Cut::Names);
    }
    // 4. each name: describe, descriptor, and its label and description
    let mut names = Vec::new();
    let mut refused = Vec::new();
    for (name, subjects) in all {
        if run.left().is_none() {
            cut = Some(Cut::Time);
            break;
        }
        let base = format!("{origin}/");
        let step = (|| -> Result<Option<FetchedName>, Stop> {
            let (status, d) = run.get(&format!("{base}describe/{name}"))?;
            let fn_hash = match (status, json::parse(&d)) {
                (200, Ok(v)) => v.get("fn_hash").and_then(Value::as_str).map(str::to_string),
                _ => None,
            };
            let Some(fn_hash) = fn_hash else { return Ok(None) };
            let (status, bytes) = run.get(&format!("{base}descriptor/{fn_hash}"))?;
            if status != 200 || datumesh_core::function_hash(&bytes).ok().as_deref() != Some(fn_hash.as_str()) {
                return Ok(None);
            }
            let (mut label, mut description) = (String::new(), String::new());
            let (status, f) = run.get(&format!("{base}fn/{name}?format=json"))?;
            if status == 200 {
                if let Ok(v) = json::parse(&f) {
                    label = v.get("label").and_then(Value::as_str).unwrap_or_default().to_string();
                    description = v.get("description").and_then(Value::as_str).unwrap_or_default().to_string();
                }
            }
            Ok(Some(FetchedName {
                name: name.clone(),
                fn_hash,
                subjects: subjects.clone(),
                descriptor: bytes,
                label,
                description,
            }))
        })();
        match step {
            Ok(Some(n)) => names.push(n),
            Ok(None) => refused.push(name),
            Err(Stop::Bytes) => {
                cut = Some(Cut::Bytes);
                break;
            }
            Err(Stop::Unreachable(detail)) => {
                if run.left().is_none() {
                    cut = Some(Cut::Time);
                    break;
                }
                return Fetch::Unreachable { detail };
            }
        }
    }
    Fetch::Fetched { key_id, names, refused, cut }
}

/// Applies a fetch of `host` at `now` (an instant) to the store (L3, L8):
///
/// * [`Fetch::Fetched`]: each name of `names` whose descriptor hashes to its `fn_hash` is put (a name whose descriptor
///   does not is dropped like a refused one); without a cut, every listing of the host not among the verified names drops
///   out; with a cut, only the `refused` ones do (the names past the cut were not checked). The host's row: `last_seen` the
///   day of `now`, `unreachable_since` none, written only when that changes it.
/// * [`Fetch::KeyDocFailed`], [`Fetch::MeshDocFailed`]: every listing of the host drops out at once; the host's row is
///   left as it was.
/// * [`Fetch::Unreachable`]: the listings stay; the host's `unreachable_since` is written once, at the first failed run;
///   when `now` is [`UNREACHABLE_DROP_AFTER`] or more after it, every listing of the host drops out.
///
/// A run that finds nothing changed writes nothing.
pub fn apply<S: Store + ?Sized>(store: &S, host: &str, fetch: &Fetch, now: &str) -> Result<Applied, ListingError> {
    let se = |e: &dyn std::fmt::Display| ListingError { detail: e.to_string() };
    let mut applied = Applied {
        outcome: Outcome::Ok,
        cut: None,
        added: Vec::new(),
        changed: Vec::new(),
        dropped: Vec::new(),
        listed: 0,
        writes: 0,
    };
    let existing: Vec<String> = store.listings_of(host).map_err(|e| se(&e))?.into_iter().map(|l| l.name).collect();
    let mut drop_all = false;
    let mut drop_set: BTreeSet<String> = BTreeSet::new();
    match fetch {
        Fetch::Fetched { key_id, names, refused, cut } => {
            let mut verified = BTreeSet::new();
            let mut refuse: BTreeSet<String> = refused.iter().cloned().collect();
            for n in names {
                let under = n.name.starts_with(&format!("{host}/"));
                let ok =
                    under && datumesh_core::function_hash(&n.descriptor).ok().as_deref() == Some(n.fn_hash.as_str());
                if !ok {
                    refuse.insert(n.name.clone());
                    continue;
                }
                let mut subjects = n.subjects.clone();
                subjects.sort();
                subjects.dedup();
                let l = Listing {
                    host: host.to_string(),
                    name: n.name.clone(),
                    fn_hash: n.fn_hash.clone(),
                    subjects,
                    descriptor: n.descriptor.clone(),
                    key_id: key_id.clone(),
                    label: n.label.clone(),
                    description: n.description.clone(),
                    fetched_at: now.to_string(),
                    listed_at: now.to_string(),
                };
                match store.put_listing(&l).map_err(|e| se(&e))? {
                    ListingWrite::Unchanged => {}
                    ListingWrite::Inserted => {
                        applied.added.push(n.name.clone());
                        applied.writes += 1;
                    }
                    ListingWrite::Changed => {
                        applied.changed.push(n.name.clone());
                        applied.writes += 1;
                    }
                }
                verified.insert(n.name.clone());
            }
            if cut.is_none() {
                drop_set.extend(existing.iter().filter(|e| !verified.contains(*e)).cloned());
            } else {
                drop_set.extend(existing.iter().filter(|e| refuse.contains(*e)).cloned());
                applied.cut = *cut;
                applied.outcome = Outcome::Cut;
            }
            let day = &now[..10];
            let row = ListedHost { host: host.to_string(), last_seen: Some(day.to_string()), unreachable_since: None };
            if store.put_listed_host(&row).map_err(|e| se(&e))? {
                applied.writes += 1;
            }
        }
        Fetch::KeyDocFailed { .. } => {
            applied.outcome = Outcome::KeyDocFailed;
            drop_all = true;
        }
        Fetch::MeshDocFailed { .. } => {
            applied.outcome = Outcome::MeshDocFailed;
            drop_all = true;
        }
        Fetch::Unreachable { .. } => {
            applied.outcome = Outcome::Unreachable;
            let row = store.listed_host(host).map_err(|e| se(&e))?;
            match row {
                None => {
                    let r = ListedHost {
                        host: host.to_string(),
                        last_seen: None,
                        unreachable_since: Some(now.to_string()),
                    };
                    store.put_listed_host(&r).map_err(|e| se(&e))?;
                    applied.writes += 1;
                }
                Some(r) => match r.unreachable_since.as_deref() {
                    None => {
                        let r2 = ListedHost { unreachable_since: Some(now.to_string()), ..r };
                        store.put_listed_host(&r2).map_err(|e| se(&e))?;
                        applied.writes += 1;
                    }
                    Some(since) => {
                        let due = crate::keynames::add_seconds(since, UNREACHABLE_DROP_AFTER.as_secs());
                        if due.is_some_and(|d| d.as_str() <= now) {
                            drop_all = true;
                        }
                    }
                },
            }
        }
    }
    if drop_all {
        drop_set.extend(existing.iter().cloned());
    }
    for name in &drop_set {
        if store.delete_listing(name).map_err(|e| se(&e))? {
            applied.dropped.push(name.clone());
            applied.writes += 1;
        }
    }
    applied.listed = store.listings_of(host).map_err(|e| se(&e))?.len();
    Ok(applied)
}

/// [`fetch`] then [`apply`].
pub fn run_host<S: Store + ?Sized>(
    store: &S,
    host: &str,
    origin: &str,
    limits: &Limits,
    now: &str,
) -> Result<Applied, ListingError> {
    let f = fetch(host, origin, limits, now);
    apply(store, host, &f, now)
}

// ---------------------------------------------------------------------------------------------------------------------
// What this mesh says about a listed name (L6): refused for every read, described and shown as a page

/// The listing of `name`, when it is a listed name here.
pub(crate) fn listed(store: &dyn Store, name: &str) -> Option<Listing> {
    store.listing(name).ok().flatten()
}

/// The listed name a read names: the first prefix of `text` (a path rest, `name`, `name(args)` or `name?query`, a series
/// page below a name) that is a listed name.
pub(crate) fn listed_in(store: &dyn Store, text: &str) -> Option<Listing> {
    let mut ends: Vec<usize> =
        text.char_indices().filter(|(_, c)| matches!(c, '/' | '(' | '?' | ' ')).map(|(i, _)| i).collect();
    ends.push(text.len());
    ends.into_iter().filter(|&e| text[..e].contains('/')).find_map(|e| listed(store, &text[..e]))
}

/// The listed name a formula (the body of `POST /x/calc`) reads: one of its words is a listed name.
pub(crate) fn listed_in_formula(store: &dyn Store, body: &[u8]) -> Option<Listing> {
    let formula = json::parse(body).ok()?.get("formula").and_then(Value::as_str)?.to_string();
    formula
        .split(|c: char| !(c.is_alphanumeric() || "._/-".contains(c)))
        .filter(|w| w.contains('/'))
        .find_map(|w| listed(store, w))
}

/// The host's own origin as a listed answer names it (`https://<host>`, never the peers origin).
fn mesh_of(host: &str) -> String {
    format!("https://{host}")
}

/// `E_LISTED_ELSEWHERE` (CONTRACT-http-listing L6): a read of a listed name is `not_covered`, with the mesh that holds it.
/// Spec 0.7 adds the code; until then this mesh writes it itself.
pub(crate) fn refusal(l: &Listing) -> Reply {
    let body = Value::Object(vec![
        ("outcome".into(), Value::String("not_covered".into())),
        ("error".into(), Value::String("E_LISTED_ELSEWHERE".into())),
        ("mesh".into(), Value::String(mesh_of(&l.host))),
        ("name".into(), Value::String(l.name.clone())),
    ]);
    Reply::json(404, body.to_jcs(), Cache::NoStore)
}

/// The note a page of this mesh carries about a listed name that ranks first in its subject (CONTRACT-http-listing L7): the
/// relay's page and its takeover page link to it, with the day it was last seen. `None` when `name` is not such a name.
pub(crate) fn rank_one_note(inner: &crate::mesh::Inner, name: &str) -> Option<String> {
    let l = listed(&*inner.store, name)?;
    if !crate::find::rank_one(inner, name) {
        return None;
    }
    let last_seen = inner.store.listed_host(&l.host).ok().flatten().and_then(|r| r.last_seen).unwrap_or_default();
    Some(format!(
        "<p>The source&#39;s own function is listed here and ranks first in its subject (last seen {}): \
         <a href=\"{}\"><code>{}</code></a>.</p>\n",
        escape(&last_seen),
        escape(&crate::fnpage::page_url(inner, &l.name)),
        escape(&l.name)
    ))
}

/// The members a listed name's description carries (describe and the function page as JSON).
fn listed_members(store: &dyn Store, l: &Listing) -> Vec<(String, Value)> {
    let last_seen = store.listed_host(&l.host).ok().flatten().and_then(|r| r.last_seen).unwrap_or_default();
    let descriptor = json::parse(&l.descriptor).unwrap_or(Value::Null);
    let s = |v: &str| Value::String(v.to_string());
    vec![
        ("outcome".into(), s("ok")),
        ("kind".into(), s("function")),
        ("fn".into(), s(&l.name)),
        ("listed".into(), Value::Bool(true)),
        ("mesh".into(), s(&mesh_of(&l.host))),
        ("publisher".into(), s(&l.host)),
        ("address".into(), s(&format!("{}/fn/{}", mesh_of(&l.host), l.name))),
        ("last_seen".into(), s(&last_seen)),
        ("fn_hash".into(), s(&l.fn_hash)),
        ("label".into(), s(&l.label)),
        ("description".into(), s(&l.description)),
        ("descriptor".into(), descriptor),
    ]
}

/// `describe` of a listed name: its listed descriptor, with `listed: true`, the mesh and the day last seen.
pub(crate) fn describe_reply(store: &dyn Store, l: &Listing) -> Reply {
    Reply::json(200, Value::Object(listed_members(store, l)).to_jcs(), Cache::Public(300))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// `/fn/<name>` of a listed name: a short page (no script) with the description, the mesh, a link to the name's page there
/// and the day last seen; as JSON, the members of describe.
pub(crate) fn fn_reply(store: &dyn Store, l: &Listing, json_form: bool) -> Reply {
    if json_form {
        return describe_reply(store, l);
    }
    let members = listed_members(store, l);
    let last_seen = members.iter().find(|(k, _)| k == "last_seen").and_then(|(_, v)| v.as_str()).unwrap_or_default();
    let address = mesh_of(&l.host) + "/fn/" + &l.name;
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{name}</title></head>\
         <body><h1>{name}</h1><p>{description}</p>\
         <p>Listed from <a href=\"{address}\">{mesh}</a>, last seen {last_seen}. This mesh describes the function; \
         it does not serve its values.</p></body></html>",
        name = escape(&l.name),
        description = escape(&l.description),
        address = escape(&address),
        mesh = escape(&mesh_of(&l.host)),
        last_seen = escape(last_seen),
    );
    Reply {
        status: 200,
        body: html.into_bytes().into(),
        content_type: "text/html; charset=utf-8",
        cache: Cache::Public(300),
        canonical: None,
        retry_after: None,
    }
}

/// The note of a relay's page about its source (CONTRACT-http-listing L7): the listed names of the source's domain that rank
/// first in their subject, linked here. `d` is the function page's data; `None` when there are none.
pub(crate) fn relay_note(inner: &crate::mesh::Inner, d: &Value) -> Option<String> {
    let domain = d.get("lineage")?.get("source")?.get("name")?.as_str()?;
    let notes: String =
        inner.store.listings_of(domain).ok()?.iter().filter_map(|l| rank_one_note(inner, &l.name)).collect();
    (!notes.is_empty()).then_some(notes)
}
