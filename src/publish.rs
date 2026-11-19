//! Publishing stored cells as the mesh (`datumesh-mesh publish-stored`): a JSON lines file of
//! stored functions and their cells, signed with the mesh's key, put in the store.
//!
//! Each line is one of:
//!
//! * a descriptor (since 0.3.1, CONTRACT-1.7a H1): `{"descriptor": <function descriptor>}`: the
//!   descriptor is put with `put_descriptor` and binds nothing. Only a stored function is
//!   accepted (`code.lang` = `stored`, `code.body` = `""`, `inputs` = `[]`); any other is refused
//!   at its line. A class later in the file (or in a later run) may name its hash as a method;
//! * a function: `{"function": "<ns>/<name>", "signature": {...}, "bound": "<instant>"}`: the
//!   descriptor `{"code": {"body": "", "lang": "stored"}, "inputs": [], "signature": ...}`
//!   is put and the name bound to it from `bound` (unless it already is);
//! * a formula function: `{"formula": "<ns>/<name>", "source": "<formula>", "signature":
//!   {...}, "bound": "<instant>"}`: defined with `datumesh_formula::definition::define_function`
//!   (its callees as bound at `bound`) and published (`put_descriptor`, `bind_name` from
//!   `bound`) unless the name is already bound to that function; its cells are evaluated on
//!   demand by the mesh;
//! * a class (since 0.2.2, spec-0.5.0, 16.2): `{"class": "<ns>/<name>", "descriptor": {...as
//!   written...}, "bound": "<instant>"}`: defined with the formula crate's `define_class` (core's
//!   rules: the class is under one of the mesh's own namespaces, else `E_CLASS_OWNER`; the method
//!   functions are those bound as of `bound`), `put_class`, and `bind_class` from `bound` unless the
//!   name is already bound to that class hash. Method functions come first in the file (or in an
//!   earlier one), `bound` later than theirs. A line with both `class` and `descriptor` is a class
//!   line;
//! * a cell: `{"cell": {"fn": "<ns>/<name>", "args": {<name>: <typed scalar>}}, "value": <typed
//!   scalar> or {"blob": "<file>", "media_type": "<type>"}, "tx"?, "asof"?, "issued"?, "licence",
//!   "summary"}`: a blob's file (relative to the JSON lines file) is put with `put_blob` and the
//!   value is `<sha256>:<length>:<media type>`; the receipt is signed with the mesh's key
//!   (`computed_by` the mesh host, `publisher` the cell's namespace, `valid` its valid-time
//!   arguments; `issued` defaults to the later of `asof` and the key's start) and stored with
//!   `put_receipt` and `put_version`. `tx` and `asof` default to the run's clock now (since
//!   0.3.1, H4), each independently. A `source` object (since 0.4.0, spec-0.6.0 C2: `name`, `url`,
//!   `retrieved`, and optionally `sha256` and `takeover`, nothing else) marks the value as relayed: the
//!   receipt signs it, and its trust is `relayed`; its rules are the receipt's (`retrieved` no later
//!   than `issued`, a publisher that does not relay itself, `https://` URLs), refused at the line.
//!
//! The function of a cell is the one its name is bound to as of the cell's `tx`. Re-running a
//! file changes nothing (a cell line without `tx` whose latest version states the same thing is
//! skipped, since 0.3.1): descriptors and blobs are content-addressed, signing is deterministic
//! (the same receipt, the same hash), and a version already there is skipped. The key is never
//! logged.
//!
//! Rule A (since 0.3.0, CONTRACT-1.8b H1): no binding starts earlier than the mesh clock. The
//! start of a binding (a name's, a class's, or a class method's `N/m`) is its `bound`, or the
//! clock's now when `bound` is left out, and a start earlier than now is refused with
//! [`PublishErrorKind::StartBeforeNow`]. A `--backfill` run ([`PublishOptions::backfill`]) lifts
//! the start check, and is refused ([`PublishErrorKind::Backfill`]) when a name it would bind has
//! a binding, or when a namespace of such a name has any binding (store 0.4.2's
//! `has_bindings_under`). A run that writes appends one line to the log: `{"at": …, "namespaces":
//! […], "starts": […]}`, a JCS object.
//!
//! No partial writes (since 0.3.1, CONTRACT-1.7a H2): the file is read in two passes. The first
//! plans every write the second one makes, and checks each as the store will: a bind is refused
//! when its start is not later than the latest binding of its name (the store's, or one planned
//! earlier in the file, W1b), when the name is of the other kind (a class or a function), or when
//! its namespace is suspended; a cell's function must be bound as of its `tx` (counting the
//! earlier binds of the file), its blob must be readable, `tx <= asof <= issued`, and a version
//! at or after its `tx` must be this receipt. Receipts are signed in the first pass (signing is
//! deterministic), so their hashes are known. A file that the first pass refuses writes nothing.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use datumesh_core::json::{self, Value};
use datumesh_core::{
    sha256_hex, Arg, Args, Cell, ClassDescriptor, FunctionDescriptor, Receipt, ReceiptBuilder, Scalar, SigningKey,
    Source, Type,
};
use datumesh_engine::{Clock, SystemClock};
use datumesh_formula::definition::{Callee, Callees, ClassCallee};
use datumesh_store::{Binding, CellWrite, Namespace, Store};

use crate::testrun::{self, run_test, Overlay, Ran, RowWrite, TestSpec};

use crate::config::ConfigError;

/// The end of time: a binding as of it is the latest one.
const END: &str = "9999-12-31T23:59:59.999999Z";

/// Cell lines planned together, and cell writes per `put_cells` call (CONTRACT-perf H1, H2).
const CHUNK: usize = 1_000;

/// The environment variable of the publish lanes (CONTRACT-perf H5), read once per run.
const LANES_VAR: &str = "DATUMESH_PUBLISH_LANES";

/// The lanes of a run (CONTRACT-perf H5): `DATUMESH_PUBLISH_LANES` (a positive integer), else
/// `min(4, available_parallelism)`, at least 1. An invalid value refuses the run before any write.
fn publish_lanes() -> Result<usize, PublishError> {
    let bad = |detail: String| PublishError::at(0, ConfigError::new(LANES_VAR, detail).to_string());
    match std::env::var(LANES_VAR) {
        Err(std::env::VarError::NotPresent) => Ok(std::thread::available_parallelism().map_or(1, |n| n.get()).min(4)),
        Ok(v) => match v.parse::<usize>() {
            Ok(n) if n > 0 => Ok(n),
            _ => Err(bad(format!("must be a positive integer, not {v:?}"))),
        },
        Err(e) => Err(bad(e.to_string())),
    }
}

/// What a run put.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Published {
    /// Functions newly bound (stored and formula).
    pub functions: usize,
    /// Classes newly bound (since 0.2.2).
    pub classes: usize,
    /// Cell lines read.
    pub cells: usize,
    /// Receipts stored (new or already there).
    pub receipts: usize,
    /// Versions newly added.
    pub versions: usize,
    /// Blobs put (new or already there).
    pub blobs: usize,
    /// Blob bytes put.
    pub blob_bytes: u64,
    /// Descriptors newly put by descriptor lines (since 0.3.1); a re-run counts 0.
    pub descriptors: u64,
}

/// How a run reads the clock and whether it backfills (CONTRACT-1.8b H1).
#[derive(Clone)]
pub struct PublishOptions {
    /// The mesh clock: the start of a binding left out is its now, and a start earlier than it is
    /// refused (rule A).
    pub clock: Arc<dyn Clock + Send + Sync>,
    /// The operator log of a `--backfill` run (`DATUMESH_BACKFILL_LOG`). `Some` lifts the start
    /// check, for a file whose names and namespaces have no bindings yet, and appends a line to
    /// the log for each run that writes.
    pub backfill: Option<PathBuf>,
}

impl PublishOptions {
    /// The system clock, and no backfill.
    pub fn system() -> PublishOptions {
        PublishOptions { clock: Arc::new(SystemClock), backfill: None }
    }
}

/// Why a line (or the file, at line 0) was refused.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublishErrorKind {
    /// Any other refusal: [`PublishError::detail`] says what.
    Line,
    /// A binding's start is earlier than the mesh clock (rule A).
    StartBeforeNow {
        /// The name (or class name, or method name) that would bind.
        name: String,
        /// Its start.
        start: String,
        /// The mesh clock.
        now: String,
    },
    /// A `--backfill` run that would write over a binding, or under a namespace with one.
    Backfill(String),
    /// `import-vectors`: a name the vectors would bind is under a namespace the mesh owns.
    OwnedVectorName(String),
    /// The public rule (D8, Build 2 Part 3, `publish_stored_checked`): a computed function's tests
    /// do not cover the derived cases or every branch. The text lists what is missing.
    TestsMissing(String),
    /// The public rule: a test of a computed function fails. The text names the case and the
    /// numbers.
    TestsFailing(String),
}

