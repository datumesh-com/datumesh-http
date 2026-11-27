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

use std::time::Duration;

use datumesh_store::{Store, SubjectKeys};

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
    let _ = (config, host);
    todo!("listing 0.6.0: origin_of")
}

/// Fetches `host` at `origin` (L2), read-only, within `limits`; `now` (an instant) is the time the key document's keys
/// must be valid at.
pub fn fetch(host: &str, origin: &str, limits: &Limits, now: &str) -> Fetch {
    let _ = (host, origin, limits, now);
    todo!("listing 0.6.0: fetch")
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
    let _ = (store, host, fetch, now);
    todo!("listing 0.6.0: apply")
}

/// [`fetch`] then [`apply`].
pub fn run_host<S: Store + ?Sized>(
    store: &S,
    host: &str,
    origin: &str,
    limits: &Limits,
    now: &str,
) -> Result<Applied, ListingError> {
    let _ = (store, host, origin, limits, now);
    todo!("listing 0.6.0: run_host")
}
