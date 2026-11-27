//! Listing another mesh (CONTRACT-http-listing, http 0.6.0): the functions of a mesh this mesh does not host, as
//! descriptions only, with no cells stored.
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
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use datumesh_core::json::{self, Value};
use datumesh_core::lexical;
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
    /// A request for a name did not answer with a result (a 429, a 5xx, no answer, or a page that is not one), or the
    /// catalogue could not be read on: that name, and the names past it, keep their listings (C1, C2, smaller 7).
    Unreached,
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

/// An instant as a string that compares in time order: `YYYY-MM-DDTHH:MM:SS` and the fraction to nine digits (smaller 6).
fn instant_key(s: &str) -> String {
    let s = s.strip_suffix('Z').unwrap_or(s);
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    let frac = &frac[..frac.len().min(9)];
    format!("{whole}.{frac:0<9}")
}

/// The key of `doc` valid at `now` (`valid_from <= now < valid_to`), if any; instants compare as instants.
fn key_valid_at(doc: &KeyDocument, now: &str) -> Option<String> {
    let now = instant_key(now);
    doc.keys
        .keys
        .iter()
        .find(|k| instant_key(&k.valid_from) <= now && k.valid_to.as_deref().is_none_or(|t| now < instant_key(t)))
        .map(|k| k.id.clone())
}

/// The next page of the catalogue, when this run follows it (B2): a path under the host's own slice, relative, or an
/// `https://<host>` link re-rooted on `origin`. Anything else (another origin, `http://`) is not fetched.
fn next_page(host: &str, origin: &str, next: &str) -> Option<String> {
    let slice = format!("/slice/{host}/");
    if next.starts_with(&slice) {
        return Some(format!("{origin}{next}"));
    }
    let rest = next.strip_prefix(&format!("https://{host}"))?;
    rest.starts_with(&slice).then(|| format!("{origin}{rest}"))
}

/// The subjects of each name of a catalogue, by name.
type Members = BTreeMap<String, BTreeSet<SubjectKeys>>;

/// The function name's subjects and source names, from the host's signed catalogue cells (L2.3): each member's receipt
/// is verified against the host's key document; a member that does not verify is skipped. Pages are followed. A page
/// after the first that cannot be read ends the catalogue as a cut (C1, C2), keeping the members read so far. When no
/// page was read (a 404, a 5xx, no answer, a time-out), the catalogue cannot be read: `Stop::Unreachable` (N1).
fn catalogue(run: &mut Run, host: &str, origin: &str, doc: &KeyDocument) -> Result<(Members, Option<Cut>), Stop> {
    let mut keys = Keys::new();
    keys.insert_document(doc.clone());
    let mut out: BTreeMap<String, BTreeSet<SubjectKeys>> = BTreeMap::new();
    let mut url = format!("{origin}/slice/{host}/catalog/subject/source?domain=*&measure=*&rank=*&region=*&thing=*");
    // The cell of a subject's source: the method `source` of the class `<host>/catalog/subject` (receipt `ns` and `fn`).
    let source_ns = format!("{host}/catalog/subject");
    // N1 (re-review): a catalogue none of whose pages was read cannot be read, so the host is unreachable; a stop on a
    // later page is a cut, keeping the members read.
    let mut pages = 0usize;
    loop {
        let (status, body) = match run.get(&url) {
            Ok(x) => x,
            Err(Stop::Bytes) => return Ok((out, Some(Cut::Bytes))),
            Err(Stop::Unreachable(detail)) => {
                if pages == 0 {
                    return Err(Stop::Unreachable(detail));
                }
                let cut = if run.left().is_none() { Cut::Time } else { Cut::Unreached };
                return Ok((out, Some(cut)));
            }
        };
        let page = if status == 200 { json::parse(&body).ok() } else { None };
        let Some(page) = page else {
            if pages == 0 {
                return Err(Stop::Unreachable(format!("{url}: status {status}")));
            }
            return Ok((out, Some(Cut::Unreached)));
        };
        pages += 1;
        if let Some(Value::Array(members)) = page.get("members") {
            for m in members {
                let Some(receipt) = m.get("receipt") else { continue };
                let r = datumesh_core::verify(&receipt.to_jcs(), &keys, None, None);
                if r.error().is_some() {
                    continue;
                }
                let Some(cell) = receipt.get("cell") else { continue };
                // Only the host's own source cells name sources (smaller 3): their `ns` and `fn`, a rank and the four keys.
                let is_source = cell.get("fn").and_then(Value::as_str) == Some("source")
                    && cell.get("ns").and_then(Value::as_str) == Some(source_ns.as_str());
                if !is_source {
                    continue;
                }
                let arg = |k: &str| -> Option<String> {
                    cell.get("args")?.get(k)?.get("value")?.as_str().map(str::to_string)
                };
                let (Some(d), Some(t), Some(me), Some(rg)) =
                    (arg("domain"), arg("thing"), arg("measure"), arg("region"))
                else {
                    continue;
                };
                if cell.get("args").and_then(|a| a.get("rank")).is_none() {
                    continue;
                }
                let value = receipt.get("value");
                let name = value.and_then(|v| v.as_str().or_else(|| v.get("value").and_then(Value::as_str)));
                let Some(name) = name else { continue };
                let under = name.starts_with(&format!("{host}/"));
                if under && lexical::check_function_name(name, "name").is_ok() {
                    out.entry(name.to_string()).or_default().insert(SubjectKeys::new(&d, &t, &me, &rg));
                }
            }
        }
        match page.get("next") {
            None | Some(Value::Null) => break,
            Some(Value::String(next)) => match next_page(host, origin, next) {
                Some(u) => url = u,
                None => return Ok((out, Some(Cut::Unreached))),
            },
            Some(_) => return Ok((out, Some(Cut::Unreached))),
        }
    }
    Ok((out, None))
}