impl std::fmt::Display for PublishErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PublishErrorKind::Line => f.write_str("refused"),
            PublishErrorKind::StartBeforeNow { name, start, now } => {
                write!(f, "binding start {start} of {name} is earlier than the mesh clock {now} (rule A)")
            }
            PublishErrorKind::Backfill(why) => write!(f, "--backfill refused: {why}"),
            PublishErrorKind::OwnedVectorName(name) => {
                write!(f, "{name} is under a namespace the mesh owns: import-vectors does not bind it")
            }
            PublishErrorKind::TestsMissing(text) => write!(f, "E_TESTS_MISSING: {text}"),
            PublishErrorKind::TestsFailing(text) => write!(f, "E_TESTS_FAILING: {text}"),
        }
    }
}

/// Why a run stopped: the line, what was wrong, and its typed kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishError {
    /// The line number (1-based; 0 for the file).
    pub line: usize,
    /// Why, in words.
    pub detail: String,
    /// Why, typed.
    pub kind: PublishErrorKind,
}

impl PublishError {
    fn at(line: usize, detail: impl Into<String>) -> PublishError {
        PublishError { line, detail: detail.into(), kind: PublishErrorKind::Line }
    }

    pub(crate) fn typed(line: usize, kind: PublishErrorKind) -> PublishError {
        PublishError { line, detail: kind.to_string(), kind }
    }
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.detail)
    }
}

impl std::error::Error for PublishError {}

fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

/// The namespace of a name: the name minus its last `/` segment.
fn namespace_of(name: &str) -> &str {
    name.rsplit_once('/').map(|(ns, _)| ns).unwrap_or("")
}

/// The host of a name: its first segment.
fn host_of(name: &str) -> &str {
    name.split('/').next().unwrap_or("")
}

/// The definer of a class `name`: its own namespace when the mesh owns the name's host, else the
/// mesh's host (which no class name under another host has, so core answers `E_CLASS_OWNER`).
fn definer<'a>(name: &'a str, host: &'a str, owned: &[String]) -> &'a str {
    let host_of_name = name.split('/').next().unwrap_or("");
    if owned.iter().any(|o| o == host_of_name) {
        namespace_of(name)
    } else {
        host
    }
}

/// The start of a line's binding: its `bound`, or the clock's now when it is left out.
fn bound_of(v: &Value, line: usize, now: &str) -> Result<String, PublishError> {
    match v.get("bound") {
        None => Ok(now.to_string()),
        Some(b) => b.as_str().map(str::to_string).ok_or_else(|| PublishError::at(line, "bound is not an instant")),
    }
}

/// The bindings and versions a pass one has planned so far: the file's own, which are seen before
/// the store's.
#[derive(Default)]
struct Simulation {
    /// Function name to the (tx, function hash) of each bind the file would make.
    functions: BTreeMap<String, Vec<(String, String)>>,
    /// Class name to the (tx, class hash) of each class bind the file would make.
    classes: BTreeMap<String, Vec<(String, String)>>,
    /// Canonical bytes by function hash, of the descriptors the file would store.
    descriptors: BTreeMap<String, Vec<u8>>,
    /// Canonical bytes by class hash, of the classes the file would store.
    class_bytes: BTreeMap<String, Vec<u8>>,
    /// Cell id to the (tx, receipt hash) of each version the file would add, in tx order.
    versions: BTreeMap<String, Vec<(String, String)>>,
}

impl Simulation {
    /// The latest of `binds`' entries for `name` with tx <= `at`.
    fn latest(binds: &BTreeMap<String, Vec<(String, String)>>, name: &str, at: &str) -> Option<(String, String)> {
        binds.get(name)?.iter().filter(|(t, _)| t.as_str() <= at).max_by(|a, b| a.0.cmp(&b.0)).cloned()
    }
}

/// The callees of a definition: the store's bindings, and with `sim` the file's own over them.
struct Callers<'a, S: Store + ?Sized> {
    store: &'a S,
    sim: Option<&'a Simulation>,
}

impl<S: Store + ?Sized> Callers<'_, S> {
    fn function_bytes(&self, hash: &str) -> Result<Option<Vec<u8>>, String> {
        if let Some(b) = self.sim.and_then(|s| s.descriptors.get(hash)) {
            return Ok(Some(b.clone()));
        }
        Ok(self.store.descriptor(hash).map(|b| b.into_owned()))
    }

    fn class_bytes(&self, hash: &str) -> Result<Option<Vec<u8>>, String> {
        if let Some(b) = self.sim.and_then(|s| s.class_bytes.get(hash)) {
            return Ok(Some(b.clone()));
        }
        self.store.class(hash).map_err(|e| e.to_string())
    }
}

impl<S: Store + ?Sized> Callees for Callers<'_, S> {
    fn callee(&self, name: &str, at: &str) -> Result<Option<Callee>, String> {
        let stored = self.store.resolve_binding(name, at).map_err(|e| e.to_string())?;
        let simmed = self.sim.and_then(|s| Simulation::latest(&s.functions, name, at));
        let (fn_hash, start) = match (stored, simmed) {
            (Some(b), Some((t, h))) if t >= b.tx => (h, t),
            (Some(b), _) => (b.fn_hash, b.tx),
            (None, Some((t, h))) => (h, t),
            (None, None) => return Ok(None),
        };
        let bytes = self.function_bytes(&fn_hash)?.ok_or_else(|| format!("descriptor {fn_hash} is not stored"))?;
        let descriptor = json::parse(&bytes).map_err(|e| e.to_string())?;
        Ok(Some(Callee { fn_hash, start, descriptor }))
    }

    fn bound_class(&self, name: &str, at: &str) -> Result<Option<ClassCallee>, String> {
        let stored = self.store.resolve_class_binding(name, at).map_err(|e| e.to_string())?;
        let simmed = self.sim.and_then(|s| Simulation::latest(&s.classes, name, at));
        let (class_hash, start) = match (stored, simmed) {
            (Some(b), Some((t, h))) if t >= b.tx => (h, t),
            (Some(b), _) => (b.class_hash, b.tx),
            (None, Some((t, h))) => (h, t),
            (None, None) => return Ok(None),
        };
        let bytes = self.class_bytes(&class_hash)?.ok_or_else(|| format!("class {class_hash} is not stored"))?;
        let descriptor = ClassDescriptor::from_json(&bytes).map_err(|e| e.to_string())?;
        Ok(Some(ClassCallee { class_hash, start, descriptor }))
    }

    fn class_descriptor(&self, class_hash: &str) -> Result<Option<ClassDescriptor>, String> {
        let Some(bytes) = self.class_bytes(class_hash)? else { return Ok(None) };
        ClassDescriptor::from_json(&bytes).map(Some).map_err(|e| e.to_string())
    }

    fn function_descriptor(&self, fn_hash: &str) -> Result<Option<Value>, String> {
        let Some(bytes) = self.function_bytes(fn_hash)? else { return Ok(None) };
        json::parse(&bytes).map(Some).map_err(|e| e.to_string())
    }
}

/// What a run reads from the store again and again, kept for the run (CONTRACT-perf H1). The
/// store does not change during the first pass, so each answer is the same whenever it is asked,
/// and a failed read is kept as its message, which is what the check that asks it reports. A
/// prefetch that fails keeps nothing: each line then asks for itself and fails at its own line.
#[derive(Default)]
struct Reads {
    bindings: HashMap<(String, String), Result<Option<Binding>, String>>,
    descriptors: HashMap<String, Option<Vec<u8>>>,
    namespaces: HashMap<String, Result<Option<Namespace>, String>>,
    /// The store's latest version of a cell (tx, receipt hash), prefetched per chunk.
    latest_of: HashMap<String, Option<(String, String)>>,
    /// The store's receipt, prefetched for the latest receipts a skip check reads.
    receipt_of: HashMap<String, Option<Vec<u8>>>,
}

impl Reads {
    fn binding<S: Store + ?Sized>(&mut self, store: &S, name: &str, at: &str) -> Result<Option<Binding>, String> {
        let key = (name.to_string(), at.to_string());
        if let Some(r) = self.bindings.get(&key) {
            return r.clone();
        }
        let r = store.resolve_binding(name, at).map_err(|e| e.to_string());
        self.bindings.insert(key, r.clone());
        r
    }

    fn descriptor<S: Store + ?Sized>(&mut self, store: &S, hash: &str) -> Option<Vec<u8>> {
        if let Some(b) = self.descriptors.get(hash) {
            return b.clone();
        }
        let b = store.descriptor(hash).map(|b| b.into_owned());
        self.descriptors.insert(hash.to_string(), b.clone());
        b
    }

    fn namespace<S: Store + ?Sized>(&mut self, store: &S, host: &str) -> Result<Option<Namespace>, String> {
        if let Some(r) = self.namespaces.get(host) {
            return r.clone();
        }
        let r = store.namespace(host).map_err(|e| e.to_string());
        self.namespaces.insert(host.to_string(), r.clone());
        r
    }

