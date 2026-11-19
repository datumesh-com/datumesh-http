//! Test runs (Build 2 Part 3, E5): a computed function's tests evaluated against the store, the
//! derived edge cases and branch coverage the public rule counts, and the rerun of the tests of
//! the functions that read a rebound name.
//!
//! A test (`datumesh-http`'s format, C4) is `{"test": <function name>, "call": <call>, "asof"?,
//! "inputs"?: {<call of an input cell>: <typed scalar>}, "expect": <typed scalar> | {"fault":
//! <code>}}`. Its case id is the first 16 hex digits of the SHA-256 of its JCS.
//!
//! The evaluation is the formula evaluator's ([`datumesh_formula::eval::evaluate_observed`]) over a
//! [`World`] that reads the store, and a file's own planned writes first (its [`Overlay`]), so that
//! a file is checked before anything of it is written. Inputs are overrides of that world: the
//! evaluation of a test is a scope, as a `whatif` is.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use datumesh_core::json::{self, Value};
use datumesh_core::{Arg, Args as CoreArgs, Cell, FunctionDescriptor, Receipt, Scalar, Slice, Type};
use datumesh_engine::{Budget, EvalError, Inputs, Runtime};
use datumesh_formula::coverage::{self, BranchKind, Coverage, EvalObserver};
use datumesh_formula::eval::{self, Args, Binding, CellKey, Outcome, PatternArg, SliceKey, Stop, Version, World};
use datumesh_formula::{check_descriptor, Code, Compiled, Env, Function};
use datumesh_store::{FunctionTest, Store};

use crate::call::{self, Bindings, Bound};
use datumesh_core::ErrorCode;

/// The lane of the test-case cells (`<host>/test_case`).
pub const TEST_CASE_LANG: &str = "test_case";
/// The name of the test-case function under the mesh's host.
pub const TEST_CASE_NAME: &str = "test_case";
/// The argument of a test-case cell that names the function under test (5.1 reserves `fn_hash`).
pub const TEST_CASE_HASH_ARG: &str = "function_hash";

/// Fuel of one test run (the formula evaluator's budget; a test that needs more fails).
const FUEL: u64 = 10_000_000;
/// The deepest nesting of computed cells a test run follows.
const DEPTH: usize = 200;

/// A test line, parsed.
#[derive(Clone, Debug, PartialEq)]
pub struct TestSpec {
    /// The function's name (`test`).
    pub function: String,
    /// The call (`call`).
    pub call: String,
    /// The asof (`asof`), if the test has one.
    pub asof: Option<String>,
    /// The inputs: the call of each input cell and its typed value, in the file's order.
    pub inputs: Vec<(String, Value)>,
    /// What the test expects.
    pub expect: Expect,
    /// The test's JCS.
    pub jcs: Vec<u8>,
    /// The case id: the first 16 hex digits of the SHA-256 of [`TestSpec::jcs`].
    pub case: String,
}

/// What a test expects: a typed value or a fault.
#[derive(Clone, Debug, PartialEq)]
pub enum Expect {
    /// A value of the function's return type.
    Value(Scalar),
    /// A fault, by its code.
    Fault(String),
}

/// The text of a test's expectation, for a detail.
fn expect_text(e: &Expect) -> String {
    match e {
        Expect::Value(s) => s.lexical().to_string(),
        Expect::Fault(c) => c.clone(),
    }
}

/// Parses a test object (a `test` line, or a bundle's `tests` member).
pub fn parse_test(v: &Value) -> Result<TestSpec, String> {
    let obj = v.as_object().ok_or("a test is a JSON object")?;
    let text = |k: &str| -> Result<String, String> {
        v.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("a test has no string {k}"))
    };
    for (k, _) in obj {
        if !matches!(k.as_str(), "test" | "call" | "asof" | "inputs" | "expect") {
            return Err(format!("a test has no member {k}"));
        }
    }
    let function = text("test")?;
    let call = text("call")?;
    let asof = match v.get("asof") {
        None => None,
        Some(a) => Some(a.as_str().ok_or("a test's asof is a string")?.to_string()),
    };
    let mut inputs = Vec::new();
    if let Some(i) = v.get("inputs") {
        for (call, typed) in i.as_object().ok_or("a test's inputs is an object")? {
            typed_scalar(typed).map_err(|e| format!("input {call}: {e}"))?;
            inputs.push((call.clone(), typed.clone()));
        }
    }
    let expect = match v.get("expect") {
        Some(Value::Object(m)) if m.len() == 1 && m[0].0 == "fault" => {
            Expect::Fault(m[0].1.as_str().ok_or("a fault is a code")?.to_string())
        }
        Some(t) => Expect::Value(typed_scalar(t).map_err(|e| format!("expect: {e}"))?),
        None => return Err("a test has no expect".into()),
    };
    let jcs = v.to_jcs();
    let case = datumesh_core::sha256_hex(&jcs)[..16].to_string();
    Ok(TestSpec { function, call, asof, inputs, expect, jcs, case })
}

