//! Publishing stored cells as the mesh (`datumesh-mesh publish-stored`): a JSON lines file of
//! stored functions and their cells, signed with the mesh's key, put in the store.
//!
//! Each line is one of:
//!
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
//!   earlier one), `bound` later than theirs.
//! * a cell: `{"cell": {"fn": "<ns>/<name>", "args": {<name>: <typed scalar>}}, "value": <typed
//!   scalar> or {"blob": "<file>", "media_type": "<type>"}, "tx", "asof", "issued"?, "licence",
//!   "summary"}`: a blob's file (relative to the JSON lines file) is put with `put_blob` and the
//!   value is `<sha256>:<length>:<media type>`; the receipt is signed with the mesh's key
//!   (`computed_by` the mesh host, `publisher` the cell's namespace, `valid` its valid-time
//!   arguments; `issued` defaults to the later of `asof` and the key's start) and stored with
//!   `put_receipt` and `put_version`.
//!
//! The function of a cell is the one its name is bound to as of the cell's `tx`. Re-running a
//! file changes nothing: descriptors and blobs are content-addressed, signing is deterministic
//! (the same receipt, the same hash), and a version already there is skipped. The key is never
//! logged.
//!
//! Rule A (since 0.3.0, CONTRACT-1.8b H1): no binding starts earlier than the mesh clock. The
//! start of a binding (a name's, a class's, or a class method's `N/m`) is its `bound`, or the
//! clock's now when `bound` is left out, and a start earlier than now is refused with
//! [`PublishErrorKind::StartBeforeNow`]. The file is read in two passes: the first works out
//! which binds would happen (a bind is skipped when the name is already bound to that hash as of
//! `9999-12-31T23:59:59.999999Z`) and checks their starts; nothing is written unless every check
//! passes. A `--backfill` run ([`PublishOptions::backfill`]) lifts the start check, and is refused
//! ([`PublishErrorKind::Backfill`]) when a name it would bind has a binding, or when a namespace
//! of such a name is in the operator log of an earlier backfill. (Store 0.4.1 cannot list the
//! names under a namespace; the log is what stands in for that check. A run that writes appends
//! one line to the log: `{"at": …, "namespaces": […], "starts": […]}`, a JCS object.)

use std::collections::BTreeSet;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use datumesh_core::json::{self, Value};
use datumesh_core::{Arg, Args, Cell, ClassDescriptor, FunctionDescriptor, ReceiptBuilder, Scalar, SigningKey, Type};
use datumesh_engine::{Clock, SystemClock};
use datumesh_formula::definition::{Callee, Callees, ClassCallee};
use datumesh_store::Store;

/// The end of time: a binding as of it is the latest one.
const END: &str = "9999-12-31T23:59:59.999999Z";

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

/// The bindings a dry run has made so far: the file's own, which are seen before the store's.
#[derive(Default)]
struct Simulation {
    /// Function name to the (tx, function hash) of each bind the file would make.
    functions: std::collections::BTreeMap<String, Vec<(String, String)>>,
    /// Class name to the (tx, class hash) of each class bind the file would make.
    classes: std::collections::BTreeMap<String, Vec<(String, String)>>,
    /// Canonical bytes by function hash, of the descriptors the file would store.
    descriptors: std::collections::BTreeMap<String, Vec<u8>>,
    /// Canonical bytes by class hash, of the classes the file would store.
    class_bytes: std::collections::BTreeMap<String, Vec<u8>>,
}