    /// The store's latest version of `id`: prefetched, else asked now.
    fn latest<S: Store + ?Sized>(&mut self, store: &S, id: &str) -> Result<Option<(String, String)>, String> {
        if let Some(l) = self.latest_of.get(id) {
            return Ok(l.clone());
        }
        store.latest(id).map(|v| v.map(|v| (v.tx, v.receipt_hash))).map_err(|e| e.to_string())
    }

    /// The stored receipt `hash`, as `receipt` answers: prefetched, else asked now.
    fn receipt<S: Store + ?Sized>(&mut self, store: &S, hash: &str) -> Option<Vec<u8>> {
        if let Some(b) = self.receipt_of.get(hash) {
            return b.clone();
        }
        store.receipt(hash).map(|b| b.into_owned())
    }

    /// One `latest_many` for the ids not read yet.
    fn prefetch_latest<S: Store + ?Sized>(&mut self, store: &S, ids: &[&str]) {
        let mut seen = HashSet::new();
        let missing: Vec<&str> =
            ids.iter().copied().filter(|id| !self.latest_of.contains_key(*id) && seen.insert(*id)).collect();
        if missing.is_empty() {
            return;
        }
        if let Ok(found) = store.latest_many(&missing) {
            for (id, v) in missing.iter().zip(found) {
                self.latest_of.insert(id.to_string(), v.map(|v| (v.tx, v.receipt_hash)));
            }
        }
    }

    /// One `receipts_many` for the hashes not read yet.
    fn prefetch_receipts<S: Store + ?Sized>(&mut self, store: &S, hashes: &[&str]) {
        let mut seen = HashSet::new();
        let missing: Vec<&str> =
            hashes.iter().copied().filter(|h| !self.receipt_of.contains_key(*h) && seen.insert(*h)).collect();
        if missing.is_empty() {
            return;
        }
        if let Ok(found) = store.receipts_many(&missing) {
            for (h, b) in missing.iter().zip(found) {
                self.receipt_of.insert(h.to_string(), b);
            }
        }
    }
}

/// `f` over `items` on up to `available_parallelism()` threads; the results in item order.
fn parallel_owned<T: Send, R: Send>(items: Vec<T>, f: impl Fn(T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(items.len());
    if threads <= 1 {
        return items.into_iter().map(f).collect();
    }
    let size = items.len().div_ceil(threads);
    let mut parts: Vec<Vec<T>> = Vec::with_capacity(threads);
    let mut rest = items.into_iter();
    loop {
        let part: Vec<T> = rest.by_ref().take(size).collect();
        if part.is_empty() {
            break;
        }
        parts.push(part);
    }
    std::thread::scope(|s| {
        let handles: Vec<_> = parts
            .into_iter()
            .map(|part| {
                let f = &f;
                s.spawn(move || part.into_iter().map(f).collect::<Vec<R>>())
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e))).collect()
    })
}

/// The function hash `name` means at the end of time: the file's own over the store's.
fn current_function<S: Store + ?Sized>(store: &S, sim: &Simulation, name: &str) -> Result<Option<String>, String> {
    if let Some((_, h)) = Simulation::latest(&sim.functions, name, END) {
        return Ok(Some(h));
    }
    store.resolve_name(name, END).map_err(|e| e.to_string())
}

/// The class hash `name` means at the end of time: the file's own over the store's.
fn current_class<S: Store + ?Sized>(store: &S, sim: &Simulation, name: &str) -> Result<Option<String>, String> {
    if let Some((_, h)) = Simulation::latest(&sim.classes, name, END) {
        return Ok(Some(h));
    }
    store.resolve_class(name, END).map_err(|e| e.to_string())
}

/// The function hash `name` means as of `at`, counting the file's earlier binds.
fn function_at<S: Store + ?Sized>(
    reads: &mut Reads,
    store: &S,
    sim: &Simulation,
    name: &str,
    at: &str,
) -> Result<Option<String>, String> {
    let stored = reads.binding(store, name, at)?;
    let simmed = Simulation::latest(&sim.functions, name, at);
    Ok(match (stored, simmed) {
        (Some(b), Some((t, h))) if t >= b.tx => Some(h),
        (Some(b), _) => Some(b.fn_hash),
        (None, Some((_, h))) => Some(h),
        (None, None) => None,
    })
}

/// The latest binding tx of `name` (a class's, or a function's), the store's or the file's.
fn latest_bind<S: Store + ?Sized>(
    store: &S,
    sim: &Simulation,
    name: &str,
    class: bool,
) -> Result<Option<String>, String> {
    let stored: Option<String> = if class {
        store.resolve_class_binding(name, END).map_err(|e| e.to_string())?.map(|b| b.tx)
    } else {
        store.resolve_binding(name, END).map_err(|e| e.to_string())?.map(|b| b.tx)
    };
    let binds = if class { &sim.classes } else { &sim.functions };
    let planned = binds.get(name).and_then(|v| v.iter().map(|(t, _)| t.clone()).max());
    Ok(match (stored, planned) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    })
}

/// Whether `name` is bound as the other kind: a function for a class, a class for a function.
fn bound_as_other<S: Store + ?Sized>(store: &S, sim: &Simulation, name: &str, class: bool) -> Result<bool, String> {
    if class {
        Ok(store.resolve_binding(name, END).map_err(|e| e.to_string())?.is_some() || sim.functions.contains_key(name))
    } else {
        Ok(store.resolve_class_binding(name, END).map_err(|e| e.to_string())?.is_some()
            || sim.classes.contains_key(name))
    }
}

/// The latest version of a cell, the store's or the file's: its tx and receipt hash.
fn cell_latest<S: Store + ?Sized>(
    reads: &mut Reads,
    store: &S,
    sim: &Simulation,
    id: &str,
) -> Result<Option<(String, String)>, String> {
    let stored = reads.latest(store, id)?;
    let planned = sim.versions.get(id).and_then(|v| v.last().cloned());
    Ok(match (stored, planned) {
        (Some(a), Some(b)) => Some(if b.0 > a.0 { b } else { a }),
        (a, b) => a.or(b),
    })
}

/// The receipt hash of a cell's version as of `at`, the store's or the file's.
fn cell_version_asof<S: Store + ?Sized>(
    store: &S,
    sim: &Simulation,
    id: &str,
    at: &str,
) -> Result<Option<String>, String> {
    let stored = store.version_asof(id, at).map_err(|e| e.to_string())?.map(|v| (v.tx, v.receipt_hash));
    let planned = sim.versions.get(id).and_then(|v| v.iter().rev().find(|(t, _)| t.as_str() <= at).cloned());
    let latest = match (stored, planned) {
        (Some(a), Some(b)) => Some(if b.0 > a.0 { b } else { a }),
        (a, b) => a.or(b),
    };
    Ok(latest.map(|x| x.1))
}

/// The descriptor bytes of a function hash: the file's own, else the store's (memoized).
fn function_bytes_of<S: Store + ?Sized>(
    reads: &mut Reads,
    store: &S,
    sim: &Simulation,
    hash: &str,
) -> Result<Option<Vec<u8>>, String> {
    if let Some(b) = sim.descriptors.get(hash) {
        return Ok(Some(b.clone()));
    }
    Ok(reads.descriptor(store, hash))
}

/// Whether a namespace is suspended (the store refuses binds and class puts under it).
fn suspended<S: Store + ?Sized>(reads: &mut Reads, store: &S, host: &str, line: usize) -> Result<(), PublishError> {
    match reads.namespace(store, host) {
        Ok(Some(n)) if n.suspended_at.is_some() => {
            Err(PublishError::at(line, format!("{host} is suspended: no new bindings or classes under it")))
        }
        Ok(_) => Ok(()),
        Err(e) => Err(PublishError::at(line, e)),
    }
}

/// A binding a run would make: the line that makes it and its name.
struct Bind {
    line: usize,
    name: String,
}

/// What a run writes, for the backfill log: the namespaces and starts of its binds.
#[derive(Default)]
struct Written {
    namespaces: BTreeSet<String>,
    starts: BTreeSet<String>,
}

impl Written {
    fn bound(&mut self, name: &str, start: &str) {
        self.namespaces.insert(namespace_of(name).to_string());
        self.starts.insert(start.to_string());
    }
}

/// The per-run values both passes use.
struct Ctx<'a> {
    host: &'a str,
    owned: &'a [String],
    /// The start of the mesh's key.
    key_from: &'a str,
    /// The clock's now, read once for the run.
    now: &'a str,
    /// The directory of the file (blobs are relative to it).
    base: &'a Path,
}