/// A typed scalar `{"type", "value"}` of the JSON object `v`.
pub fn typed_scalar(v: &Value) -> Result<Scalar, String> {
    let ty = v.get("type").and_then(Value::as_str).ok_or("a typed scalar has a type")?;
    let value = v.get("value").and_then(Value::as_str).ok_or("a typed scalar has a value")?;
    let ty = Type::from_name(ty).ok_or_else(|| format!("unknown type {ty}"))?;
    Scalar::from_lexical(ty, value).map_err(|e| e.to_string())
}

/// The planned writes of a run, as the store will see them: the functions bound, their
/// descriptors, and the cell versions. Empty for a run with nothing planned (a rerun).
#[derive(Default, Clone, Debug)]
pub struct Overlay {
    /// Function name to (start, fn hash) of each binding the run makes.
    pub functions: BTreeMap<String, Vec<(String, String)>>,
    /// Canonical descriptor bytes by fn hash.
    pub descriptors: BTreeMap<String, Vec<u8>>,
    /// The cell versions the run makes.
    pub cells: Vec<PlannedCell>,
}

/// One planned cell version.
#[derive(Clone, Debug)]
pub struct PlannedCell {
    /// The function name.
    pub name: String,
    /// The cell id.
    pub id: String,
    /// The version's `tx`.
    pub tx: String,
    /// Its receipt's hash.
    pub receipt_hash: String,
    /// The arguments.
    pub args: Args,
    /// The value.
    pub value: Scalar,
}

impl Overlay {
    /// Records a planned receipt (its JSON as the run signs it).
    pub fn cell(&mut self, receipt: &[u8]) -> Result<(), String> {
        let r = Receipt::from_json(receipt).map_err(|e| e.to_string())?;
        let cell = r.cell();
        let name = format!("{}/{}", cell.ns, cell.fn_name);
        let args = plain_args(&cell.args);
        self.cells.push(PlannedCell {
            name,
            id: cell.id(),
            tx: r.tx().to_string(),
            receipt_hash: r.hash().to_string(),
            args,
            value: r.value().clone(),
        });
        Ok(())
    }
}

/// The value arguments of a core argument map.
fn plain_args(args: &CoreArgs) -> Args {
    args.iter()
        .filter_map(|(k, a)| match a {
            Arg::Value(s) => Some((k.clone(), s.clone())),
            Arg::Slice => None,
        })
        .collect()
}

fn core_args(args: &Args) -> CoreArgs {
    args.iter().map(|(k, s)| (k.clone(), Arg::Value(s.clone()))).collect()
}

/// The namespace and the local name of a fully qualified function name.
fn split(name: &str) -> (&str, &str) {
    name.rsplit_once('/').unwrap_or(("", name))
}

/// The cell id of `name` with `hash` and `args`.
fn cell_id(name: &str, hash: &str, args: &Args) -> String {
    let (ns, f) = split(name);
    Cell { ns: ns.into(), fn_name: f.into(), fn_hash: hash.into(), args: core_args(args) }.id()
}

/// Which function the name `name` is bound to as of `at`, and since when: the store's binding or the
/// overlay's, the later one.
pub fn binding_of<S: Store + ?Sized>(
    store: &S,
    ov: &Overlay,
    name: &str,
    at: &str,
) -> Result<Option<(String, String)>, String> {
    let stored = store.resolve_binding(name, at).map_err(|e| e.to_string())?;
    let planned = ov
        .functions
        .get(name)
        .and_then(|v| v.iter().filter(|(t, _)| t.as_str() <= at).max_by(|a, b| a.0.cmp(&b.0)).cloned());
    Ok(match (stored, planned) {
        (Some(b), Some((t, h))) if t >= b.tx => Some((h, t)),
        (Some(b), _) => Some((b.fn_hash, b.tx)),
        (None, Some((t, h))) => Some((h, t)),
        (None, None) => None,
    })
}

/// The canonical bytes of the descriptor `hash`: planned, else stored.
pub fn descriptor_of<S: Store + ?Sized>(store: &S, ov: &Overlay, hash: &str) -> Result<Option<Vec<u8>>, String> {
    if let Some(b) = ov.descriptors.get(hash) {
        return Ok(Some(b.clone()));
    }
    Ok(store.descriptor(hash).map(|b| b.into_owned()))
}

fn descriptor_value<S: Store + ?Sized>(store: &S, ov: &Overlay, hash: &str) -> Result<Value, String> {
    let bytes = descriptor_of(store, ov, hash)?.ok_or_else(|| format!("descriptor {hash} is not stored"))?;
    json::parse(&bytes).map_err(|e| e.to_string())
}

/// What the evaluator needs of a bound function: its signature and body, and its compiled form
/// when it is computed.
pub(crate) struct Built {
    pub(crate) function: Arc<Function>,
    pub(crate) compiled: Option<Compiled>,
}