/// What one name's requests made of it.
enum NameStep {
    /// Verified: describe, descriptor and function page (its label and description).
    Got(FetchedName),
    /// Reached, and the answer does not verify or the name is gone (describe 404, a hash that does not match): refused.
    Refused,
    /// Not reached: a 429, a 5xx or a page that is not an answer (C2, smaller 7). The stored listing stays as it is.
    NotReached,
}

/// One name's requests (see [`NameStep`]); a request that stops the run is `Err`.
fn name_step(run: &mut Run, origin: &str, name: &str, subjects: Vec<SubjectKeys>) -> Result<NameStep, Stop> {
    let base = format!("{origin}/");
    let (status, d) = run.get(&format!("{base}describe/{name}"))?;
    if status == 404 {
        return Ok(NameStep::Refused);
    }
    if status != 200 {
        return Ok(NameStep::NotReached);
    }
    let fn_hash = json::parse(&d)
        .ok()
        .and_then(|v| v.get("fn_hash").and_then(Value::as_str).map(str::to_string))
        .filter(|h| lexical::check_hex(h, 64, "fn_hash").is_ok());
    let Some(fn_hash) = fn_hash else { return Ok(NameStep::Refused) };
    let (status, bytes) = run.get(&format!("{base}descriptor/{fn_hash}"))?;
    if status != 200 {
        return Ok(NameStep::NotReached);
    }
    if datumesh_core::function_hash(&bytes).ok().as_deref() != Some(fn_hash.as_str()) {
        return Ok(NameStep::Refused);
    }
    let (status, f) = run.get(&format!("{base}fn/{name}?format=json"))?;
    let page = if status == 200 { json::parse(&f).ok() } else { None };
    let Some(v) = page else { return Ok(NameStep::NotReached) };
    let label = v.get("label").and_then(Value::as_str).unwrap_or_default().to_string();
    let description = v.get("description").and_then(Value::as_str).unwrap_or_default().to_string();
    if description.is_empty() {
        return Ok(NameStep::NotReached);
    }
    Ok(NameStep::Got(FetchedName { name: name.to_string(), fn_hash, subjects, descriptor: bytes, label, description }))
}

/// Records the first cut of a run (the first limit reached is the one reported).
fn first_cut(cut: &mut Option<Cut>, c: Cut) {
    if cut.is_none() {
        *cut = Some(c);
    }
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
    // 3. the host's own catalogue, verified and not stored (a cut keeps the members read)
    let (subjects, mut cut) = match catalogue(&mut run, host, origin, &doc) {
        Ok(x) => x,
        Err(s) => return unreachable(s),
    };
    let mut all: Vec<(String, Vec<SubjectKeys>)> =
        subjects.into_iter().map(|(n, s)| (n, s.into_iter().collect())).collect();
    if all.len() > limits.names {
        all.truncate(limits.names);
        first_cut(&mut cut, Cut::Names);
    }
    // 4. each name: describe, descriptor, and its label and description
    let mut names = Vec::new();
    let mut refused = Vec::new();
    for (name, subjects) in all {
        if run.left().is_none() {
            first_cut(&mut cut, Cut::Time);
            break;
        }
        match name_step(&mut run, origin, &name, subjects) {
            Ok(NameStep::Got(n)) => names.push(n),
            Ok(NameStep::Refused) => refused.push(name),
            Ok(NameStep::NotReached) => first_cut(&mut cut, Cut::Unreached),
            Err(Stop::Bytes) => {
                first_cut(&mut cut, Cut::Bytes);
                break;
            }
            Err(Stop::Unreachable(_)) => {
                if run.left().is_none() {
                    first_cut(&mut cut, Cut::Time);
                    break;
                }
                first_cut(&mut cut, Cut::Unreached);
            }
        }
    }
    Fetch::Fetched { key_id, names, refused, cut }
}

