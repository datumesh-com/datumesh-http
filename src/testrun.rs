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
use datumesh_store::{FunctionTest, Store, TestSummary};

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
        for (call, entry) in i.as_object().ok_or("a test's inputs is an object")? {
            if entry.get("withdrawn").is_some() {
                withdrawn_shape(entry).map_err(|e| format!("input {call}: {e}"))?;
            } else if entry.get("type").is_some() {
                typed_scalar(entry).map_err(|e| format!("input {call}: {e}"))?;
            } else {
                slice_override_shape(entry).map_err(|e| format!("input {call}: {e}"))?;
            }
            inputs.push((call.clone(), entry.clone()));
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

/// A typed scalar `{"type", "value"}` of the JSON object `v`. A boolean's value is the JSON `true` or `false`
/// (CONTRACT-classes-round K3, as a cell line writes it); the other types' values are their lexical strings.
pub fn typed_scalar(v: &Value) -> Result<Scalar, String> {
    let ty = v.get("type").and_then(Value::as_str).ok_or("a typed scalar has a type")?;
    let ty = Type::from_name(ty).ok_or_else(|| format!("unknown type {ty}"))?;
    let lexical = if ty == Type::Boolean {
        match v.get("value") {
            Some(Value::Bool(b)) => if *b { "true" } else { "false" }.to_string(),
            _ => return Err("a boolean's value is the JSON true or false".into()),
        }
    } else {
        v.get("value").and_then(Value::as_str).ok_or("a typed scalar has a value")?.to_string()
    };
    Scalar::from_lexical(ty, &lexical).map_err(|e| e.to_string())
}

/// A test input that withdraws a cell (R1): `{"withdrawn": "<reason>"}`, a non-empty string under the string rules, and
/// nothing else. The cell has no value in the run, and is not a member of a slice.
fn withdrawn_shape(v: &Value) -> Result<(), String> {
    let obj = v.as_object().ok_or("a withdrawn input is an object")?;
    if obj.len() != 1 {
        return Err("a withdrawn input has the member withdrawn and no other".into());
    }
    match v.get("withdrawn") {
        Some(Value::String(text)) if !text.is_empty() && text.len() <= 1024 && !text.chars().any(char::is_control) => {
            Ok(())
        }
        _ => Err("withdrawn is a non-empty string of at most 1024 bytes without control characters".into()),
    }
}

/// The shape of a slice override in a test's `inputs` (CONTRACT-classes-round K1): `{"add"?: {<cell call>: <typed
/// scalar>, ...}, "remove"?: [<cell call>, ...]}`, at least one member; or `{"members": {<cell call>: <typed scalar>, ...}}`
/// (R3), which replaces the slice's whole member set and excludes `add` and `remove`. Whether the cells are members of the
/// slice is checked by the run.
fn slice_override_shape(v: &Value) -> Result<(), String> {
    let obj = v.as_object().ok_or("an input is a typed scalar or a slice override")?;
    if obj.is_empty() {
        return Err("a slice override has neither add nor remove".into());
    }
    if obj.iter().any(|(k, _)| k == "members") && obj.len() > 1 {
        return Err("E_SLICE_MEMBER: members excludes add and remove".into());
    }
    for (k, x) in obj {
        match (k.as_str(), x) {
            ("members", Value::Object(cells)) => {
                for (call, typed) in cells {
                    typed_scalar(typed).map_err(|e| format!("members {call}: {e}"))?;
                }
            }
            ("members", _) => return Err("E_SLICE_MEMBER: members is an object of cell calls to typed scalars".into()),
            ("add", Value::Object(cells)) => {
                for (call, typed) in cells {
                    typed_scalar(typed).map_err(|e| format!("add {call}: {e}"))?;
                }
            }
            ("remove", Value::Array(calls)) => {
                if calls.iter().any(|c| c.as_str().is_none()) {
                    return Err("remove is a list of cell calls".into());
                }
            }
            _ => return Err(format!("a slice override has no member {k} of that shape")),
        }
    }
    Ok(())
}

/// A slice override of a test run (K1): the members a slice read gives are the store's, minus `remove`, plus `add`.
#[derive(Clone, Debug)]
pub struct SliceOverride {
    /// The slice the override is for, as the read names it.
    pub slice: SliceKey,
    /// The cells added, by their arguments.
    pub add: Vec<(Args, Scalar)>,
    /// The cells removed, by their arguments.
    pub remove: Vec<Args>,
    /// The whole member set, when the override replaces it (R3, `members`): then `add` and `remove` are empty and
    /// the stored members are not seen.
    pub members: Option<Vec<(Args, Scalar)>>,
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
    /// The value; `None` for a withdrawal (R1), which has no value from its tx on.
    pub value: Option<Scalar>,
}

/// The value a stored receipt or withdrawal gives: `Some` for a receipt, `None` for a withdrawal (R1).
pub(crate) fn version_value(bytes: &[u8]) -> Result<Option<Scalar>, String> {
    match Receipt::from_json(bytes) {
        Ok(r) => Ok(Some(r.value().clone())),
        Err(e) => match datumesh_core::Withdrawal::from_json(bytes) {
            Ok(_) => Ok(None),
            Err(_) => Err(e.to_string()),
        },
    }
}

impl Overlay {
    /// Records a planned receipt (its JSON as the run signs it).
    pub fn cell(&mut self, receipt: &[u8]) -> Result<(), String> {
        if let Ok(r) = Receipt::from_json(receipt) {
            let cell = r.cell();
            self.cells.push(PlannedCell {
                name: format!("{}/{}", cell.ns, cell.fn_name),
                id: cell.id(),
                tx: r.tx().to_string(),
                receipt_hash: r.hash().to_string(),
                args: plain_args(&cell.args),
                value: Some(r.value().clone()),
            });
            return Ok(());
        }
        let w = datumesh_core::Withdrawal::from_json(receipt).map_err(|e| e.to_string())?;
        let cell = w.cell();
        self.cells.push(PlannedCell {
            name: format!("{}/{}", cell.ns, cell.fn_name),
            id: cell.id(),
            tx: w.tx().to_string(),
            receipt_hash: w.hash().to_string(),
            args: plain_args(&cell.args),
            value: None,
        });
        Ok(())
    }
}

/// The value arguments of a core argument map.
pub(crate) fn plain_args(args: &CoreArgs) -> Args {
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
pub(crate) fn cell_id(name: &str, hash: &str, args: &Args) -> String {
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
    /// The cells the test's inputs withdraw (R1), by key: they have no value and are no slice's members.
    withdrawn: BTreeSet<String>,
    slice_overrides: Vec<SliceOverride>,
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
            withdrawn: BTreeSet::new(),
            slice_overrides: Vec::new(),
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
            return Ok(p.value.clone());
        }
        let Some(v) = self.store.version_asof(id, t).map_err(|e| RunError(e.to_string()))? else {
            return Ok(None);
        };
        let bytes = self.store.receipt(&v.receipt_hash).ok_or_else(|| RunError("a version has no receipt".into()))?;
        version_value(&bytes).map_err(RunError)
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
        // At most the fuel left plus one is read; more is the budget (CONTRACT-round-0.7-P HP8).
        let remaining = FUEL.saturating_sub(self.used) as usize;
        let values = self
            .store
            .slice_values_limit(&core_slice, &t, remaining.saturating_add(1))
            .map_err(|e| RunError(e.to_string()))?;
        if values.len() > remaining {
            return Err(RunError("E_EVAL_BUDGET: the slice is larger than the fuel left".into()));
        }
        for sv in values {
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
                match &p.value {
                    Some(value) => {
                        let ver = Version {
                            value: value.clone(),
                            receipt_hash: p.receipt_hash.clone(),
                            tx: p.tx.clone(),
                            asof: p.tx.clone(),
                        };
                        members.insert(p.id.clone(), (p.args.clone(), ver));
                    }
                    // A withdrawal leaves the cell out of the slice (R1)
                    None => {
                        members.remove(&p.id);
                    }
                }
            }
        }
        // A withdrawn input is no member (R1), whatever the store and the file give.
        members.retain(|_, (args, _)| !self.withdrawn.contains(&Run::<S>::key(&slice.name, args)));
        for o in self.slice_overrides.iter().filter(|o| o.slice == *slice) {
            if let Some(whole) = &o.members {
                // R3: the replacement is the whole member set, the stored and the file's members not seen.
                members.clear();
                for (args, value) in whole {
                    let ver =
                        Version { value: value.clone(), receipt_hash: String::new(), tx: t.clone(), asof: t.clone() };
                    members.insert(cell_id(&slice.name, &hash, args), (args.clone(), ver));
                }
                continue;
            }
            for args in &o.remove {
                members.remove(&cell_id(&slice.name, &hash, args));
            }
            for (args, value) in &o.add {
                let ver = Version { value: value.clone(), receipt_hash: String::new(), tx: t.clone(), asof: t.clone() };
                members.insert(cell_id(&slice.name, &hash, args), (args.clone(), ver));
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
        let key = Run::<S>::key(&cell.name, &cell.args);
        if let Some(v) = self.overrides.get(&key) {
            return Ok(Some(Version { value: v.clone(), receipt_hash: String::new(), tx: t.clone(), asof: t }));
        }
        if self.withdrawn.contains(&key) {
            self.events.insert(format!("absent:{}", cell.name));
            return Err(Stop::Code(Code::Absent));
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
    for (call, entry) in &spec.inputs {
        if entry.get("type").is_none() && entry.get("withdrawn").is_none() {
            match slice_override(store, overlay, &asof, call, entry) {
                Ok(o) => run.slice_overrides.push(o),
                Err(e) => return fail(e),
            }
            continue;
        }
        let t = match call::resolve_call(call, &asof, &BTreeMap::new(), &names, false) {
            Ok(t) => t,
            Err(_) => return fail(format!("the input {call} does not resolve")),
        };
        let key = Run::<S>::key(&format!("{}/{}", t.cell.ns, t.cell.fn_name), &plain_args(&t.cell.args));
        if entry.get("withdrawn").is_some() {
            run.withdrawn.insert(key);
            continue;
        }
        let v = match typed_scalar(entry) {
            Ok(v) => v,
            Err(e) => return fail(e),
        };
        run.overrides.insert(key, v);
    }
    let b = match run.built(&name, &hash) {
        Ok(b) => b,
        Err(RunError(e)) => return fail(e),
    };
    if b.compiled.is_none() {
        // A stored function (CONTRACT-build2-part5 F2, spec 7.3): the test reads the stored cell its
        // call names, at the test's asof, and compares that value with `expect`. A cell that is not
        // stored, or cannot be read, is an error: a test that cannot be checked is not a pass.
        let id = cell_id(&name, &hash, &args);
        let (got, error) = match run.stored_value(&id, &asof) {
            Ok(Some(v)) => (Ok(v), None),
            Ok(None) => (Err(String::new()), Some(format!("{} is not stored at {asof}", spec.call))),
            Err(RunError(e)) => (Err(String::new()), Some(e)),
        };
        return Ran { args, got, error, coverage: Coverage::new(), events: run.events };
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

/// The arguments of the cell a call names, which must be a member of `slice` (K1): the same function, and arguments
/// that the slice's pattern matches.
fn member_args<S: Store + ?Sized>(
    names: &Names<'_, S>,
    asof: &str,
    slice: &SliceKey,
    call: &str,
) -> Result<Args, String> {
    let t = call::resolve_call(call, asof, &BTreeMap::new(), names, false)
        .map_err(|_| format!("the cell {call} does not resolve"))?;
    let args = plain_args(&t.cell.args);
    if format!("{}/{}", t.cell.ns, t.cell.fn_name) != slice.name || !slice.matches(&args) {
        return Err(format!("E_SLICE_MEMBER: the cell {call} is not a member of the slice {}", slice.name));
    }
    Ok(args)
}

/// The slice override of a test's `inputs` entry (K1): the slice call, its `add` (typed cells) and its `remove`.
fn slice_override<S: Store + ?Sized>(
    store: &S,
    overlay: &Overlay,
    asof: &str,
    call: &str,
    entry: &Value,
) -> Result<SliceOverride, String> {
    let names = Names { store, overlay };
    let t = call::resolve_call(call, asof, &BTreeMap::new(), &names, true)
        .map_err(|_| format!("the slice {call} does not resolve"))?;
    let pattern: BTreeMap<String, PatternArg> = t
        .cell
        .args
        .iter()
        .map(|(k, a)| {
            let p = match a {
                Arg::Value(s) => PatternArg::Value(s.clone()),
                Arg::Slice => PatternArg::Open,
            };
            (k.clone(), p)
        })
        .collect();
    let slice = SliceKey { name: format!("{}/{}", t.cell.ns, t.cell.fn_name), args: pattern };
    let mut add = Vec::new();
    let mut remove = Vec::new();
    let mut members = None;
    for (k, x) in entry.as_object().ok_or("a slice override is an object")? {
        match (k.as_str(), x) {
            ("members", Value::Object(cells)) => {
                let mut whole = Vec::new();
                for (cell, typed) in cells {
                    let args = member_args(&names, asof, &slice, cell)?;
                    let v = typed_scalar(typed).map_err(|e| format!("members {cell}: {e}"))?;
                    whole.push((args, v));
                }
                members = Some(whole);
            }
            ("add", Value::Object(cells)) => {
                for (cell, typed) in cells {
                    let args = member_args(&names, asof, &slice, cell)?;
                    let v = typed_scalar(typed).map_err(|e| format!("add {cell}: {e}"))?;
                    add.push((args, v));
                }
            }
            ("remove", Value::Array(cells)) => {
                for cell in cells {
                    let cell = cell.as_str().ok_or("remove is a list of cell calls")?;
                    remove.push(member_args(&names, asof, &slice, cell)?);
                }
            }
            _ => return Err(format!("a slice override has no member {k} of that shape")),
        }
    }
    if members.is_some() && (!add.is_empty() || !remove.is_empty()) {
        return Err("E_SLICE_MEMBER: members excludes add and remove".into());
    }
    Ok(SliceOverride { slice, add, remove, members })
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

/// The held tests of `hash`, parsed (rows whose JCS does not parse are skipped). Reads every row of the
/// hash: a publish reads it, a page does not (see [`summary_of`] and [`rows_page`]).
pub fn held_tests<S: Store + ?Sized>(store: &S, hash: &str) -> Result<Vec<(FunctionTest, TestSpec)>, String> {
    let mut out = Vec::new();
    for row in store.function_tests(hash).map_err(|e| e.to_string())? {
        let Ok(v) = json::parse(&row.test_json) else { continue };
        let Ok(spec) = parse_test(&v) else { continue };
        out.push((row, spec));
    }
    Ok(out)
}

/// The name of the function a test's `call` names. Stored functions have only their signature as
/// descriptor, so two of them with one signature share a function hash, and with it their rows in
/// `function_tests` (QUESTIONS-build2 D23): a function's own tests are the held tests whose call
/// names it. The call is read with the call parser; a call that does not resolve (its name since
/// unbound) is read as the text before its first `(`.
pub(crate) fn call_function<S: Store + ?Sized>(store: &S, now: &str, spec: &TestSpec) -> String {
    resolve_call_of(store, now, spec).map_or_else(|| text_name(&spec.call), |(name, _)| name)
}

/// The text before the first `(` of a call: the name a call that does not resolve is read as.
fn text_name(call: &str) -> String {
    call.split('(').next().unwrap_or("").trim().to_string()
}

/// The function a test's call names and the id of the cell it reads, as the store reads them now.
/// `None` when the call does not resolve.
fn resolve_call_of<S: Store + ?Sized>(store: &S, now: &str, spec: &TestSpec) -> Option<(String, String)> {
    let overlay = Overlay::default();
    let names = Names { store, overlay: &overlay };
    let asof = spec.asof.clone().unwrap_or_else(|| now.to_string());
    let t = call::resolve_call(&spec.call, &asof, &BTreeMap::new(), &names, false).ok()?;
    Some((format!("{}/{}", t.cell.ns, t.cell.fn_name), t.cell.id()))
}

/// The held tests of the function `name` (bound to `hash`), parsed, each with the id of the cell its call
/// reads (empty when the call does not resolve). Publish-time reads: every row of the hash is looked at.
pub(crate) fn held_of_name<S: Store + ?Sized>(
    store: &S,
    hash: &str,
    name: &str,
    now: &str,
) -> Result<Vec<(FunctionTest, TestSpec, String)>, String> {
    let mut out = Vec::new();
    for (row, spec) in held_tests(store, hash)? {
        let (fname, cell) =
            resolve_call_of(store, now, &spec).unwrap_or_else(|| (text_name(&spec.call), String::new()));
        if fname == name {
            out.push((row, spec, cell));
        }
    }
    Ok(out)
}

/// Adds one row to a summary.
fn accumulate(s: &mut TestSummary, row: &FunctionTest) {
    s.count += 1;
    match row.last_result.as_deref() {
        Some("pass") => s.pass += 1,
        Some("fail") => s.fail += 1,
        Some("error") => s.error += 1,
        _ => s.not_run += 1,
    }
    if let Some(at) = &row.last_run_at {
        if s.last_run_at.as_ref().is_none_or(|latest| at > latest) {
            s.last_run_at = Some(at.clone());
        }
    }
    s.branches_total = s.branches_total.max(row.branches_total);
    s.branches_covered = s.branches_covered.max(row.branches_covered);
}

/// Every row of `hash` that belongs to the function `name`, in case order, handed to `f` until it
/// returns `false`. Whether a page reads the tests by the name their rows carry (`unnamed` false, one
/// indexed page at a time) or by the call of each row (`unnamed` true) depends on whether the hash still has
/// rows no one has named: the naming thread of the mesh names the old rows once, and until a hash's rows are
/// named its pages read this way (a transitional cost, on the pages of that hash only).
fn scan_rows<S: Store + ?Sized>(
    store: &S,
    hash: &str,
    name: &str,
    unnamed: bool,
    now: &str,
    page_size: usize,
    mut f: impl FnMut(&FunctionTest, Option<&TestSpec>) -> bool,
) -> Result<(), String> {
    let by = if unnamed { None } else { Some(name) };
    let mut after: Option<String> = None;
    loop {
        let page = store.function_tests_page(hash, by, after.as_deref(), page_size).map_err(|e| e.to_string())?;
        let Some(last) = page.last() else { return Ok(()) };
        after = Some(last.case.clone());
        for row in &page {
            let spec = json::parse(&row.test_json).ok().and_then(|v| parse_test(&v).ok());
            if unnamed {
                let Some(spec) = spec.as_ref() else { continue };
                if call_function(store, now, spec) != name {
                    continue;
                }
            }
            if !f(row, spec.as_ref()) {
                return Ok(());
            }
        }
        if page.len() < page_size {
            return Ok(());
        }
    }
}

/// The most rows named in one batch by [`name_old_tests`].
const NAME_BATCH: usize = 1_000;

/// Names the stored tests written before 0.4.9 (CONTRACT-0.4.9 H1): the rows with no name, in batches
/// of [`NAME_BATCH`] (S1), each named by the function its call names. A hash with rows still unnamed is in
/// `inner.unnamed` while the pass runs, so its pages read its tests by call meanwhile; the pass ends by clearing
/// the set. Runs once, on the naming thread of the mesh; the rows written from now on are named when written.
pub(crate) fn name_old_tests(inner: &crate::mesh::Inner) {
    let store: &dyn Store = &*inner.store;
    loop {
        let batch = match store.unnamed_function_tests(NAME_BATCH) {
            Ok(b) if !b.is_empty() => b,
            Ok(_) => break,
            Err(e) => {
                tracing::warn!(error = %e, "the tests could not be read to name them; they stay unnamed");
                return;
            }
        };
        let now = inner.now();
        {
            let mut unnamed = inner.unnamed.lock().unwrap_or_else(|p| p.into_inner());
            unnamed.extend(batch.iter().map(|row| row.fn_hash.clone()));
        }
        let mut by_hash: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for row in batch {
            let name = match json::parse(&row.test_json).ok().and_then(|v| parse_test(&v).ok()) {
                Some(spec) => call_function(store, &now, &spec),
                // A row whose test does not parse names nothing: it is named as no function, once.
                None => String::new(),
            };
            by_hash.entry(row.fn_hash).or_default().push((row.case, name));
        }
        for (hash, names) in by_hash {
            if let Err(e) = store.name_function_tests(&hash, &names) {
                tracing::warn!(error = %e, "the tests could not be named; they stay unnamed");
                return;
            }
        }
    }
    inner.unnamed.lock().unwrap_or_else(|p| p.into_inner()).clear();
}

/// The summary of the tests of the function `name` (bound to `hash`), from the store's aggregate.
pub(crate) fn summary_of<S: Store + ?Sized>(
    store: &S,
    hash: &str,
    name: &str,
    unnamed: bool,
    now: &str,
) -> Result<TestSummary, String> {
    if !unnamed {
        return store.function_test_summary(hash, Some(name)).map_err(|e| e.to_string());
    }
    let mut s = TestSummary::default();
    scan_rows(store, hash, name, true, now, 1_000, |row, _| {
        accumulate(&mut s, row);
        true
    })?;
    Ok(s)
}

/// The first `limit` tests of the function `name` after `after` (by case), each with its parsed spec.
/// Rows whose JCS does not parse are skipped.
pub(crate) fn rows_page<S: Store + ?Sized>(
    store: &S,
    hash: &str,
    name: &str,
    unnamed: bool,
    now: &str,
    limit: usize,
) -> Result<Vec<(FunctionTest, TestSpec)>, String> {
    let mut out = Vec::new();
    scan_rows(store, hash, name, unnamed, now, limit.max(1), |row, spec| {
        if let Some(spec) = spec {
            out.push((row.clone(), spec.clone()));
        }
        out.len() < limit
    })?;
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
    /// The function the test's call names (the store's name of the row, set when the row is written).
    pub fn_name: String,
}

/// The writes that bring the store's rows to `rows`: an insert for a new row, a record for a row
/// whose result changed, and the branch counts when they changed, and the name of each row (set once).
/// The rows of a hash are read once. Returns how many rows changed.
pub fn apply_rows<S: Store + ?Sized>(store: &S, rows: &[RowWrite], run_at: &str) -> Result<usize, String> {
    let mut changed = 0;
    let mut by_hash: BTreeMap<&str, Vec<&RowWrite>> = BTreeMap::new();
    for r in rows {
        by_hash.entry(r.fn_hash.as_str()).or_default().push(r);
    }
    for (hash, rs) in by_hash {
        let mut held: BTreeMap<String, FunctionTest> =
            store.function_tests(hash).map_err(|e| e.to_string())?.into_iter().map(|t| (t.case.clone(), t)).collect();
        let before_branches = held.values().next().map(|h| (h.branches_total, h.branches_covered));
        let mut names: Vec<(String, String)> = Vec::new();
        for r in &rs {
            let detail = Some(r.detail.clone()).filter(|d| !d.is_empty());
            match held.get(&r.case) {
                None => {
                    let mut t = FunctionTest::new(hash, &r.case, &r.jcs, &r.signed_by);
                    t.last_run_at = Some(run_at.to_string());
                    t.last_result = Some(r.result.to_string());
                    t.last_detail = detail;
                    t.branches_total = r.branches_total;
                    t.branches_covered = r.branches_covered;
                    store.put_function_test(&t).map_err(|e| e.to_string())?;
                    held.insert(r.case.clone(), t);
                    changed += 1;
                }
                Some(h) => {
                    if h.last_result.as_deref() != Some(r.result) || h.last_detail != detail {
                        store
                            .record_function_test(hash, &r.case, run_at, r.result, &r.detail)
                            .map_err(|e| e.to_string())?;
                        changed += 1;
                    }
                }
            }
            names.push((r.case.clone(), r.fn_name.clone()));
        }
        if let Some(last) = rs.last() {
            let target = (last.branches_total, last.branches_covered);
            if before_branches.is_some_and(|b| b != target) {
                store.set_function_branches(hash, target.0, target.1).map_err(|e| e.to_string())?;
                changed += 1;
            }
        }
        store.name_function_tests(hash, &names).map_err(|e| e.to_string())?;
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
            let all = held_tests(store, &hash)?;
            let Some((_, first)) = all.first() else { continue };
            let fname = call_function(store, run_at, first);
            if store.resolve_name(&fname, run_at).map_err(|e| e.to_string())?.as_deref() != Some(hash.as_str()) {
                continue;
            }
            let held = held_of_name(store, &hash, &fname, run_at)?;
            let mut run = Run::new(store, &overlay, run_at);
            let Ok(b) = run.built(&fname, &hash) else { continue };
            let Some(c) = &b.compiled else { continue };
            let ran: Vec<(TestSpec, Ran)> =
                held.iter().map(|(_, s, _)| (s.clone(), run_test(store, &overlay, run_at, s))).collect();
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
                        fn_name: fname.clone(),
                    }
                })
                .collect();
            apply_rows(store, &rows, run_at)?;
        }
    }
    Ok(reached)
}

/// Reruns the stored functions' tests that read what a file or bundle wrote (CONTRACT-build2-part5 F2,
/// CONTRACT-0.4.9 H6). `rebound` are the names it rebinds: every test of such a name is rerun. `written`
/// are the (function name, cell id) of the cells it wrote: only the tests whose call reads one of those
/// cells are rerun, the others keep their results and their `last_run_at`. A computed function is not
/// rerun here (its tests are unchanged). Nothing is refused: a failure is recorded. Returns the rows changed.
pub(crate) fn rerun_stored_tests<S: Store + ?Sized>(
    store: &S,
    rebound: &BTreeSet<String>,
    written: &[(String, String)],
    run_at: &str,
) -> Result<usize, String> {
    let overlay = Overlay::default();
    let mut names: BTreeSet<String> = rebound.clone();
    names.extend(written.iter().map(|(n, _)| n.clone()));
    let mut changed = 0;
    for name in &names {
        let Some(hash) = store.resolve_name(name, run_at).map_err(|e| e.to_string())? else { continue };
        let all_of_name = rebound.contains(name);
        let cells: BTreeSet<&str> = written.iter().filter(|(n, _)| n == name).map(|(_, id)| id.as_str()).collect();
        let selected: Vec<(FunctionTest, TestSpec, String)> = held_of_name(store, &hash, name, run_at)?
            .into_iter()
            .filter(|(_, _, cell)| all_of_name || cells.contains(cell.as_str()))
            .collect();
        if selected.is_empty() {
            continue;
        }
        let mut run = Run::new(store, &overlay, run_at);
        let Ok(b) = run.built(name, &hash) else { continue };
        if b.compiled.is_some() {
            continue;
        }
        let rows: Vec<RowWrite> = selected
            .iter()
            .map(|(_, s, _)| {
                let (result, detail) = result_of(s, &run_test(store, &overlay, run_at, s));
                RowWrite {
                    fn_hash: hash.clone(),
                    case: s.case.clone(),
                    jcs: s.jcs.clone(),
                    signed_by: String::new(),
                    result,
                    detail,
                    branches_total: 0,
                    branches_covered: 0,
                    fn_name: name.clone(),
                }
            })
            .collect();
        changed += apply_rows(store, &rows, run_at)?;
    }
    Ok(changed)
}

/// Most counterexamples a page or a describe lists or counts from foreign test cells (CONTRACT-round-0.7-P HP5).
pub(crate) const COUNTEREXAMPLES_SHOWN: usize = 100;

/// The counts of the tests of the function `name` (hash `hash`), for `describe` and the function page, from
/// the store's aggregate: `(count, branches, covered, counterexamples)`. A counterexample is an open one
/// ([`counterexamples`]): a failing or erroring test of this mesh, or a `false` test cell of any host for
/// the function's hash (spec 7.3, 16.8). Runs nothing; the foreign ones are counted to
/// [`COUNTEREXAMPLES_SHOWN`].
pub fn counts<S: Store + ?Sized>(
    store: &S,
    host: &str,
    name: &str,
    hash: &str,
    unnamed: bool,
    now: &str,
) -> Result<(u64, u64, u64, u64), String> {
    let s = summary_of(store, hash, name, unnamed, now)?;
    let open = counterexamples(store, host, name, hash, unnamed, now, &s, false)?;
    Ok((s.count, s.branches_total as u64, s.branches_covered as u64, open.number()))
}

/// An open counterexample of a function (spec 7.3): who says the function gives a wrong result,
/// and the 16-hex case id of the test that says so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counterexample {
    /// The host that states it: this mesh for its own failing tests, the cell's host otherwise.
    pub signer: String,
    /// The case id (16 lowercase hex digits).
    pub case: String,
}

/// The open counterexamples of a function, bounded (CONTRACT-round-0.7-P HP5): the stored failing and
/// erroring tests of this mesh, counted exactly from the counter and listed to [`COUNTEREXAMPLES_SHOWN`]; then the
/// `false` `<host>/test_case` cells of every host held for the hash, listed (and counted) to
/// [`COUNTEREXAMPLES_SHOWN`], with `foreign_more` when there are more.
pub(crate) struct Counterexamples {
    /// The stored failing and erroring tests (exact).
    pub stored: u64,
    /// The first of them by case, at most [`COUNTEREXAMPLES_SHOWN`].
    pub stored_list: Vec<Counterexample>,
    /// The foreign `false` test cells, at most [`COUNTEREXAMPLES_SHOWN`].
    pub foreign: Vec<Counterexample>,
    /// Whether there are more foreign ones than are listed.
    pub foreign_more: bool,
}

impl Counterexamples {
    /// The number of counterexamples: the stored count and the foreign ones read (see `foreign_more`).
    pub(crate) fn number(&self) -> u64 {
        self.stored + self.foreign.len() as u64
    }

    /// The listed ones: the stored, then the foreign.
    pub(crate) fn listed(&self) -> impl Iterator<Item = &Counterexample> {
        self.stored_list.iter().chain(self.foreign.iter())
    }
}

/// The open counterexamples of the function `name` (hash `hash`), given its summary `s` (read once by the
/// caller). The stored ones are read by `function_tests_failing` (a SQL limit) when the function's rows are named;
/// the foreign ones by `false_test_cells`, one query. Nothing is run.
#[allow(clippy::too_many_arguments)]
pub(crate) fn counterexamples<S: Store + ?Sized>(
    store: &S,
    host: &str,
    name: &str,
    hash: &str,
    unnamed: bool,
    now: &str,
    s: &TestSummary,
    list: bool,
) -> Result<Counterexamples, String> {
    let stored = s.fail + s.error;
    let mut stored_list = Vec::new();
    if stored > 0 && list {
        if unnamed {
            scan_rows(store, hash, name, true, now, 1_000, |row, _| {
                if matches!(row.last_result.as_deref(), Some("fail") | Some("error")) {
                    stored_list.push(Counterexample { signer: host.to_string(), case: row.case.clone() });
                }
                stored_list.len() < COUNTEREXAMPLES_SHOWN
            })?;
        } else {
            let rows =
                store.function_tests_failing(hash, Some(name), COUNTEREXAMPLES_SHOWN).map_err(|e| e.to_string())?;
            stored_list.extend(rows.into_iter().map(|t| Counterexample { signer: host.to_string(), case: t.case }));
        }
    }
    let (foreign, foreign_more) = foreign_counterexamples(store, name, hash)?;
    Ok(Counterexamples { stored, stored_list, foreign, foreign_more })
}

/// The `false` `<host>/test_case` cells of every host this mesh holds for `hash`, at most
/// [`COUNTEREXAMPLES_SHOWN`] of them, and whether there are more (see [`counterexamples`]).
pub(crate) fn foreign_counterexamples<S: Store + ?Sized>(
    store: &S,
    name: &str,
    hash: &str,
) -> Result<(Vec<Counterexample>, bool), String> {
    let mut out = Vec::new();
    // Spec 7.3 gives a two-argument cell the function hash and the case only, so for stored functions of one
    // signature it applies to each of them; the named form (spec 7.3, 16.8) adds the function's name, and
    // counts only for the function `name` it names. One bounded query, filtered by the named cells' function.
    let cells = store.false_test_cells(hash, COUNTEREXAMPLES_SHOWN + 1).map_err(|e| e.to_string())?;
    let more = cells.len() > COUNTEREXAMPLES_SHOWN;
    for (_, bytes) in cells.iter().take(COUNTEREXAMPLES_SHOWN) {
        let Ok(r) = datumesh_core::Receipt::from_json(bytes) else { continue };
        if let Some(Arg::Value(f)) = r.cell().args.get(FUNCTION_ARG) {
            if f.lexical() != name {
                continue;
            }
        }
        let Some(Arg::Value(case)) = r.cell().args.get("case") else { continue };
        out.push(Counterexample { signer: r.cell().ns.clone(), case: case.lexical().to_string() });
    }
    Ok((out, more))
}

/// The function argument of the named test-case cell: the test's function name (spec 7.3).
const FUNCTION_ARG: &str = "function";

/// The descriptor of `<host>/test_case` (E5, mesh-test; spec 7.3): `case`, `function_hash`, and with
/// `named`, `function` between them, returns boolean. Its code is the mesh's lane, [`TEST_CASE_LANG`].
fn test_case_descriptor(named: bool) -> Value {
    let s = |x: &str| Value::String(x.to_string());
    let arg = |t: &str| Value::Object(vec![("type".into(), s(t)), ("valid".into(), Value::Bool(false))]);
    let mut args = vec![(TEST_CASE_HASH_ARG.into(), arg("string")), ("case".into(), arg("string"))];
    let mut order = vec![s(TEST_CASE_HASH_ARG), s("case")];
    if named {
        args.push((FUNCTION_ARG.into(), arg("string")));
        order = vec![s("case"), s(FUNCTION_ARG), s(TEST_CASE_HASH_ARG)];
    }
    Value::Object(vec![
        ("code".into(), Value::Object(vec![("body".into(), s("")), ("lang".into(), s(TEST_CASE_LANG))])),
        ("inputs".into(), Value::Array(vec![])),
        (
            "signature".into(),
            Value::Object(vec![
                ("args".into(), Value::Object(args)),
                ("order".into(), Value::Array(order)),
                ("returns".into(), s("boolean")),
            ]),
        ),
    ])
}

/// Puts the test-case function `<host>/test_case(case, function, function_hash) -> boolean` (spec 7.3,
/// the named form) in the store, bound since the start of time, unless the name is bound already: a name
/// bound to the two-argument form keeps it (no rebind). Its cells are computed by [`TestCases`].
pub fn install_test_cases(store: &dyn Store, host: &str) -> Result<(), String> {
    let name = format!("{host}/{TEST_CASE_NAME}");
    if store.resolve_name(&name, "9999-12-31T23:59:59.999999Z").map_err(|e| e.to_string())?.is_some() {
        return Ok(());
    }
    let bytes = test_case_descriptor(true).to_jcs();
    let d = FunctionDescriptor::from_json(&bytes).map_err(|e| e.to_string())?;
    store.put_descriptor(&bytes).map_err(|e| e.to_string())?;
    store.bind_name(&name, &d.hash(), TEST_CASE_SINCE).map_err(|e| e.to_string())
}

/// When the test-case function is bound from (before any test cell's asof).
const TEST_CASE_SINCE: &str = "2000-01-01T00:00:00.000000Z";

/// The test-case runtime (E5, mesh-test): `<host>/test_case(function_hash, case) -> boolean` is
/// `true` when the function under test, run as the held test says, gives what the test expects. The
/// named form (spec 7.3) also takes `function`: only the tests whose function is that name are its tests.
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
        // One row by its key (CONTRACT-round-0.7-P HP5): a test cell does not read every test of the function.
        let row = self.store.function_test(&hash, &case).map_err(|e| EvalError::Runtime(e.to_string()))?;
        let mut spec = row
            .and_then(|t| json::parse(&t.test_json).ok())
            .and_then(|v| parse_test(&v).ok())
            .filter(|s| s.case == case);
        if cell.args.contains_key(FUNCTION_ARG) {
            // The named form: only a test whose call names the function `function` names is its test (S1).
            let function = text(FUNCTION_ARG)?;
            spec = spec.filter(|s| call_function(&*self.store, &asof, s) == function);
        }
        let Some(spec) = spec else {
            return Err(EvalError::Runtime(format!("no test {case} of {hash}")));
        };
        let overlay = Overlay::default();
        let ran = run_test(&*self.store, &overlay, &asof, &spec);
        let ok = ran.error.is_none() && check(&spec, &ran).is_ok();
        Scalar::from_lexical(Type::Boolean, if ok { "true" } else { "false" })
            .map_err(|e| EvalError::Runtime(e.to_string()))
    }
}