/// One write the second pass makes, planned (and checked) by the first.
enum Op {
    /// A descriptor line: put; counted when newly put.
    Descriptor { hash: String, bytes: Vec<u8>, counted: bool },
    /// A function (stored or formula): put its descriptor, and bind the name when `bind`.
    Function { name: String, start: String, hash: String, bytes: Vec<u8>, bind: bool },
    /// A class: put it, and bind it (its methods with it, in the store) when `bind`.
    Class { name: String, start: String, hash: String, bytes: Vec<u8>, bind: bool, methods: Vec<String> },
    /// A cell: its blob, its receipt, and the version when `version`.
    Cell { tx: String, id: String, receipt: Vec<u8>, blob: Option<Vec<u8>>, version: bool },
    /// A catalogue line (R5): the entry, and its dictionaries (`publisher`, signed by the run's key).
    Catalog { entry: datumesh_store::CatalogEntry, dictionaries: Vec<(String, Vec<datumesh_store::KeyValue>)> },
}

/// The lines of a file, numbered, without the blank ones.
fn read_lines(path: &Path) -> Result<Vec<(usize, String)>, PublishError> {
    let file = std::fs::File::open(path).map_err(|e| PublishError::at(0, format!("{}: {e}", path.display())))?;
    let mut out = Vec::new();
    for (i, line) in std::io::BufReader::new(file).lines().enumerate() {
        let n = i + 1;
        let line = line.map_err(|e| PublishError::at(n, e.to_string()))?;
        if !line.trim().is_empty() {
            out.push((n, line));
        }
    }
    Ok(out)
}

/// The dictionary values the file's cells have for the string arguments (CONTRACT-build2-part2-read
/// R3): each distinct (function, key, value) that no dictionary has yet is stored with an empty label
/// (the value stands for itself). A dictionary's label is never replaced by a cell's.
fn index_cell_values<S: Store + ?Sized>(store: &S, ops: &[Op]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for op in ops {
        if let Op::Cell { receipt, .. } = op {
            if let Ok(r) = datumesh_core::Receipt::from_json(receipt) {
                cell_string_values(&r, &mut seen);
            }
        }
    }
    store_cell_values(store, seen)
}

/// The (function, key, value) of each string argument of a receipt's cell, into `out`.
pub(crate) fn cell_string_values(r: &datumesh_core::Receipt, out: &mut BTreeSet<(String, String, String)>) {
    let cell = r.cell();
    for (key, arg) in &cell.args {
        if let datumesh_core::Arg::Value(datumesh_core::Scalar::String(v)) = arg {
            if !v.is_empty() && v.len() <= 1024 {
                out.insert((cell.function_name(), key.clone(), v.clone()));
            }
        }
    }
}