/// Ignores the evaluator's branch arms (the callees').
struct Quiet;

impl EvalObserver for Quiet {
    fn branch_taken(&mut self, _id: datumesh_formula::NodeId, _arm: usize) {}
}

/// The formula world of a test run: the store, the file's overlay, the evaluation's asof, the
/// test's inputs (overrides), and what the run saw (the absent cells and the empty slices).
pub struct Run<'a, S: Store + ?Sized> {
    store: &'a S,
    overlay: &'a Overlay,
    asof: String,
    overrides: BTreeMap<String, Scalar>,
    used: u64,
    depth: usize,
    built: BTreeMap<String, Arc<Built>>,
    /// `absent:<name>` for each input read with no value, `empty:<name>` for each slice with no member.
    pub events: BTreeSet<String>,
}

/// The world's error: a store or a definition that does not work.
#[derive(Debug)]
pub struct RunError(pub String);

impl<'a, S: Store + ?Sized> Run<'a, S> {
    /// A run at `asof` with `inputs`: each `(call, scalar)` of a test's inputs, resolved to a cell.
    pub fn new(store: &'a S, overlay: &'a Overlay, asof: &str) -> Run<'a, S> {
        Run {
            store,
            overlay,
            asof: asof.to_string(),
            overrides: BTreeMap::new(),
            used: 0,
            depth: 0,
            built: BTreeMap::new(),
            events: BTreeSet::new(),
        }
    }

    fn key(name: &str, args: &Args) -> String {
        let parts: Vec<String> = args.iter().map(|(k, v)| format!("{k}={}", v.lexical())).collect();
        format!("{name}({})", parts.join(","))
    }

    pub(crate) fn built(&mut self, name: &str, hash: &str) -> Result<Arc<Built>, RunError> {
        if let Some(b) = self.built.get(hash) {
            return Ok(b.clone());
        }
        let v = descriptor_value(self.store, self.overlay, hash).map_err(RunError)?;
        let function = Function::from_value(&v).map_err(|e| RunError(e.to_string()))?;
        let compiled = if function.lang == "formula" {
            let mut env = Env::new(name, function.signature.clone());
            for callee in &function.inputs {
                if callee == name {
                    continue;
                }
                if let Some((h, _)) = binding_of(self.store, self.overlay, callee, &self.asof).map_err(RunError)? {
                    let cv = descriptor_value(self.store, self.overlay, &h).map_err(RunError)?;
                    env.add_function(callee, &cv).map_err(|e| RunError(e.to_string()))?;
                }
            }
            Some(check_descriptor(name, &v, &env).map_err(|e| RunError(e.to_string()))?)
        } else {
            None
        };
        let b = Arc::new(Built { function: Arc::new(function), compiled });
        self.built.insert(hash.to_string(), b.clone());
        Ok(b)
    }

    /// Evaluates a computed function's body at `t` with `args`, observed by `obs`.
    fn evaluate(
        &mut self,
        b: &Built,
        args: &Args,
        t: &str,
        obs: &mut dyn EvalObserver,
    ) -> Result<Outcome, Stop<RunError>> {
        let Some(c) = &b.compiled else {
            return Err(Stop::World(RunError("a stored function has no body to evaluate".into())));
        };
        if self.depth >= DEPTH {
            return Err(Stop::World(RunError("computed cells nest too deeply".into())));
        }
        self.depth += 1;
        let r = eval::evaluate_observed(c, &b.function, args, t, self, obs);
        self.depth -= 1;
        r
    }

    fn stored_value(&self, id: &str, t: &str) -> Result<Option<Scalar>, RunError> {
        if let Some(p) =
            self.overlay.cells.iter().filter(|p| p.id == id && p.tx.as_str() <= t).max_by(|a, b| a.tx.cmp(&b.tx))
        {
            return Ok(Some(p.value.clone()));
        }
        let Some(v) = self.store.version_asof(id, t).map_err(|e| RunError(e.to_string()))? else {
            return Ok(None);
        };
        let bytes = self.store.receipt(&v.receipt_hash).ok_or_else(|| RunError("a version has no receipt".into()))?;
        let r = Receipt::from_json(&bytes).map_err(|e| RunError(e.to_string()))?;
        Ok(Some(r.value().clone()))
    }

    /// The members of a stored function's slice: the store's, and the overlay's (the later version
    /// of a cell wins).
    fn slice(&mut self, slice: &SliceKey) -> Result<Vec<(Args, Version)>, RunError> {
        let t = self.asof.clone();
        let Some((hash, _)) = binding_of(self.store, self.overlay, &slice.name, &t).map_err(RunError)? else {
            return Ok(Vec::new());
        };
        let b = self.built(&slice.name, &hash)?;
        if b.compiled.is_some() {
            return Err(RunError("a slice of a computed function is not read by a test run".into()));
        }
        let mut members: BTreeMap<String, (Args, Version)> = BTreeMap::new();
        let mut obj = BTreeMap::new();
        for (k, p) in &slice.args {
            let v = match p {
                PatternArg::Value(s) => {
                    let ty = b.function.signature.args.get(k).map(|d| d.ty).unwrap_or(s.ty());
                    Value::Object(vec![
                        ("type".into(), Value::String(ty.as_str().into())),
                        ("value".into(), Value::String(s.lexical().into())),
                    ])
                }
                PatternArg::Open => Value::String("*".into()),
                PatternArg::Range(..) => return Err(RunError("a range is not read by a test run".into())),
            };
            obj.insert(k.clone(), v);
        }
        let (ns, f) = split(&slice.name);
        let sv = Value::Object(vec![
            ("ns".into(), Value::String(ns.into())),
            ("fn".into(), Value::String(f.into())),
            ("fn_hash".into(), Value::String(hash.clone())),
            ("args".into(), Value::Object(obj.into_iter().collect())),
        ]);
        let core_slice = Slice::from_value(&sv).map_err(|e| RunError(e.to_string()))?;
        for sv in self.store.slice_values(&core_slice, &t).map_err(|e| RunError(e.to_string()))? {
            let args = plain_args(&sv.cell.args);
            let id = sv.cell.id();
            let ver = Version {
                value: sv.value,
                receipt_hash: sv.version.receipt_hash.clone(),
                tx: sv.version.tx.clone(),
                asof: sv.version.asof.clone(),
            };
            members.insert(id, (args, ver));
        }
        for p in &self.overlay.cells {
            if p.name != slice.name || p.tx.as_str() > t.as_str() {
                continue;
            }
            let matches = slice.args.iter().all(|(k, pat)| match pat {
                PatternArg::Value(v) => p.args.get(k) == Some(v),
                PatternArg::Open => true,
                PatternArg::Range(..) => false,
            });
            if !matches {
                continue;
            }
            let newer = members.get(&p.id).is_none_or(|(_, v)| v.tx < p.tx);
            if newer {
                let ver = Version {
                    value: p.value.clone(),
                    receipt_hash: p.receipt_hash.clone(),
                    tx: p.tx.clone(),
                    asof: p.tx.clone(),
                };
                members.insert(p.id.clone(), (p.args.clone(), ver));
            }
        }
        Ok(members.into_values().collect())
    }
}