impl Simulation {
    /// The latest of `binds`' entries for `name` with tx <= `at`.
    fn latest(
        binds: &std::collections::BTreeMap<String, Vec<(String, String)>>,
        name: &str,
        at: &str,
    ) -> Option<(String, String)> {
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

/// A binding a run would make: the line that makes it and its name.
struct Bind {
    line: usize,
    name: String,
}

/// Records a bind that would happen, refusing a start earlier than now unless backfilling.
fn record(
    binds: &mut Vec<Bind>,
    line: usize,
    name: &str,
    start: &str,
    now: &str,
    backfill: bool,
) -> Result<(), PublishError> {
    if !backfill && start < now {
        return Err(PublishError::typed(
            line,
            PublishErrorKind::StartBeforeNow { name: name.into(), start: start.into(), now: now.into() },
        ));
    }
    binds.push(Bind { line, name: name.into() });
    Ok(())
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

/// Pass one: the binds the lines would make, each start checked (rule A), nothing written.
fn plan<S: Store + ?Sized>(
    store: &S,
    ctx: &Ctx<'_>,
    lines: &[(usize, String)],
    backfill: bool,
) -> Result<Vec<Bind>, PublishError> {
    let mut sim = Simulation::default();
    let mut binds = Vec::new();
    for (n, text) in lines {
        let n = *n;
        let err = |d: String| PublishError::at(n, d);
        let v = json::parse(text.as_bytes()).map_err(|e| err(e.to_string()))?;
        if let Some(name) = v.get("class").and_then(Value::as_str) {
            let descriptor = v.get("descriptor").ok_or_else(|| err("a class without descriptor".into()))?;
            let start = bound_of(&v, n, ctx.now)?;
            let def = datumesh_formula::definition::define_class(
                name,
                &descriptor.to_jcs(),
                definer(name, ctx.host, ctx.owned),
                &Callers { store, sim: Some(&sim) },
                &start,
            )
            .map_err(|e| err(format!("define {name}: {e}")))?;
            if current_class(store, &sim, name).map_err(err)?.as_deref() != Some(def.class_hash.as_str()) {
                record(&mut binds, n, name, &start, ctx.now, backfill)?;
                sim.classes.entry(name.to_string()).or_default().push((start.clone(), def.class_hash.clone()));
                sim.class_bytes.insert(def.class_hash.clone(), def.bytes.clone());
                // bind_class binds the methods too, at the class's start.
                for (m, fh) in &def.descriptor.methods {
                    let mname = format!("{name}/{m}");
                    if current_function(store, &sim, &mname).map_err(err)?.as_deref() != Some(fh.as_str()) {
                        record(&mut binds, n, &mname, &start, ctx.now, backfill)?;
                        sim.functions.entry(mname).or_default().push((start.clone(), fh.clone()));
                    }
                }
            }
            continue;
        }
        let function = if let Some(name) = v.get("function").and_then(Value::as_str) {
            let sig = v.get("signature").ok_or_else(|| err("a function without a signature".into()))?;
            let start = bound_of(&v, n, ctx.now)?;
            let d = Value::Object(vec![
                ("code".into(), Value::Object(vec![("body".into(), s("")), ("lang".into(), s("stored"))])),
                ("inputs".into(), Value::Array(vec![])),
                ("signature".into(), sig.clone()),
            ]);
            let fd = FunctionDescriptor::from_value(&d).map_err(|e| err(e.to_string()))?;
            Some((name, start, fd.hash(), fd.canonical()))
        } else if let Some(name) = v.get("formula").and_then(Value::as_str) {
            let source =
                v.get("source").and_then(Value::as_str).ok_or_else(|| err("a formula without source".into()))?;
            let sig = v.get("signature").ok_or_else(|| err("a formula without a signature".into()))?;
            let start = bound_of(&v, n, ctx.now)?;
            let def = datumesh_formula::definition::define_function(
                name,
                source,
                sig,
                &Default::default(),
                &Callers { store, sim: Some(&sim) },
                &start,
            )
            .map_err(|e| err(format!("define {name}: {e}")))?;
            Some((name, start, def.fn_hash, def.bytes))
        } else if v.get("cell").is_none() {
            return Err(err("neither a function nor a cell".into()));
        } else {
            None
        };
        if let Some((name, start, h, bytes)) = function {
            if current_function(store, &sim, name).map_err(err)?.as_deref() != Some(h.as_str()) {
                record(&mut binds, n, name, &start, ctx.now, backfill)?;
                sim.functions.entry(name.to_string()).or_default().push((start, h.clone()));
                sim.descriptors.insert(h, bytes);
            }
        }
    }
    Ok(binds)
}

/// The namespaces a backfill log holds (every line's `namespaces`); none when there is no log.
fn logged_namespaces(log: &Path) -> Result<BTreeSet<String>, String> {
    let text = match std::fs::read_to_string(log) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(e) => return Err(format!("{}: {e}", log.display())),
    };
    let mut out = BTreeSet::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v = json::parse(line.as_bytes()).map_err(|e| format!("{}: {}", log.display(), e))?;
        if let Some(Value::Array(ns)) = v.get("namespaces") {
            out.extend(ns.iter().filter_map(|n| n.as_str().map(str::to_string)));
        }
    }
    Ok(out)
}

/// The backfill's checks: no name the file binds has a binding, and no namespace of one is in the
/// log of an earlier backfill.
fn backfill_check<S: Store + ?Sized>(store: &S, binds: &[Bind], log: &Path) -> Result<(), PublishError> {
    let logged = logged_namespaces(log).map_err(|e| PublishError::typed(0, PublishErrorKind::Backfill(e)))?;
    for b in binds {
        let refuse = |why: String| PublishError::typed(b.line, PublishErrorKind::Backfill(why));
        if store.resolve_binding(&b.name, END).map_err(|e| PublishError::at(b.line, e.to_string()))?.is_some() {
            return Err(refuse(format!("{} has a binding", b.name)));
        }
        let ns = namespace_of(&b.name);
        if logged.contains(ns) {
            return Err(refuse(format!("{ns} has bindings: an earlier backfill wrote under it")));
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

/// Pass two: the writes, the binds as the first pass found them (every start checked there).
fn write<S: Store + ?Sized>(
    store: &S,
    key: &SigningKey,
    ctx: &Ctx<'_>,
    lines: &[(usize, String)],
) -> Result<(Published, Written), PublishError> {
    let mut out = Published::default();
    let mut written = Written::default();
    for (n, text) in lines {
        let n = *n;
        let err = |d: String| PublishError::at(n, d);
        let v = json::parse(text.as_bytes()).map_err(|e| err(e.to_string()))?;
        if let Some(name) = v.get("class").and_then(Value::as_str) {
            let descriptor = v.get("descriptor").ok_or_else(|| err("a class without descriptor".into()))?;
            let start = bound_of(&v, n, ctx.now)?;
            let def = datumesh_formula::definition::define_class(
                name,
                &descriptor.to_jcs(),
                definer(name, ctx.host, ctx.owned),
                &Callers { store, sim: None },
                &start,
            )
            .map_err(|e| err(format!("define {name}: {e}")))?;
            let bound_hash = store.put_class(&def.bytes).map_err(|e| err(e.to_string()))?;
            match store.resolve_class(name, END).map_err(|e| err(e.to_string()))? {
                Some(h) if h == bound_hash => {}
                _ => {
                    store.bind_class(name, &bound_hash, &start).map_err(|e| err(format!("bind {name}: {e}")))?;
                    out.classes += 1;
                    written.bound(name, &start);
                    for m in def.descriptor.methods.keys() {
                        written.bound(&format!("{name}/{m}"), &start);
                    }
                }
            }
            continue;
        }
        if let Some(name) = v.get("function").and_then(Value::as_str) {
            let sig = v.get("signature").ok_or_else(|| err("a function without a signature".into()))?;
            let start = bound_of(&v, n, ctx.now)?;
            let d = Value::Object(vec![
                ("code".into(), Value::Object(vec![("body".into(), s("")), ("lang".into(), s("stored"))])),
                ("inputs".into(), Value::Array(vec![])),
                ("signature".into(), sig.clone()),
            ]);
            let fd = FunctionDescriptor::from_value(&d).map_err(|e| err(e.to_string()))?;
            let h = store.put_descriptor(&fd.canonical()).map_err(|e| err(e.to_string()))?;
            match store.resolve_binding(name, END).map_err(|e| err(e.to_string()))? {
                Some(b) if b.fn_hash == h => {}
                _ => {
                    store.bind_name(name, &h, &start).map_err(|e| err(format!("bind {name}: {e}")))?;
                    out.functions += 1;
                    written.bound(name, &start);
                }
            }
            continue;
        }
        if let Some(name) = v.get("formula").and_then(Value::as_str) {
            let source =
                v.get("source").and_then(Value::as_str).ok_or_else(|| err("a formula without source".into()))?;
            let sig = v.get("signature").ok_or_else(|| err("a formula without a signature".into()))?;
            let start = bound_of(&v, n, ctx.now)?;
            let def = datumesh_formula::definition::define_function(
                name,
                source,
                sig,
                &Default::default(),
                &Callers { store, sim: None },
                &start,
            )
            .map_err(|e| err(format!("define {name}: {e}")))?;
            let h = store.put_descriptor(&def.bytes).map_err(|e| err(e.to_string()))?;
            match store.resolve_binding(name, END).map_err(|e| err(e.to_string()))? {
                Some(b) if b.fn_hash == h => {}
                _ => {
                    store.bind_name(name, &h, &start).map_err(|e| err(format!("bind {name}: {e}")))?;
                    out.functions += 1;
                    written.bound(name, &start);
                }
            }
            continue;
        }
        let cell_v = v.get("cell").ok_or_else(|| err("neither a function nor a cell".into()))?;
        let name = cell_v.get("fn").and_then(Value::as_str).ok_or_else(|| err("a cell without fn".into()))?;
        let (ns, f) = name.rsplit_once('/').ok_or_else(|| err(format!("{name}: not a function name")))?;
        let field =
            |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| err(format!("no {k}")));
        let (tx, asof, licence, summary) = (field("tx")?, field("asof")?, field("licence")?, field("summary")?);
        let issued = v
            .get("issued")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| std::cmp::max(asof.clone(), ctx.key_from.to_string()));
        let fn_hash = store
            .resolve_name(name, &tx)
            .map_err(|e| err(e.to_string()))?
            .ok_or_else(|| err(format!("{name} is not bound as of {tx}")))?;
        let d = store.descriptor(&fn_hash).ok_or_else(|| err(format!("no descriptor {fn_hash}")))?;
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
        let value_v = v.get("value").ok_or_else(|| err("a cell without value".into()))?;
        let value = match value_v.get("blob").and_then(Value::as_str) {
            Some(file) => {
                let media = value_v.get("media_type").and_then(Value::as_str).unwrap_or("application/octet-stream");
                let bytes = std::fs::read(ctx.base.join(file)).map_err(|e| err(format!("{file}: {e}")))?;
                let sha = store.put_blob(&bytes).map_err(|e| err(e.to_string()))?;
                out.blobs += 1;
                out.blob_bytes += bytes.len() as u64;
                Scalar::from_lexical(Type::Blob, &format!("{sha}:{}:{media}", bytes.len()))
                    .map_err(|e| err(e.to_string()))?
            }
            None => Scalar::from_json(&value_v.to_jcs()).map_err(|e| err(e.to_string()))?,
        };
        let cell = Cell { ns: ns.to_string(), fn_name: f.to_string(), fn_hash, args };
        let r = ReceiptBuilder::new()
            .cell(cell.clone())
            .value(value)
            .valid(valid)
            .asof(asof.as_str())
            .tx(tx.as_str())
            .issued(issued.as_str())
            .publisher(ns)
            .signer_host(ctx.host)
            .licence(licence.as_str())
            .summary(summary.as_str())
            .sign(key)
            .map_err(|e| err(e.to_string()))?;
        let h = store.put_receipt(&receipt_json(&r)).map_err(|e| err(e.to_string()))?;
        out.cells += 1;
        out.receipts += 1;
        let id = cell.id();
        match store.latest(&id).map_err(|e| err(e.to_string()))? {
            Some(l) if l.tx.as_str() >= tx.as_str() => {
                if store.version_asof(&id, &tx).ok().flatten().is_none_or(|v| v.receipt_hash != h) {
                    return Err(err(format!("{id} has a version at or after {tx} that is not this receipt")));
                }
            }
            _ => {
                store.put_version(&id, &tx, &h).map_err(|e| err(e.to_string()))?;
                out.versions += 1;
            }
        }
    }
    Ok((out, written))
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
/// The clock is the system clock, and there is no backfill.
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

/// As [`publish_stored_in`], with the run's clock and backfill (CONTRACT-1.8b H1, rule A).
///
/// Errors: a line refused by its checks, or a binding whose start is earlier than the clock
/// ([`PublishErrorKind::StartBeforeNow`]), or a backfill refused ([`PublishErrorKind::Backfill`]).
/// Nothing is written when the first pass refuses.
pub fn publish_stored_with<S: Store + ?Sized>(
    store: &S,
    key: &SigningKey,
    host: &str,
    owned: &[String],
    path: &Path,
    opts: &PublishOptions,
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
    let binds = plan(store, &ctx, &lines, opts.backfill.is_some())?;
    if let Some(log) = &opts.backfill {
        backfill_check(store, &binds, log)?;
    }
    let (out, written) = write(store, key, &ctx, &lines)?;
    if let Some(log) = &opts.backfill {
        if out.functions + out.classes + out.versions > 0 {
            append_log(log, &written).map_err(|e| PublishError::at(0, format!("backfill log: {e}")))?;
        }
    }
    Ok(out)
}