/// Stores the values of [`cell_string_values`] that no dictionary has yet (R3).
pub(crate) fn store_cell_values<S: Store + ?Sized>(
    store: &S,
    seen: BTreeSet<(String, String, String)>,
) -> Result<(), String> {
    for (owner, key, value) in seen {
        if crate::find::has_value(store, &owner, &key, &value) {
            continue;
        }
        let kv = datumesh_store::KeyValue { value, label: String::new(), aliases: Vec::new() };
        store.upsert_key_values(&owner, &key, std::slice::from_ref(&kv)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Whether a line is a cell line: none of the other kinds (the dispatch order of `plan_line`).
fn is_cell_line(v: &Value) -> bool {
    v.get("class").and_then(Value::as_str).is_none()
        && v.get("descriptor").is_none()
        && v.get("function").and_then(Value::as_str).is_none()
        && v.get("formula").and_then(Value::as_str).is_none()
        && v.get("cell").is_some()
}

/// A cell line prepared for signing: everything its receipt takes, checked up to there.
struct Pending {
    line: usize,
    ns: String,
    cell: Cell,
    value: Scalar,
    valid: Args,
    tx: String,
    asof: String,
    issued: String,
    has_tx: bool,
    licence: String,
    summary: String,
    blob: Option<Vec<u8>>,
    /// The `source` of a relayed value (spec-0.6.0, 8.1), as the line gives it.
    source: Option<Source>,
}

/// The `source` object of a cell line (spec-0.6.0, C2): exactly `name`, `url` and `retrieved`, with
/// `sha256` and `takeover` optional, all strings. Its rules (a DNS name, an instant, an `https://`
/// takeover, a publisher that does not relay itself) are the receipt's, checked when it is signed.
fn source_of_line(v: &Value) -> Result<Source, String> {
    let Value::Object(m) = v else { return Err("source is an object".into()) };
    for (k, x) in m {
        if !matches!(k.as_str(), "name" | "url" | "retrieved" | "sha256" | "takeover") {
            return Err(format!("source has no member {k}"));
        }
        if !matches!(x, Value::String(_)) {
            return Err(format!("source.{k} is a string"));
        }
    }
    let text = |k: &str| -> Result<&str, String> {
        v.get(k).and_then(Value::as_str).ok_or_else(|| format!("source has no {k}"))
    };
    let mut out = Source::new(text("name")?, text("url")?, text("retrieved")?);
    if let Some(h) = v.get("sha256").and_then(Value::as_str) {
        out = out.with_sha256(h);
    }
    if let Some(t) = v.get("takeover").and_then(Value::as_str) {
        out = out.with_takeover(t);
    }
    Ok(out)
}

impl Pending {
    /// Signs the receipt (deterministic) and takes its hash and cell id. Runs on any thread.
    fn sign(self, key: &SigningKey, host: &str) -> Result<Signed, PublishError> {
        let line = self.line;
        let err = move |d: String| PublishError::at(line, d);
        let b = ReceiptBuilder::new()
            .cell(self.cell.clone())
            .value(self.value)
            .valid(self.valid)
            .asof(self.asof.as_str())
            .tx(self.tx.as_str())
            .issued(self.issued.as_str())
            .publisher(self.ns.as_str())
            .signer_host(host)
            .licence(self.licence.as_str())
            .summary(self.summary.as_str());
        let b = match self.source {
            Some(s) => b.source(s),
            None => b,
        };
        let r = b.sign(key).map_err(|e| err(e.to_string()))?;
        let receipt = receipt_json(&r);
        let hash = Receipt::from_json(&receipt).map_err(|e| err(e.to_string()))?.hash().to_string();
        let id = self.cell.id();
        Ok(Signed { line, has_tx: self.has_tx, tx: self.tx, id, hash, receipt, blob: self.blob })
    }
}

/// A cell line signed: what its checks and its write take.
struct Signed {
    line: usize,
    has_tx: bool,
    tx: String,
    id: String,
    hash: String,
    receipt: Vec<u8>,
    blob: Option<Vec<u8>>,
}

/// Pass one: plans the writes of the lines and checks each as the store will (CONTRACT-1.7a H2).
struct Planner<'a, S: Store + ?Sized> {
    store: &'a S,
    key: &'a SigningKey,
    ctx: Ctx<'a>,
    backfill: bool,
    sim: Simulation,
    reads: Reads,
    ops: Vec<Op>,
    binds: Vec<Bind>,
    /// The test lines of the file, with their lines (Build 2 Part 3).
    tests: Vec<(usize, TestSpec)>,
    /// The computed functions the file defines: (line, name, function hash).
    formulas: Vec<(usize, String, String)>,
}

impl<S: Store + ?Sized> Planner<'_, S> {
    fn callers(&self) -> Callers<'_, S> {
        Callers { store: self.store, sim: Some(&self.sim) }
    }

    /// Plans the lines in file order. Cell lines are planned in chunks of up to `CHUNK`
    /// consecutive ones; any other line ends a chunk (CONTRACT-perf H1).
    fn run(&mut self, lines: &[(usize, String)]) -> Result<(), PublishError> {
        let mut cells: Vec<(usize, Value)> = Vec::new();
        for (n, text) in lines {
            let n = *n;
            match json::parse(text.as_bytes()).map_err(|e| PublishError::at(n, e.to_string())) {
                Ok(v) if is_cell_line(&v) => {
                    cells.push((n, v));
                    if cells.len() == CHUNK {
                        self.plan_cells(std::mem::take(&mut cells))?;
                    }
                }
                other => {
                    // The cells before this line are checked first: a failure of either is the
                    // file's first.
                    self.plan_cells(std::mem::take(&mut cells))?;
                    self.plan_line(n, &other?)?;
                }
            }
        }
        self.plan_cells(cells)
    }

    /// One line that is not a cell (the dispatch of the lines before cells, as it was).
    fn plan_line(&mut self, n: usize, v: &Value) -> Result<(), PublishError> {
        let err = |d: String| PublishError::at(n, d);
        if let Some(name) = v.get("class").and_then(Value::as_str) {
            self.class_line(n, name, v)?;
        } else if let Some(d) = v.get("descriptor") {
            self.descriptor_line(n, d)?;
        } else if let Some(name) = v.get("function").and_then(Value::as_str) {
            let sig = v.get("signature").ok_or_else(|| err("a function without a signature".into()))?;
            let start = bound_of(v, n, self.ctx.now)?;
            let fd = FunctionDescriptor::from_value(&stored_value(sig)).map_err(|e| err(e.to_string()))?;
            self.function(n, name, start, fd.hash(), fd.canonical())?;
        } else if v.get("catalog").is_some() {
            self.catalog_line(n, v)?;
        } else if let Some(name) = v.get("formula").and_then(Value::as_str) {
            let source =
                v.get("source").and_then(Value::as_str).ok_or_else(|| err("a formula without source".into()))?;
            let sig = v.get("signature").ok_or_else(|| err("a formula without a signature".into()))?;
            let start = bound_of(v, n, self.ctx.now)?;
            let def = datumesh_formula::definition::define_function(
                name,
                source,
                sig,
                &Default::default(),
                &self.callers(),
                &start,
            )
            .map_err(|e| err(format!("define {name}: {e}")))?;
            let hash = def.fn_hash.clone();
            self.function(n, name, start, def.fn_hash, def.bytes)?;
            self.formulas.push((n, name.to_string(), hash));
        } else if v.get("test").is_some() {
            let spec = testrun::parse_test(v).map_err(err)?;
            self.tests.push((n, spec));
        } else {
            return Err(err("neither a function nor a cell".into()));
        }
        Ok(())
    }

    /// `{"catalog": <name>, "kind", "label", "description", "args", "examples", "dictionary"}` (R5):
    /// the publisher's text for a function or class bound in this file or the store, under a host
    /// this mesh owns. Checked here, so a refused line writes nothing.
    fn catalog_line(&mut self, n: usize, v: &Value) -> Result<(), PublishError> {
        let err = |d: String| PublishError::at(n, d);
        let d = crate::catalog::parse(v, "catalog").map_err(err)?;
        let host = host_of(&d.name);
        if host != self.ctx.host && !self.ctx.owned.iter().any(|o| o == host) {
            return Err(err(format!("{} is not under a host this mesh owns", d.name)));
        }
        let hash = match d.kind {
            datumesh_store::CatalogKind::Class => current_class(self.store, &self.sim, &d.name).map_err(err)?,
            _ => current_function(self.store, &self.sim, &d.name).map_err(err)?,
        };
        if hash.is_none() {
            return Err(err(format!("{} is bound nowhere", d.name)));
        }
        let entry = d.entry(hash, datumesh_store::CatalogSource::Publisher, Some(self.key.key_id().to_string()));
        self.ops.push(Op::Catalog { entry, dictionaries: d.dictionary });
        Ok(())
    }

    /// `{"descriptor": ...}`: a stored function's descriptor, put and bound to nothing.
    fn descriptor_line(&mut self, n: usize, d: &Value) -> Result<(), PublishError> {
        let fd = FunctionDescriptor::from_value(d).map_err(|e| PublishError::at(n, e.to_string()))?;
        if fd.lang != "stored" || !fd.body.is_empty() || !fd.inputs.is_empty() {
            return Err(PublishError::at(
                n,
                "a descriptor line takes a stored function only: code.lang stored, code.body empty, no inputs",
            ));
        }
        let (hash, bytes) = (fd.hash(), fd.canonical());
        self.sim.descriptors.insert(hash.clone(), bytes.clone());
        self.ops.push(Op::Descriptor { hash, bytes, counted: true });
        Ok(())
    }

    /// A function's descriptor (stored or formula): put, and bound when its name does not mean it.
    fn function(
        &mut self,
        n: usize,
        name: &str,
        start: String,
        hash: String,
        bytes: Vec<u8>,
    ) -> Result<(), PublishError> {
        self.sim.descriptors.insert(hash.clone(), bytes.clone());
        let current = current_function(self.store, &self.sim, name).map_err(|e| PublishError::at(n, e))?;
        let bind = current.as_deref() != Some(hash.as_str());
        if bind {
            self.check_bind(n, name, &start, false)?;
            self.sim.functions.entry(name.to_string()).or_default().push((start.clone(), hash.clone()));
        }
        self.ops.push(Op::Function { name: name.to_string(), start, hash, bytes, bind });
        Ok(())
    }

    fn class_line(&mut self, n: usize, name: &str, v: &Value) -> Result<(), PublishError> {
        let err = |d: String| PublishError::at(n, d);
        let descriptor = v.get("descriptor").ok_or_else(|| err("a class without descriptor".into()))?;
        let start = bound_of(v, n, self.ctx.now)?;
        let def = datumesh_formula::definition::define_class(
            name,
            &descriptor.to_jcs(),
            definer(name, self.ctx.host, self.ctx.owned),
            &self.callers(),
            &start,
        )
        .map_err(|e| err(format!("define {name}: {e}")))?;
        // put_class refuses under a suspended steward namespace, whether or not the name binds. A
        // namespace row is a host's (9.9), so the steward namespace's host is asked: a steward with
        // a path (datumesh.com/scripture) is not a host and Store::namespace refuses it.
        suspended(&mut self.reads, self.store, host_of(&def.descriptor.steward.namespace), n)?;
        let hash = def.class_hash.clone();
        let methods: Vec<(String, String)> =
            def.descriptor.methods.iter().map(|(m, fh)| (format!("{name}/{m}"), fh.clone())).collect();
        let bind = current_class(self.store, &self.sim, name).map_err(err)?.as_deref() != Some(hash.as_str());
        if bind {
            self.check_bind(n, name, &start, true)?;
            // bind_class binds every method in the same transaction (the store's rule).
            for (mname, _) in &methods {
                self.check_bind(n, mname, &start, false)?;
            }
            self.sim.classes.entry(name.to_string()).or_default().push((start.clone(), hash.clone()));
            for (mname, fh) in &methods {
                self.sim.functions.entry(mname.clone()).or_default().push((start.clone(), fh.clone()));
            }
        }
        self.sim.class_bytes.insert(hash.clone(), def.bytes.clone());
        self.ops.push(Op::Class {
            name: name.to_string(),
            start,
            hash,
            bytes: def.bytes,
            bind,
            methods: methods.into_iter().map(|(m, _)| m).collect(),
        });
        Ok(())
    }

    /// One bind a line would make: rule A, the other kind, the namespace, and the latest binding.
    fn check_bind(&mut self, line: usize, name: &str, start: &str, class: bool) -> Result<(), PublishError> {
        if !self.backfill && start < self.ctx.now {
            return Err(PublishError::typed(
                line,
                PublishErrorKind::StartBeforeNow { name: name.into(), start: start.into(), now: self.ctx.now.into() },
            ));
        }
        let err = |d: String| PublishError::at(line, d);
        if bound_as_other(self.store, &self.sim, name, class).map_err(err)? {
            let kind = if class { "function" } else { "class" };
            return Err(err(format!("{name} is bound as a {kind}: one namespace of names")));
        }
        suspended(&mut self.reads, self.store, host_of(name), line)?;
        if let Some(latest) = latest_bind(self.store, &self.sim, name, class).map_err(err)? {
            if start <= latest.as_str() {
                return Err(err(format!("{name} is bound from {latest}: a bind at {start} is not later")));
            }
        }
        self.binds.push(Bind { line, name: name.into() });
        Ok(())
    }

    /// Plans the cell lines of a chunk (CONTRACT-perf H1): each is prepared in file order up to its
    /// receipt; the receipts are signed on threads; the store is read once for the chunk; then each
    /// is checked in file order. The first failure is the chunk's, and a line that fails before
    /// signing is reported after the lines before it are checked.
    fn plan_cells(&mut self, lines: Vec<(usize, Value)>) -> Result<(), PublishError> {
        if lines.is_empty() {
            return Ok(());
        }
        let mut pending = Vec::with_capacity(lines.len());
        let mut failed = None;
        for (n, v) in &lines {
            match self.prepare_cell(*n, v) {
                Ok(p) => pending.push(p),
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        let (key, host) = (self.key, self.ctx.host);
        let signed = parallel_owned(pending, |p| p.sign(key, host));
        let ids: Vec<&str> = signed.iter().flatten().map(|s| s.id.as_str()).collect();
        self.reads.prefetch_latest(self.store, &ids);
        let hashes: Vec<String> = signed
            .iter()
            .flatten()
            .filter(|s| !s.has_tx)
            .filter_map(|s| self.reads.latest_of.get(&s.id).cloned().flatten())
            .map(|(_, h)| h)
            .collect();
        let hashes: Vec<&str> = hashes.iter().map(String::as_str).collect();
        self.reads.prefetch_receipts(self.store, &hashes);
        for s in signed {
            self.check_cell(s?)?;
        }
        match failed {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// A cell line up to its receipt, as the checks of a line take them in order: the fields, the
    /// instants, the function as of `tx`, the descriptor, the arguments and the value (and its blob).
    fn prepare_cell(&mut self, n: usize, v: &Value) -> Result<Pending, PublishError> {
        let err = |d: String| PublishError::at(n, d);
        let cell_v = v.get("cell").ok_or_else(|| err("a cell without fn".into()))?;
        let name = cell_v.get("fn").and_then(Value::as_str).ok_or_else(|| err("a cell without fn".into()))?;
        let (ns, f) = name.rsplit_once('/').ok_or_else(|| err(format!("{name}: not a function name")))?;
        let field = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let required = |k: &str| field(k).ok_or_else(|| err(format!("no {k}")));
        let (licence, summary) = (required("licence")?, required("summary")?);
        // tx and asof default to the run's clock now, each on its own (H4).
        let asof = field("asof").unwrap_or_else(|| self.ctx.now.to_string());
        let has_tx = field("tx").is_some();
        let tx = field("tx").unwrap_or_else(|| self.ctx.now.to_string());
        if tx > asof {
            return Err(err(format!("tx {tx} is after asof {asof} (SPEC 8)")));
        }
        let issued = field("issued").unwrap_or_else(|| std::cmp::max(asof.clone(), self.ctx.key_from.to_string()));
        if asof > issued {
            return Err(err(format!("asof {asof} is after issued {issued} (SPEC 8)")));
        }
        let fn_hash = function_at(&mut self.reads, self.store, &self.sim, name, &tx)
            .map_err(err)?
            .ok_or_else(|| err(format!("{name} is not bound as of {tx}")))?;
        let d = function_bytes_of(&mut self.reads, self.store, &self.sim, &fn_hash)
            .map_err(err)?
            .ok_or_else(|| err(format!("no descriptor {fn_hash}")))?;
        let d = FunctionDescriptor::from_json(&d).map_err(|e| err(e.to_string()))?;
        let mut args = Args::new();
        for (k, a) in cell_v.get("args").and_then(Value::as_object).ok_or_else(|| err("a cell without args".into()))? {
            args.insert(k.clone(), Arg::Value(Scalar::from_json(&a.to_jcs()).map_err(|e| err(format!("{k}: {e}")))?));
        }
        let valid: Args = args
            .iter()
            .filter(|(k, _)| d.signature.args.get(*k).is_some_and(|x| x.valid))
            .map(|(k, a)| (k.clone(), a.clone()))
            .collect();
        let source = match v.get("source") {
            Some(sv) => Some(source_of_line(sv).map_err(err)?),
            None => None,
        };
        let value_v = v.get("value").ok_or_else(|| err("a cell without value".into()))?;
        let (value, blob) = match value_v.get("blob").and_then(Value::as_str) {
            Some(file) => {
                let media = value_v.get("media_type").and_then(Value::as_str).unwrap_or("application/octet-stream");
                let bytes = std::fs::read(self.ctx.base.join(file)).map_err(|e| err(format!("{file}: {e}")))?;
                let sha = sha256_hex(&bytes);
                let scalar = Scalar::from_lexical(Type::Blob, &format!("{sha}:{}:{media}", bytes.len()))
                    .map_err(|e| err(e.to_string()))?;
                (scalar, Some(bytes))
            }
            None => (Scalar::from_json(&value_v.to_jcs()).map_err(|e| err(e.to_string()))?, None),
        };
        let cell = Cell { ns: ns.to_string(), fn_name: f.to_string(), fn_hash, args };
        Ok(Pending {
            line: n,
            ns: ns.to_string(),
            cell,
            value,
            valid,
            tx,
            asof,
            issued,
            has_tx,
            licence,
            summary,
            blob,
            source,
        })
    }

    /// The checks of a signed cell line, in file order: the skip rule of a line without `tx`, then
    /// the version rule. Its write is planned.
    fn check_cell(&mut self, s: Signed) -> Result<(), PublishError> {
        let line = s.line;
        let err = |d: String| PublishError::at(line, d);
        // A line without tx takes the run's clock, so a re-run at a later clock would sign the same
        // fact again as a new version. Re-running a file changes nothing: when the cell's latest
        // version is the store's and states the same thing (type, value, valid, licence, summary),
        // the line is skipped. A changed value is a new version, a correction.
        if !s.has_tx {
            if let Some((_, latest)) = cell_latest(&mut self.reads, self.store, &self.sim, &s.id).map_err(err)? {
                if let Some(old) = self.reads.receipt(self.store, &latest) {
                    let old = json::parse(&old).map_err(|e| err(e.to_string()))?;
                    let new = json::parse(&s.receipt).map_err(|e| err(e.to_string()))?;
                    if ["type", "value", "valid", "licence", "summary"].iter().all(|k| old.get(k) == new.get(k)) {
                        return Ok(());
                    }
                }
            }
        }
        let version = match cell_latest(&mut self.reads, self.store, &self.sim, &s.id).map_err(err)? {
            Some((latest_tx, _)) if latest_tx.as_str() >= s.tx.as_str() => {
                if cell_version_asof(self.store, &self.sim, &s.id, &s.tx).map_err(err)?.as_deref()
                    != Some(s.hash.as_str())
                {
                    return Err(err(format!("{} has a version at or after {} that is not this receipt", s.id, s.tx)));
                }
                false
            }
            _ => {
                self.sim.versions.entry(s.id.clone()).or_default().push((s.tx.clone(), s.hash.clone()));
                true
            }
        };
        self.ops.push(Op::Cell { tx: s.tx, id: s.id, receipt: s.receipt, blob: s.blob, version });
        Ok(())
    }
}

/// The stored descriptor of a function line's signature.
fn stored_value(signature: &Value) -> Value {
    Value::Object(vec![
        ("code".into(), Value::Object(vec![("body".into(), s("")), ("lang".into(), s("stored"))])),
        ("inputs".into(), Value::Array(vec![])),
        ("signature".into(), signature.clone()),
    ])
}

/// The backfill's checks (since 0.3.0 with store 0.4.2): no name the file binds has a binding, and
/// no namespace of one has any binding (`has_bindings_under`).
fn backfill_check<S: Store + ?Sized>(store: &S, binds: &[Bind]) -> Result<(), PublishError> {
    for b in binds {
        let refuse = |why: String| PublishError::typed(b.line, PublishErrorKind::Backfill(why));
        if store.resolve_binding(&b.name, END).map_err(|e| PublishError::at(b.line, e.to_string()))?.is_some() {
            return Err(refuse(format!("{} has a binding", b.name)));
        }
        let ns = namespace_of(&b.name);
        if store.has_bindings_under(ns).map_err(|e| PublishError::at(b.line, e.to_string()))? {
            return Err(refuse(format!("{ns} has bindings")));
        }
    }
    Ok(())
}

/// Appends a run's line to the backfill log (created with mode 0600).
fn append_log(log: &Path, written: &Written) -> Result<(), String> {
    let line = Value::Object(vec![
        ("at".into(), s(&SystemClock.now())),
        ("namespaces".into(), Value::Array(written.namespaces.iter().map(|n| s(n)).collect())),
        ("starts".into(), Value::Array(written.starts.iter().map(|t| s(t)).collect())),
    ])
    .to_jcs();
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(log).map_err(|e| format!("{}: {e}", log.display()))?;
    let mut bytes = line;
    bytes.push(b'\n');
    f.write_all(&bytes).map_err(|e| format!("{}: {e}", log.display()))
}

/// Pass two: the writes the first pass planned, in order. Nothing here can refuse a planned
/// write, so a refused file has written nothing. Consecutive cell writes go in `put_cells` calls
/// of up to `CHUNK` on `lanes` lanes, their blobs put first (CONTRACT-perf H2, H5).
fn write<S: Store + ?Sized>(
    store: &S,
    ops: &[Op],
    cells: impl Fn(&S, &[Op], &mut Published) -> Result<(), String>,
) -> Result<(Published, Written), PublishError> {
    let mut out = Published::default();
    let mut written = Written::default();
    let err = |d: String| PublishError::at(0, d);
    let mut i = 0;
    while i < ops.len() {
        match &ops[i] {
            Op::Descriptor { hash, bytes, counted } => {
                let existed = store.descriptor(hash).is_some();
                store.put_descriptor(bytes).map_err(|e| err(e.to_string()))?;
                if *counted && !existed {
                    out.descriptors += 1;
                }
            }
            Op::Function { name, start, hash, bytes, bind } => {
                store.put_descriptor(bytes).map_err(|e| err(e.to_string()))?;
                if *bind {
                    store.bind_name(name, hash, start).map_err(|e| err(format!("bind {name}: {e}")))?;
                    out.functions += 1;
                    written.bound(name, start);
                }
            }
            Op::Class { name, start, hash, bytes, bind, methods } => {
                store.put_class(bytes).map_err(|e| err(e.to_string()))?;
                if *bind {
                    store.bind_class(name, hash, start).map_err(|e| err(format!("bind {name}: {e}")))?;
                    out.classes += 1;
                    written.bound(name, start);
                    for m in methods {
                        written.bound(m, start);
                    }
                }
            }
            Op::Catalog { entry, dictionaries } => {
                crate::catalog::store_publisher(store, entry, dictionaries).map_err(err)?;
            }
            Op::Cell { .. } => {
                let mut j = i;
                while j < ops.len() && j - i < CHUNK && matches!(ops[j], Op::Cell { .. }) {
                    j += 1;
                }
                cells(store, &ops[i..j], &mut out).map_err(err)?;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    Ok((out, written))
}

/// A run of cell writes (up to `CHUNK`) on one lane: their blobs first, then the receipts and
/// versions in file order in `put_cells` calls of up to `CHUNK` (CONTRACT-perf H2). No `Sync`
/// bound: this is what [`publish_stored_with`] runs.
fn write_cells_one_lane<S: Store + ?Sized>(store: &S, group: &[Op], out: &mut Published) -> Result<(), String> {
    let items = put_blobs_and_items(store, group, out)?;
    let failed = AtomicBool::new(false);
    let ks: Vec<usize> = (0..items.len()).collect();
    if let Some((_, detail)) = put_lane(store, &items, &ks, &failed) {
        return Err(detail);
    }
    count_cells(group, &items, out);
    Ok(())
}

/// A run of cell writes (up to `CHUNK`): their blobs first, then their receipts and versions on
/// `lanes` lanes (CONTRACT-perf H5). A cell's id picks its lane (its first hex digit, mod `lanes`),
/// so all writes of one cell share a lane, in file order. A lane's `put_cells` calls take up to
/// `CHUNK` of its items. When one fails, no lane starts another call, and the error returned is
/// that of the earliest failing item in file order. With one lane, as [`write_cells_one_lane`].
fn write_cells<S: Store + Sync + ?Sized>(
    store: &S,
    group: &[Op],
    lanes: usize,
    out: &mut Published,
) -> Result<(), String> {
    if lanes <= 1 {
        return write_cells_one_lane(store, group, out);
    }
    let items = put_blobs_and_items(store, group, out)?;
    let mut by_lane: Vec<Vec<usize>> = vec![Vec::new(); lanes];
    for (k, op) in group.iter().enumerate() {
        if let Op::Cell { id, .. } = op {
            by_lane[lane_of(id, lanes)].push(k);
        }
    }
    let failed = AtomicBool::new(false);
    let failures: Vec<(usize, String)> = std::thread::scope(|scope| {
        let handles: Vec<_> = by_lane
            .iter()
            .map(|ks| {
                let (items, failed) = (&items, &failed);
                scope.spawn(move || put_lane(store, items, ks, failed))
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p))).collect()
    });
    if let Some((_, detail)) = failures.into_iter().min_by_key(|(k, _)| *k) {
        return Err(detail);
    }
    count_cells(group, &items, out);
    Ok(())
}

/// The blobs of a run, put in order (counted in `out`), and the run's cell writes.
fn put_blobs_and_items<'a, S: Store + ?Sized>(
    store: &S,
    group: &'a [Op],
    out: &mut Published,
) -> Result<Vec<CellItem<'a>>, String> {
    for op in group {
        if let Op::Cell { blob: Some(bytes), .. } = op {
            store.put_blob(bytes).map_err(|e| e.to_string())?;
            out.blobs += 1;
            out.blob_bytes += bytes.len() as u64;
        }
    }
    Ok(group
        .iter()
        .filter_map(|op| match op {
            Op::Cell { tx, id, receipt, version, .. } => {
                Some((receipt.as_slice(), if *version { Some((id.as_str(), tx.as_str())) } else { None }))
            }
            _ => None,
        })
        .collect())
}

/// Counts a written run of cells in `out`.
fn count_cells(group: &[Op], items: &[CellItem<'_>], out: &mut Published) {
    out.cells += group.len();
    out.receipts += group.len();
    out.versions += items.iter().filter(|(_, v)| v.is_some()).count();
}

/// The lane of a cell id: its first hex digit, mod `lanes` (CONTRACT-perf H5).
fn lane_of(id: &str, lanes: usize) -> usize {
    id.bytes().next().and_then(|b| (b as char).to_digit(16)).unwrap_or(0) as usize % lanes
}

/// A cell write of a run: its receipt's bytes and its version (cell id, tx), if any.
type CellItem<'a> = (&'a [u8], Option<(&'a str, &'a str)>);

/// One lane's writes: its items (indices into `items`, in file order) in `put_cells` calls of up to
/// `CHUNK`. Returns the index of the first item of the call that failed, and its error. No call
/// starts once `failed` is set.
fn put_lane<S: Store + ?Sized>(
    store: &S,
    items: &[CellItem<'_>],
    ks: &[usize],
    failed: &AtomicBool,
) -> Option<(usize, String)> {
    for call in ks.chunks(CHUNK) {
        if failed.load(Ordering::SeqCst) {
            return None;
        }
        let writes: Vec<CellWrite<'_>> = call.iter().map(|&k| CellWrite::new(items[k].0, items[k].1)).collect();
        if let Err(e) = store.put_cells(&writes) {
            failed.store(true, Ordering::SeqCst);
            return Some((call[0], e.to_string()));
        }
    }
    None
}

/// The receipt JSON of a signed receipt: its payload's members and `sig`.
pub fn receipt_json(r: &datumesh_core::Receipt) -> Vec<u8> {
    let mut members = vec![("sig".to_string(), Value::String(r.sig().to_string()))];
    if let Ok(Value::Object(m)) = json::parse(r.payload()) {
        members.extend(m);
    }
    Value::Object(members).to_jcs()
}

/// Publishes the file at `path` into `store` as the mesh `host` signing with `key` (see the
/// module documentation). The mesh's own namespaces are `host` alone: see [`publish_stored_in`].
/// The clock is the system clock, and there is no backfill. The cells are written on one lane;
/// see [`publish_stored_lanes`] for the parallel lanes.
pub fn publish_stored<S: Store + ?Sized>(
    store: &S,
    key: &SigningKey,
    host: &str,
    path: &Path,
) -> Result<Published, PublishError> {
    publish_stored_in(store, key, host, &[host.to_string()], path)
}

/// As [`publish_stored`], for a mesh that owns the namespaces (hosts) `owned` (`Config::namespaces`):
/// a class line is accepted for a class under one of them, and refused with `E_CLASS_OWNER` (in the
/// [`PublishError::detail`]) under any other. Since 0.2.2. The system clock, and no backfill.
pub fn publish_stored_in<S: Store + ?Sized>(
    store: &S,
    key: &SigningKey,
    host: &str,
    owned: &[String],
    path: &Path,
) -> Result<Published, PublishError> {
    publish_stored_with(store, key, host, owned, path, &PublishOptions::system())
}

/// As [`publish_stored_in`], with the run's clock and backfill (CONTRACT-1.8b H1, rule A). The
/// cells are written in file order on one lane, as in 0.3.2 (CONTRACT-perf H2).
///
/// Errors: a line refused by its checks, or a binding whose start is earlier than the clock
/// ([`PublishErrorKind::StartBeforeNow`]), or a backfill refused ([`PublishErrorKind::Backfill`]).
/// Nothing is written when the first pass refuses (CONTRACT-1.7a H2).
pub fn publish_stored_with<S: Store + ?Sized>(
    store: &S,
    key: &SigningKey,
    host: &str,
    owned: &[String],
    path: &Path,
    opts: &PublishOptions,
) -> Result<Published, PublishError> {
    publish_planned(store, key, host, owned, path, opts, false, |s, ops| {
        write(s, ops, |s, g, out| write_cells_one_lane(s, g, out))
    })
}

/// As [`publish_stored_with`], with the public rule (Build 2 Part 3, D8: datumesh-http always
/// enforces it; `publish-stored` calls this). A file that defines a computed function (formula)
/// is refused unless the tests of the function (the file's, and those already held for the same
/// function hash) exercise every derived edge case and every branch, and all of the file's tests
/// pass: [`PublishErrorKind::TestsMissing`] at the function's line, [`PublishErrorKind::TestsFailing`]
/// at the failing test's line. Stored functions are exempt. Nothing is written when refused.
pub fn publish_stored_checked<S: Store + ?Sized>(
    store: &S,
    key: &SigningKey,
    host: &str,
    owned: &[String],
    path: &Path,
    opts: &PublishOptions,
) -> Result<Published, PublishError> {
    publish_planned(store, key, host, owned, path, opts, true, |s, ops| {
        write(s, ops, |s, g, out| write_cells_one_lane(s, g, out))
    })
}

/// As [`publish_stored_with`], with the cells written on parallel lanes (CONTRACT-perf H5, H6):
/// `DATUMESH_PUBLISH_LANES` (a positive integer) if set, else `min(4, available_parallelism())`.
/// The same receipts, versions and rows as one lane; only the order of the inserts across
/// different cells differs. The variable is read once per run, before the file is read.
pub fn publish_stored_lanes<S: Store + Sync + ?Sized>(
    store: &S,
    key: &SigningKey,
    host: &str,
    owned: &[String],
    path: &Path,
    opts: &PublishOptions,
) -> Result<Published, PublishError> {
    let lanes = publish_lanes()?;
    publish_planned(store, key, host, owned, path, opts, false, |s, ops| {
        write(s, ops, |s, g, out| write_cells(s, g, lanes, out))
    })
}

/// The planned writes of a run as the tests see them (see [`Overlay`]).
fn overlay_of(ops: &[Op]) -> Result<Overlay, PublishError> {
    let mut overlay = Overlay::default();
    for op in ops {
        match op {
            Op::Function { name, start, hash, bytes, bind } => {
                overlay.descriptors.insert(hash.clone(), bytes.clone());
                if *bind {
                    overlay.functions.entry(name.clone()).or_default().push((start.clone(), hash.clone()));
                }
            }
            Op::Descriptor { hash, bytes, .. } => {
                overlay.descriptors.insert(hash.clone(), bytes.clone());
            }
            Op::Cell { receipt, .. } => {
                overlay.cell(receipt).map_err(|e| PublishError::at(0, format!("tests: {e}")))?;
            }
            Op::Class { .. } | Op::Catalog { .. } => {}
        }
    }
    Ok(overlay)
}

/// The public rule over a file (Build 2 Part 3, E5): runs the tests of each computed function the
/// file defines, over the store and the file's planned writes, and refuses the file when `checked`
/// and a test fails or the tests leave a derived case or a branch arm unexercised. Returns the
/// rows of the tests to hold (the file's and the held ones of those functions).
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_tests<S: Store + ?Sized>(
    store: &S,
    overlay: &Overlay,
    tests: &[(usize, TestSpec)],
    formulas: &[(usize, String, String)],
    host: &str,
    now: &str,
    checked: bool,
) -> Result<Vec<RowWrite>, PublishError> {
    if formulas.is_empty() {
        return Ok(Vec::new());
    }
    let mut file_tests: Vec<(usize, TestSpec, String)> = Vec::new();
    for (n, spec) in tests {
        let at = spec.asof.clone().unwrap_or_else(|| now.to_string());
        match testrun::binding_of(store, overlay, &spec.function, &at) {
            Ok(Some((hash, _))) => file_tests.push((*n, spec.clone(), hash)),
            Ok(None) => {
                return Err(PublishError::at(
                    *n,
                    format!("a test of {}: no function of that name is bound", spec.function),
                ))
            }
            Err(e) => return Err(PublishError::at(*n, e)),
        }
    }
    let mut rows = Vec::new();
    for (line, name, hash) in formulas {
        let own: Vec<(usize, TestSpec)> =
            file_tests.iter().filter(|t| &t.2 == hash).map(|t| (t.0, t.1.clone())).collect();
        let held = testrun::held_tests(store, hash).map_err(|e| PublishError::at(*line, e))?;
        if !checked && own.is_empty() && held.is_empty() {
            continue;
        }
        let mut specs: Vec<(Option<usize>, TestSpec)> = own.into_iter().map(|(n, s)| (Some(n), s)).collect();
        for (_, spec) in held {
            if !specs.iter().any(|(_, s)| s.case == spec.case) {
                specs.push((None, spec));
            }
        }
        let ran: Vec<Ran> = specs.iter().map(|(_, s)| run_test(store, overlay, now, s)).collect();
        let mut run = testrun::Run::new(store, overlay, now);
        let built = run.built(name, hash).map_err(|e| PublishError::at(*line, format!("{name}: {}", e.0)))?;
        let Some(compiled) = built.compiled.as_ref() else { continue };
        if checked {
            let mut failing: Vec<(usize, String)> = Vec::new();
            for ((n, spec), r) in specs.iter().zip(&ran) {
                if let Some(n) = n {
                    let (result, detail) = testrun::result_of(spec, r);
                    if result != "pass" {
                        failing.push((*n, format!("{name}: test {} {detail}", spec.case)));
                    }
                }
            }
            failing.sort_by_key(|f| f.0);
            if let Some((n, detail)) = failing.into_iter().next() {
                return Err(PublishError::typed(n, PublishErrorKind::TestsFailing(detail)));
            }
            let refs: Vec<&Ran> = ran.iter().collect();
            let cov = testrun::coverage_of(compiled, &built.function.signature, &refs);
            if !cov.uncovered.is_empty() || !cov.unexercised.is_empty() {
                let mut text = format!("{name} is missing tests:");
                if !cov.uncovered.is_empty() {
                    let cases: Vec<&str> = cov.uncovered.iter().map(|c| c.0.as_str()).collect();
                    text += &format!(" uncovered cases {};", cases.join(", "));
                }
                if !cov.unexercised.is_empty() {
                    text += &format!(" unexercised branches {}", cov.unexercised.join(", "));
                }
                return Err(PublishError::typed(
                    *line,
                    PublishErrorKind::TestsMissing(text.trim_end_matches(';').to_string()),
                ));
            }
        }
        let refs: Vec<&Ran> = ran.iter().collect();
        let cov = testrun::coverage_of(compiled, &built.function.signature, &refs);
        for ((_, spec), r) in specs.iter().zip(&ran) {
            let (result, detail) = testrun::result_of(spec, r);
            rows.push(RowWrite {
                fn_hash: hash.clone(),
                case: spec.case.clone(),
                jcs: spec.jcs.clone(),
                signed_by: host.to_string(),
                result,
                detail,
                branches_total: cov.branches,
                branches_covered: cov.covered,
            });
        }
    }
    Ok(rows)
}

/// As [`publish_stored_lanes`], with the public rule (see [`publish_stored_checked`]): the path of
/// `datumesh-mesh publish-stored` (D8).
pub fn publish_stored_checked_lanes<S: Store + Sync + ?Sized>(
    store: &S,
    key: &SigningKey,
    host: &str,
    owned: &[String],
    path: &Path,
    opts: &PublishOptions,
) -> Result<Published, PublishError> {
    let lanes = publish_lanes()?;
    publish_planned(store, key, host, owned, path, opts, true, |s, ops| {
        write(s, ops, |s, g, out| write_cells(s, g, lanes, out))
    })
}

/// The first pass (the planner) and the second pass (`write_ops`) of a publish.
#[allow(clippy::too_many_arguments)]
fn publish_planned<S: Store + ?Sized>(
    store: &S,
    key: &SigningKey,
    host: &str,
    owned: &[String],
    path: &Path,
    opts: &PublishOptions,
    checked: bool,
    write_ops: impl FnOnce(&S, &[Op]) -> Result<(Published, Written), PublishError>,
) -> Result<Published, PublishError> {
    let lines = read_lines(path)?;
    // The key's start: the earliest `issued`.
    let key_from = store
        .namespace(host)
        .ok()
        .flatten()
        .and_then(|n| json::parse(&n.keydoc).ok())
        .and_then(|d| match d.get("keys") {
            Some(Value::Array(ks)) => ks
                .iter()
                .find(|k| k.get("id").and_then(Value::as_str) == Some(key.key_id()))
                .and_then(|k| k.get("valid_from").and_then(Value::as_str).map(str::to_string)),
            _ => None,
        })
        .ok_or_else(|| {
            PublishError::at(0, format!("the key {} is not in {host}'s key document in the store", key.key_id()))
        })?;
    let now = opts.clock.now();
    let ctx = Ctx { host, owned, key_from: &key_from, now: &now, base: path.parent().unwrap_or(Path::new(".")) };
    let mut planner = Planner {
        store,
        key,
        ctx,
        backfill: opts.backfill.is_some(),
        sim: Simulation::default(),
        reads: Reads::default(),
        ops: Vec::new(),
        binds: Vec::new(),
        tests: Vec::new(),
        formulas: Vec::new(),
    };
    planner.run(&lines)?;
    let Planner { ops, binds, tests, formulas, .. } = planner;
    if opts.backfill.is_some() {
        backfill_check(store, &binds)?;
    }
    // The tests of the file's computed functions, run over the store and the file's own writes:
    // refused before anything is written (Build 2 Part 3, D8).
    let rows = check_tests(store, &overlay_of(&ops)?, &tests, &formulas, host, &now, checked)?;
    let (out, written) = write_ops(store, &ops)?;
    index_cell_values(store, &ops).map_err(|e| PublishError::at(0, format!("catalogue: {e}")))?;
    testrun::apply_rows(store, &rows, &now).map_err(|e| PublishError::at(0, format!("tests: {e}")))?;
    let rebound: Vec<String> = ops
        .iter()
        .filter_map(|op| match op {
            Op::Function { name, bind: true, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    testrun::rerun_readers(store, &rebound, &now).map_err(|e| PublishError::at(0, format!("tests: {e}")))?;
    if let Some(log) = &opts.backfill {
        if out.functions + out.classes + out.versions > 0 {
            append_log(log, &written).map_err(|e| PublishError::at(0, format!("backfill log: {e}")))?;
        }
    }
    Ok(out)
}