impl<S: Store + ?Sized> World for Run<'_, S> {
    type Error = RunError;

    fn function(&mut self, name: &str, at: Option<&str>) -> Result<Option<Binding>, RunError> {
        let at = at.map_or_else(|| self.asof.clone(), str::to_string);
        let Some((hash, start)) = binding_of(self.store, self.overlay, name, &at).map_err(RunError)? else {
            return Ok(None);
        };
        let b = self.built(name, &hash)?;
        Ok(Some(Binding { function: b.function.clone(), start }))
    }

    fn read(&mut self, cell: &CellKey, at: Option<&str>) -> Result<Option<Version>, Stop<RunError>> {
        let t = at.map_or_else(|| self.asof.clone(), str::to_string);
        if let Some(v) = self.overrides.get(&Run::<S>::key(&cell.name, &cell.args)) {
            return Ok(Some(Version { value: v.clone(), receipt_hash: String::new(), tx: t.clone(), asof: t }));
        }
        let Some((hash, _)) =
            binding_of(self.store, self.overlay, &cell.name, &t).map_err(|e| Stop::World(RunError(e)))?
        else {
            self.events.insert(format!("absent:{}", cell.name));
            return Err(Stop::Code(Code::Absent));
        };
        let b = self.built(&cell.name, &hash).map_err(Stop::World)?;
        if b.compiled.is_some() {
            let mut quiet = Quiet;
            return match self.evaluate(&b, &cell.args, &t, &mut quiet) {
                Ok(o) => Ok(Some(Version { value: o.value, receipt_hash: String::new(), tx: t.clone(), asof: t })),
                Err(Stop::Code(c)) => {
                    if c == Code::Absent {
                        self.events.insert(format!("absent:{}", cell.name));
                    }
                    Err(Stop::Code(c))
                }
                Err(e) => Err(e),
            };
        }
        let id = cell_id(&cell.name, &hash, &cell.args);
        match self.stored_value(&id, &t).map_err(Stop::World)? {
            Some(value) => {
                let tx = self.store.version_asof(&id, &t).ok().flatten().map_or_else(|| t.clone(), |v| v.tx);
                Ok(Some(Version { value, receipt_hash: String::new(), tx, asof: t }))
            }
            None => {
                self.events.insert(format!("absent:{}", cell.name));
                Err(Stop::Code(Code::Absent))
            }
        }
    }

    fn members(&mut self, slice: &SliceKey) -> Result<Vec<(Args, Version)>, Stop<RunError>> {
        let members = self.slice(slice).map_err(Stop::World)?;
        if members.is_empty() {
            self.events.insert(format!("empty:{}", slice.name));
        }
        Ok(members)
    }

    fn versions(&mut self, cell: &CellKey, _from: &str, _to: &str) -> Result<Vec<Version>, Stop<RunError>> {
        Err(Stop::World(RunError(format!("the versions of {} are not read by a test run", cell.name))))
    }

    fn charge(&mut self, n: u64) -> Result<(), Code> {
        self.used = self.used.saturating_add(n);
        if self.used > FUEL {
            return Err(Code::Budget);
        }
        Ok(())
    }

    fn asof(&mut self) -> Result<String, Stop<RunError>> {
        Ok(self.asof.clone())
    }
}