/// The most subjects a listed host may create in this mesh's catalogue (B4): a subject no member bound here holds. A name
/// whose subjects would pass it is cut, in name order.
const MAX_CREATED_SUBJECTS: usize = 64;

/// Applies a fetch of `host` at `now` (an instant) to the store (L3, L8, the re-review's N1, N3, N4):
///
/// * [`Fetch::Fetched`]: each name of `names` whose descriptor hashes to its `fn_hash` is put, with the subjects it has
///   and, in a cut run, those it held (N4: a cut keeps what a name was in); a name the store refuses (its bounds, or
///   another source's entry of the name, `Invalid`) is refused, and the run goes on. The subjects this run may create here
///   are at most `MAX_CREATED_SUBJECTS` (B4), counted across runs: the subjects the host's held listings create count, the
///   kept ones too (N3); a name over it is cut, and the run is a cut (`Cut::Names`). Without a cut, every listing of the host
///   not among the verified names drops out; with a cut, the `refused` ones drop, and so does a listing not reached by a run
///   whose catalogue was read (`Cut::Unreached`) once it has been unreached for seven days (its own clock, N1). A verified
///   name's clock is cleared. The host's row: `last_seen` the day of `now`, `unreachable_since` none, written only when that
///   changes it.
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
    let held: Vec<Listing> = store.listings_of(host).map_err(|e| se(&e))?;
    let existing: Vec<String> = held.iter().map(|l| l.name.clone()).collect();
    let mut drop_all = false;
    let mut drop_set: BTreeSet<String> = BTreeSet::new();
    match fetch {
        Fetch::Fetched { key_id, names, refused, cut } => {
            let mut verified = BTreeSet::new();
            let offered: BTreeSet<&str> = names.iter().map(|n| n.name.as_str()).collect();
            let mut refuse: BTreeSet<String> = refused.iter().cloned().collect();
            let stored: BTreeMap<&str, &Listing> = held.iter().map(|l| (l.name.as_str(), l)).collect();
            // The subjects no member bound here holds: a listing creates these (B4), at most MAX_CREATED_SUBJECTS. The
            // held listings that stay (offered, or kept by a cut run) count from the start (N3).
            let bound = crate::catalog_build::bound_subjects(store).map_err(|e| se(&e))?;
            let staying =
                |l: &Listing| offered.contains(l.name.as_str()) || (cut.is_some() && !refused.contains(&l.name));
            let mut created: BTreeSet<SubjectKeys> = held
                .iter()
                .filter(|l| staying(l))
                .flat_map(|l| l.subjects.iter().cloned())
                .filter(|s| !bound.contains(s))
                .collect();
            let mut cap_cut = false;
            let mut ordered: Vec<&FetchedName> = names.iter().collect();
            ordered.sort_by(|a, b| a.name.cmp(&b.name));
            for n in ordered {
                let under = n.name.starts_with(&format!("{host}/"));
                let ok =
                    under && datumesh_core::function_hash(&n.descriptor).ok().as_deref() == Some(n.fn_hash.as_str());
                if !ok {
                    refuse.insert(n.name.clone());
                    continue;
                }
                let mut subjects = n.subjects.clone();
                if cut.is_some() {
                    // N4: a cut run may have read only some of the catalogue's pages: the subjects it held stay.
                    if let Some(s) = stored.get(n.name.as_str()) {
                        subjects.extend(s.subjects.iter().cloned());
                    }
                }
                subjects.sort();
                subjects.dedup();
                let new: Vec<SubjectKeys> =
                    subjects.iter().filter(|s| !bound.contains(*s) && !created.contains(*s)).cloned().collect();
                if created.len() + new.len() > MAX_CREATED_SUBJECTS {
                    cap_cut = true;
                    continue;
                }
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
                match store.put_listing(&l) {
                    Ok(ListingWrite::Unchanged) => {}
                    Ok(ListingWrite::Inserted) => {
                        applied.added.push(n.name.clone());
                        applied.writes += 1;
                    }
                    Ok(ListingWrite::Changed) => {
                        applied.changed.push(n.name.clone());
                        applied.writes += 1;
                    }
                    // A name the store refuses (its bounds, or another source's catalogue entry of it) is refused, and the
                    // run goes on with the others (B3, C3); nothing of it was written.
                    Err(datumesh_store::Error::Invalid(_)) => {
                        refuse.insert(n.name.clone());
                        continue;
                    }
                    Err(e) => return Err(se(&e)),
                }
                // Reached: its own clock (N1) is cleared, if it had one.
                if store.set_listing_unreached(&n.name, None).map_err(|e| se(&e))? {
                    applied.writes += 1;
                }
                created.extend(new);
                verified.insert(n.name.clone());
            }
            let cut = cut.or(cap_cut.then_some(Cut::Names));
            match cut {
                None => drop_set.extend(existing.iter().filter(|e| !verified.contains(*e)).cloned()),
                Some(c) => {
                    drop_set.extend(existing.iter().filter(|e| refuse.contains(*e)).cloned());
                    if c == Cut::Unreached {
                        // N1: a held name the run did not reach (not offered, not refused) has a clock of its own: set at the
                        // first such run, cleared when it answers, dropped seven days after it was set.
                        for name in existing.iter().filter(|e| !offered.contains(e.as_str()) && !refuse.contains(*e)) {
                            let since = match store.listing_unreached(name).map_err(|e| se(&e))? {
                                Some(since) => since,
                                None => {
                                    store.set_listing_unreached(name, Some(now)).map_err(|e| se(&e))?;
                                    applied.writes += 1;
                                    now.to_string()
                                }
                            };
                            let due = crate::keynames::add_seconds(&since, UNREACHABLE_DROP_AFTER.as_secs());
                            if due.is_some_and(|d| d.as_str() <= now) {
                                drop_set.insert(name.clone());
                            }
                        }
                    }
                    applied.cut = Some(c);
                    applied.outcome = Outcome::Cut;
                }
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

/// Whether listings are looked up at all: only with `DATUMESH_LISTED_MESHES` naming a host (smaller 1). With the list empty
/// no request reads the listings table.
fn active(inner: &crate::mesh::Inner) -> bool {
    !inner.config.listed_meshes.is_empty()
}

/// Whether `word` is a name under a host of `DATUMESH_LISTED_MESHES` (N2): only such a word is looked up, so a read of any
/// other name costs no store lookup.
fn under_listed(inner: &crate::mesh::Inner, word: &str) -> bool {
    word.split_once('/').is_some_and(|(host, _)| inner.config.listed_meshes.iter().any(|h| h == host))
}

/// One counted store lookup of a listing (`datumesh_listing_lookups_total`).
pub(crate) fn lookup(inner: &crate::mesh::Inner, name: &str) -> Result<Option<Listing>, ListingError> {
    inner.listing_lookups.fetch_add(1, Ordering::Relaxed);
    inner.store.listing(name).map_err(|e| ListingError { detail: e.to_string() })
}

/// The listing of `name` when it is a listed name here. With no listed host nothing is looked up. A store failure is an
/// error (a read that depends on it fails closed, L6).
pub(crate) fn checked(inner: &crate::mesh::Inner, name: &str) -> Result<Option<Listing>, ListingError> {
    if !active(inner) {
        return Ok(None);
    }
    lookup(inner, name)
}

/// The listing of `name`, or `None` when it is not one (a store failure reads as not listed: for the pages, not for the
/// refusals, which use [`checked`]).
pub(crate) fn listed(inner: &crate::mesh::Inner, name: &str) -> Option<Listing> {
    checked(inner, name).ok().flatten()
}

/// `text` with a leading alias (`@a` of `@a(...)`, `@a/...`) replaced by its target, when `alias` knows it (B1).
fn resolve_alias(text: &str, alias: &dyn Fn(&str) -> Option<String>) -> String {
    let Some(rest) = text.strip_prefix('@') else { return text.to_string() };
    let end = rest.find(['(', '/', '?', ' ']).unwrap_or(rest.len());
    match alias(&rest[..end]) {
        Some(target) => format!("{target}{}", &rest[end..]),
        None => text.to_string(),
    }
}

/// The listed name a read names (B1): `text` is a read's path rest, a call or a name, `name`, `name(args)` or `name?query`,
/// a series page below a name or an alias of one. Each prefix under a listed host is looked up through `bindings` (N2: a
/// batch's bindings remember each lookup), the first one returned.
pub(crate) fn checked_in(
    inner: &crate::mesh::Inner,
    bindings: &dyn crate::call::Bindings,
    alias: &dyn Fn(&str) -> Option<String>,
    text: &str,
) -> Result<Option<Listing>, ListingError> {
    if !active(inner) {
        return Ok(None);
    }
    let text = resolve_alias(text.strip_prefix("cell://").unwrap_or(text), alias);
    let mut ends: Vec<usize> =
        text.char_indices().filter(|(_, c)| matches!(c, '/' | '(' | '?' | ' ')).map(|(i, _)| i).collect();
    ends.push(text.len());
    for e in ends {
        if under_listed(inner, &text[..e]) {
            if let Some(l) = bindings.listing(&text[..e])? {
                return Ok(Some(l));
            }
        }
    }
    Ok(None)
}

/// The listed name a text names among its words (a formula's, or a scenario's body): any word under a listed host that is
/// one. Each distinct word is looked up once (N2).
pub(crate) fn checked_words(inner: &crate::mesh::Inner, text: &str) -> Result<Option<Listing>, ListingError> {
    if !active(inner) {
        return Ok(None);
    }
    let words: BTreeSet<&str> =
        text.split(|c: char| !(c.is_alphanumeric() || "._/-".contains(c))).filter(|w| under_listed(inner, w)).collect();
    for w in words {
        if let Some(l) = lookup(inner, w)? {
            return Ok(Some(l));
        }
    }
    Ok(None)
}

/// The listed name a formula (the body of `POST /x/calc`) reads: one of its words is a listed name.
pub(crate) fn checked_formula(inner: &crate::mesh::Inner, body: &[u8]) -> Result<Option<Listing>, ListingError> {
    if !active(inner) {
        return Ok(None);
    }
    let formula = json::parse(body).ok().and_then(|v| v.get("formula").and_then(Value::as_str).map(str::to_string));
    match formula {
        Some(f) => checked_words(inner, &f),
        None => Ok(None),
    }
}

/// The reply of a read that depends on the listings when the store fails (fail closed, L6): `503`.
pub(crate) fn unavailable() -> Reply {
    crate::envelope::error_reply(datumesh_core::ErrorCode::EvalUnavailable)
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

/// The host's own origin as a listed answer names it (`https://<host>`, never the peers origin).
fn mesh_of(host: &str) -> String {
    format!("https://{host}")
}

/// The note a page carries about the listed name that ranks first in the subject where `name` (a relay of a source) has its
/// place (CONTRACT-http-listing L7): the page links to it, with the day it was last seen. `None` when that first name is not
/// a listed one (or is `name` itself).
fn first_note(inner: &crate::mesh::Inner, name: &str) -> Option<String> {
    let built = crate::catalog_view::built(inner)?;
    let subject = &built.places.get(name)?.subject;
    let first = built.firsts.get(subject)?;
    if first == name {
        return None;
    }
    let l = listed(inner, first)?;
    let last_seen = inner.store.listed_host(&l.host).ok().flatten().and_then(|r| r.last_seen).unwrap_or_default();
    Some(format!(
        "<p>The source&#39;s own function is listed here and ranks first in its subject (last seen {}): \
         <a href=\"{}\"><code>{}</code></a>.</p>\n",
        escape(&last_seen),
        escape(&crate::fnpage::page_url(inner, &l.name)),
        escape(&l.name)
    ))
}

/// The note of a relay's function page about its source (CONTRACT-http-listing L7): the listed name that ranks first in the
/// relay's subject. `d` is the function page's data; `None` when there is none.
pub(crate) fn relay_note(inner: &crate::mesh::Inner, d: &Value) -> Option<String> {
    first_note(inner, d.get("name").or_else(|| d.get("fn"))?.as_str()?)
}

/// The note of a takeover page about its domain (CONTRACT-http-listing L7): [`first_note`] of each of the domain's relays,
/// each note once.
pub(crate) fn takeover_note(inner: &crate::mesh::Inner, relays: &[String]) -> String {
    let mut notes: Vec<String> = Vec::new();
    for r in relays {
        if let Some(n) = first_note(inner, r) {
            if !notes.contains(&n) {
                notes.push(n);
            }
        }
    }
    notes.concat()
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