/// The call-resolution view of a run's names: the store's and the overlay's bindings.
pub struct Names<'a, S: Store + ?Sized> {
    pub store: &'a S,
    pub overlay: &'a Overlay,
}

impl<S: Store + ?Sized> Bindings for Names<'_, S> {
    fn binding(&self, fn_name: &str, asof: &str) -> Result<Option<Arc<Bound>>, ErrorCode> {
        let Some((hash, since)) =
            binding_of(self.store, self.overlay, fn_name, asof).map_err(|_| ErrorCode::InputUnavailable)?
        else {
            return Ok(None);
        };
        let bytes = descriptor_of(self.store, self.overlay, &hash)
            .map_err(|_| ErrorCode::InputUnavailable)?
            .ok_or(ErrorCode::FnUnavailable)?;
        let d = FunctionDescriptor::from_json(&bytes).map_err(|_| ErrorCode::FnUnavailable)?;
        Ok(Some(Arc::new(Bound::new(hash, since, Arc::new(d)))))
    }
}

/// The outcome of one test.
#[derive(Debug)]
pub struct Ran {
    /// The arguments of the call.
    pub args: Args,
    /// The value, or the code of the fault it raised.
    pub got: Result<Scalar, String>,
    /// Why the test could not run (the call, an input, a definition), if it could not.
    pub error: Option<String>,
    /// The branch arms taken by the function's own body.
    pub coverage: Coverage,
    /// The events of the run (`absent:`, `empty:`).
    pub events: BTreeSet<String>,
}

/// Runs a test over `store` and `overlay` (`now` is its asof unless it has one).
pub fn run_test<S: Store + ?Sized>(store: &S, overlay: &Overlay, now: &str, spec: &TestSpec) -> Ran {
    let asof = spec.asof.clone().unwrap_or_else(|| now.to_string());
    let names = Names { store, overlay };
    let fail = |error: String| Ran {
        args: Args::new(),
        got: Err(String::new()),
        error: Some(error),
        coverage: Coverage::new(),
        events: BTreeSet::new(),
    };
    let target = match call::resolve_call(&spec.call, &asof, &BTreeMap::new(), &names, false) {
        Ok(t) => t,
        Err(_) => return fail(format!("the call {} does not resolve", spec.call)),
    };
    let name = format!("{}/{}", target.cell.ns, target.cell.fn_name);
    let hash = target.cell.fn_hash.clone();
    let args = plain_args(&target.cell.args);
    let mut run = Run::new(store, overlay, &asof);
    for (call, typed) in &spec.inputs {
        let t = match call::resolve_call(call, &asof, &BTreeMap::new(), &names, false) {
            Ok(t) => t,
            Err(_) => return fail(format!("the input {call} does not resolve")),
        };
        let v = match typed_scalar(typed) {
            Ok(v) => v,
            Err(e) => return fail(e),
        };
        let key = Run::<S>::key(&format!("{}/{}", t.cell.ns, t.cell.fn_name), &plain_args(&t.cell.args));
        run.overrides.insert(key, v);
    }
    let b = match run.built(&name, &hash) {
        Ok(b) => b,
        Err(RunError(e)) => return fail(e),
    };
    if b.compiled.is_none() {
        return fail(format!("{name} is stored: its tests are not run"));
    }
    let mut coverage = Coverage::new();
    let r = run.evaluate(&b, &args, &asof, &mut coverage);
    // A run that does not answer a value (a fault, or no evaluation) takes no branch arms into
    // account: an arm entered on the way to a fault does not exercise the branch (Build 2 Part 3,
    // E5; the derived cases of the run still count).
    let answered = r.is_ok();
    let (got, error) = match r {
        Ok(o) => (Ok(o.value), None),
        Err(Stop::Code(c)) => (Err(c.as_str().to_string()), None),
        Err(Stop::World(RunError(e))) => (Err(String::new()), Some(e)),
    };
    let coverage = if answered { coverage } else { Coverage::new() };
    Ran { args, got, error, coverage, events: run.events }
}

/// Whether the test's expectation holds for the run; the detail says what came out otherwise.
pub fn check(spec: &TestSpec, ran: &Ran) -> Result<(), String> {
    match (&spec.expect, &ran.got) {
        (Expect::Value(want), Ok(got)) if want == got => Ok(()),
        (Expect::Fault(want), Err(got)) if want == got && !got.is_empty() => Ok(()),
        (want, got) => {
            let out = match got {
                Ok(v) => v.lexical().to_string(),
                Err(c) if c.is_empty() => ran.error.clone().unwrap_or_else(|| "no value".into()),
                Err(c) => c.clone(),
            };
            Err(format!("expected {}, got {out}", expect_text(want)))
        }
    }
}

/// Is `s` a number equal to zero.
fn is_zero(s: &Scalar) -> bool {
    let t = s.lexical().trim_start_matches('-');
    t.chars().any(|c| c.is_ascii_digit()) && t.chars().all(|c| c == '0' || c == '.')
}

/// Is `s` a negative number.
fn is_negative(s: &Scalar) -> bool {
    s.lexical().starts_with('-') && !is_zero(s)
}

/// A derived case the public rule asks for, by its text: `zero(units)`, `absent(<name>)`, ...
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Case(pub String);

/// What a function's tests cover: the derived cases, and its branch arms.
#[derive(Debug, Default)]
pub struct Covered {
    /// The derived cases no test exercises, ascending.
    pub uncovered: Vec<Case>,
    /// The arms no test takes, as text (`if #12 arm 1`), ascending.
    pub unexercised: Vec<String>,
    /// The arms of the body and the arms the tests take.
    pub branches: u32,
    pub covered: u32,
}

/// The derived cases of a compiled body (all of them, from its facts), each with whether `ran`
/// exercises it.
fn derived(compiled: &Compiled, sig: &datumesh_formula::Signature, ran: Option<&Ran>) -> Vec<(Case, bool)> {
    let facts = coverage::edge_facts(compiled, sig);
    let hit = |f: &dyn Fn(&Ran) -> bool| ran.is_some_and(f);
    let mut out: Vec<(Case, bool)> = Vec::new();
    for a in &facts.divisors {
        out.push((Case(format!("zero({a})")), hit(&|r| r.args.get(a).is_some_and(is_zero))));
    }
    for a in &facts.number_args {
        out.push((Case(format!("negative({a})")), hit(&|r| r.args.get(a).is_some_and(is_negative))));
    }
    for (a, extent) in &facts.extent_args {
        if let datumesh_formula::Extent::Range(lo, hi) = extent {
            out.push((
                Case(format!("extent_lo({a})")),
                hit(&|r| r.args.get(a).map(|s| s.lexical()) == Some(lo.as_str())),
            ));
            out.push((
                Case(format!("extent_hi({a})")),
                hit(&|r| r.args.get(a).map(|s| s.lexical()) == Some(hi.as_str())),
            ));
        }
    }
    for f in &facts.cell_inputs {
        out.push((Case(format!("absent({f})")), hit(&|r| r.events.contains(&format!("absent:{f}")))));
    }
    for f in &facts.slice_inputs {
        out.push((Case(format!("empty({f})")), hit(&|r| r.events.contains(&format!("empty:{f}")))));
    }
    out
}

/// Every derived case of a function, ascending, each with whether one of `runs` exercises it.
pub(crate) fn cases_of(compiled: &Compiled, sig: &datumesh_formula::Signature, runs: &[&Ran]) -> Vec<(Case, bool)> {
    let mut cases: BTreeMap<Case, bool> = BTreeMap::new();
    for (c, _) in derived(compiled, sig, None) {
        cases.insert(c, false);
    }
    for r in runs {
        for (c, ok) in derived(compiled, sig, Some(r)) {
            let e = cases.entry(c).or_insert(false);
            *e = *e || ok;
        }
    }
    cases.into_iter().collect()
}

/// The coverage of a function's tests: its derived cases (of `runs`), and its branch arms.
pub fn coverage_of(compiled: &Compiled, sig: &datumesh_formula::Signature, runs: &[&Ran]) -> Covered {
    let cases: BTreeMap<Case, bool> = cases_of(compiled, sig, runs).into_iter().collect();
    let mut arms = Coverage::new();
    for r in runs {
        arms.union(&r.coverage);
    }
    let points = coverage::branches(&compiled.ast);
    let unexercised = arms
        .missing(&points)
        .into_iter()
        .map(|(id, arm)| {
            let kind = points.iter().find(|p| p.id == id).map(|p| p.kind);
            let what = match (kind, arm) {
                (Some(BranchKind::If), 0) => "then",
                (Some(BranchKind::If), _) => "else",
                (Some(BranchKind::Otherwise), 0) => "value",
                (Some(BranchKind::Otherwise), _) => "fallback",
                (None, _) => "arm",
            };
            format!("{} #{id} {what}", if kind == Some(BranchKind::Otherwise) { "otherwise" } else { "if" })
        })
        .collect();
    Covered {
        uncovered: cases.iter().filter(|(_, ok)| !**ok).map(|(c, _)| c.clone()).collect(),
        unexercised,
        branches: Coverage::total(&points) as u32,
        covered: arms.covered(&points) as u32,
    }
}

/// The result of a test as a store row says it (`pass`, `fail`, `error`).
pub fn result_of(spec: &TestSpec, ran: &Ran) -> (&'static str, String) {
    if ran.error.is_some() {
        return ("error", ran.error.clone().unwrap_or_default());
    }
    match check(spec, ran) {
        Ok(()) => ("pass", String::new()),
        Err(d) => ("fail", d),
    }
}

/// The held tests of `hash`, parsed (rows whose JCS does not parse are skipped).
pub fn held_tests<S: Store + ?Sized>(store: &S, hash: &str) -> Result<Vec<(FunctionTest, TestSpec)>, String> {
    let mut out = Vec::new();
    for row in store.function_tests(hash).map_err(|e| e.to_string())? {
        let Ok(v) = json::parse(&row.test_json) else { continue };
        let Ok(spec) = parse_test(&v) else { continue };
        out.push((row, spec));
    }
    Ok(out)
}

/// A store write a test run makes: the row (inserted, or its result updated) and the branch counts
/// of its function.
#[derive(Clone, Debug)]
pub struct RowWrite {
    pub fn_hash: String,
    pub case: String,
    pub jcs: Vec<u8>,
    pub signed_by: String,
    pub result: &'static str,
    pub detail: String,
    pub branches_total: u32,
    pub branches_covered: u32,
}

/// The writes that bring the store's rows to `rows`: an insert for a new row, a record for a row
/// whose result changed, and the branch counts when they changed. Returns how many rows changed.
pub fn apply_rows<S: Store + ?Sized>(store: &S, rows: &[RowWrite], run_at: &str) -> Result<usize, String> {
    let mut changed = 0;
    let mut branches: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for r in rows {
        let held = store.function_tests(&r.fn_hash).map_err(|e| e.to_string())?;
        match held.iter().find(|h| h.case == r.case) {
            None => {
                let mut t = FunctionTest::new(&r.fn_hash, &r.case, &r.jcs, &r.signed_by);
                t.last_run_at = Some(run_at.to_string());
                t.last_result = Some(r.result.to_string());
                t.last_detail = Some(r.detail.clone()).filter(|d| !d.is_empty());
                t.branches_total = r.branches_total;
                t.branches_covered = r.branches_covered;
                store.put_function_test(&t).map_err(|e| e.to_string())?;
                changed += 1;
            }
            Some(h) => {
                let detail = Some(r.detail.clone()).filter(|d| !d.is_empty());
                if h.last_result.as_deref() != Some(r.result) || h.last_detail != detail {
                    store
                        .record_function_test(&r.fn_hash, &r.case, run_at, r.result, &r.detail)
                        .map_err(|e| e.to_string())?;
                    changed += 1;
                }
            }
        }
        branches.insert(r.fn_hash.clone(), (r.branches_total, r.branches_covered));
    }
    for (hash, (total, covered)) in branches {
        let held = store.function_tests(&hash).map_err(|e| e.to_string())?;
        let differs = held.first().is_some_and(|h| h.branches_total != total || h.branches_covered != covered);
        if differs {
            store.set_function_branches(&hash, total, covered).map_err(|e| e.to_string())?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Reruns the tests of every function that reads one of `names`, transitively, and writes the
/// results (CONTRACT-build2-part3 E5, rerun on rebind). Nothing is refused: a failure is recorded.
/// A function is rerun only while its hash is the one its name is bound to. Returns the functions
/// reached.
pub fn rerun_readers<S: Store + ?Sized>(store: &S, names: &[String], run_at: &str) -> Result<usize, String> {
    let overlay = Overlay::default();
    let mut todo: Vec<String> = names.to_vec();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut reached = 0;
    while let Some(name) = todo.pop() {
        for hash in store.function_readers(&name).map_err(|e| e.to_string())? {
            if !seen.insert(hash.clone()) {
                continue;
            }
            reached += 1;
            todo.extend(store.bound_names(&hash).map_err(|e| e.to_string())?);
            let held = held_tests(store, &hash)?;
            let Some((_, first)) = held.first() else { continue };
            let fname = first.function.clone();
            if store.resolve_name(&fname, run_at).map_err(|e| e.to_string())?.as_deref() != Some(hash.as_str()) {
                continue;
            }
            let mut run = Run::new(store, &overlay, run_at);
            let Ok(b) = run.built(&fname, &hash) else { continue };
            let Some(c) = &b.compiled else { continue };
            let ran: Vec<(TestSpec, Ran)> =
                held.iter().map(|(_, s)| (s.clone(), run_test(store, &overlay, run_at, s))).collect();
            let runs: Vec<&Ran> = ran.iter().map(|(_, r)| r).collect();
            let cov = coverage_of(c, &b.function.signature, &runs);
            let rows: Vec<RowWrite> = ran
                .iter()
                .map(|(s, r)| {
                    let (result, detail) = result_of(s, r);
                    RowWrite {
                        fn_hash: hash.clone(),
                        case: s.case.clone(),
                        jcs: s.jcs.clone(),
                        signed_by: String::new(),
                        result,
                        detail,
                        branches_total: cov.branches,
                        branches_covered: cov.covered,
                    }
                })
                .collect();
            apply_rows(store, &rows, run_at)?;
        }
    }
    Ok(reached)
}

/// The counts of a function's held tests, for `describe`: `(count, branches, covered,
/// counterexamples)`, where a counterexample is a failing (or erroring) test.
pub fn counts<S: Store + ?Sized>(store: &S, hash: &str) -> Result<(u64, u64, u64, u64), String> {
    let rows = store.function_tests(hash).map_err(|e| e.to_string())?;
    let count = rows.len() as u64;
    let (branches, covered) = rows.first().map_or((0, 0), |r| (r.branches_total as u64, r.branches_covered as u64));
    let counterexamples =
        rows.iter().filter(|r| matches!(r.last_result.as_deref(), Some("fail") | Some("error"))).count() as u64;
    Ok((count, branches, covered, counterexamples))
}

/// Puts the test-case function `<host>/test_case(function_hash, case) -> boolean` (E5, mesh-test) in
/// the store, bound since the start of time, unless it is bound already. Its cells are computed by
/// [`TestCases`].
pub fn install_test_cases(store: &dyn Store, host: &str) -> Result<(), String> {
    let name = format!("{host}/{TEST_CASE_NAME}");
    if store.resolve_name(&name, "9999-12-31T23:59:59.999999Z").map_err(|e| e.to_string())?.is_some() {
        return Ok(());
    }
    let s = |x: &str| Value::String(x.to_string());
    let arg = |t: &str| Value::Object(vec![("type".into(), s(t)), ("valid".into(), Value::Bool(false))]);
    let v = Value::Object(vec![
        ("code".into(), Value::Object(vec![("body".into(), s("")), ("lang".into(), s(TEST_CASE_LANG))])),
        ("inputs".into(), Value::Array(vec![])),
        (
            "signature".into(),
            Value::Object(vec![
                (
                    "args".into(),
                    Value::Object(vec![(TEST_CASE_HASH_ARG.into(), arg("string")), ("case".into(), arg("string"))]),
                ),
                ("order".into(), Value::Array(vec![s(TEST_CASE_HASH_ARG), s("case")])),
                ("returns".into(), s("boolean")),
            ]),
        ),
    ]);
    let bytes = v.to_jcs();
    let d = FunctionDescriptor::from_json(&bytes).map_err(|e| e.to_string())?;
    store.put_descriptor(&bytes).map_err(|e| e.to_string())?;
    store.bind_name(&name, &d.hash(), TEST_CASE_SINCE).map_err(|e| e.to_string())
}

/// When the test-case function is bound from (before any test cell's asof).
const TEST_CASE_SINCE: &str = "2000-01-01T00:00:00.000000Z";

/// The test-case runtime (E5, mesh-test): `<host>/test_case(function_hash, case) -> boolean` is
/// `true` when the function under test, run as the held test says, gives what the test expects.
pub struct TestCases {
    pub store: Arc<dyn Store + Send + Sync>,
}

impl Runtime for TestCases {
    fn lang(&self) -> &str {
        TEST_CASE_LANG
    }

    fn evaluate(
        &self,
        _d: &FunctionDescriptor,
        cell: &Cell,
        inputs: &mut dyn Inputs,
        budget: &mut Budget,
    ) -> Result<Scalar, EvalError> {
        budget.charge(1)?;
        let text = |k: &str| match cell.args.get(k) {
            Some(Arg::Value(s)) => Ok(s.lexical().to_string()),
            _ => Err(EvalError::Runtime(format!("the test case has no {k}"))),
        };
        let hash = text(TEST_CASE_HASH_ARG)?;
        let case = text("case")?;
        let asof = inputs.asof();
        let held = held_tests(&*self.store, &hash).map_err(EvalError::Runtime)?;
        let Some((_, spec)) = held.into_iter().find(|(_, s)| s.case == case) else {
            return Err(EvalError::Runtime(format!("no test {case} of {hash}")));
        };
        let overlay = Overlay::default();
        let ran = run_test(&*self.store, &overlay, &asof, &spec);
        let ok = ran.error.is_none() && check(&spec, &ran).is_ok();
        Scalar::from_lexical(Type::Boolean, if ok { "true" } else { "false" })
            .map_err(|e| EvalError::Runtime(e.to_string()))
    }
}
