//! `POST /x/calc` (Build 2 Part 3 addition; CONTRACT-build2-part3-calc C1 and C3; D27, D30): a
//! formula over cells, evaluated in a scope and answered with its value, its type, the formula and
//! its canonical form, the asof, and the lineage of the cells it read.
//!
//! Nothing is written: no receipt, no version, no binding, no memo entry. The evaluator is the
//! formula crate's ([`eval::evaluate`]) over a [`Run`] that reads the store as a test run does
//! ([`testrun`]), and records each cell it reads. A computed cell is evaluated in the scope, never
//! stored; a stored one is read as its version answers at the asof. The answer is
//! `kind: "calculation"`: it has no receipt and no trust. The receipts of its lineage are the
//! signed ones the mesh serves at `/receipt/<hash>`.
//!
//! The released engine evaluates a bound function only (`Engine::whatif` names a cell of a
//! function in the store), so a formula that is not bound has no engine entry point in 0.5.0; this
//! module is the scope the engine's whatif runs in, over the same store reads.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
use std::time::Instant;

use datumesh_core::json::{self, Value};
use datumesh_core::{Answer, ErrorCode, Fault, Receipt, Scalar, Slice, Type};
use datumesh_formula::ast::{Ast, Node, ReadArg};
use datumesh_formula::definition::Callees;
use datumesh_formula::eval::{self, Args, Binding, CellKey, PatternArg, SliceKey, Stop, Version, World};
use datumesh_formula::{canonical_ast, check_descriptor, define, Code, Compiled, Env, Function, Signature};
use datumesh_store::Store;

use crate::apikeys::Principal;
use crate::config::Rule;
use crate::envelope::{self, Cache, Reply};
use crate::mesh::Inner;
use crate::testrun::{self, Built, Overlay};

/// The most bytes of a formula body (D30: more is `413` `E_TOO_LARGE`).
pub(crate) const MAX_FORMULA: usize = 4096;
/// The fuel of an evaluation: the engine's default (SPEC 15.12).
const FUEL: u64 = 10_000_000;
/// How deep computed cells may nest (as a test run's).
const DEPTH: usize = 200;

/// What stops a calculation other than a formula code.
pub(crate) enum CalcError {
    /// The mesh's budget (`DATUMESH_BUDGET_MS`) ran out while members were read: `E_EVAL_BUDGET`.
    Budget,
    /// A cycle of computed cells: the calls of the path, its first and last the same.
    Cycle(Vec<String>),
    /// Computed cells nest deeper than the evaluator allows.
    Depth,
    /// Something this calculation does not read (a range of a stored slice, the versions of a
    /// computed function).
    Unsupported(String),
    /// The store failed, or a stored receipt is missing.
    Store(String),
    /// A licence or payment rule refuses a function the calculation reads: the answer to give.
    Refused(Reply),
}

/// One cell a calculation read, as the answer's lineage lists it.
pub(crate) struct Entry {
    call: String,
    /// The arguments of the call, as their lexical forms (a slice's members name their keys here).
    args: Args,
    receipt: Option<String>,
    publisher: String,
    trust: String,
}

/// The world of one calculation: the store as of its asof, the scope it is evaluated in, and what
/// it has read.
pub(crate) struct Run<'a> {
    inner: &'a Inner,
    store: &'a (dyn Store + Send + Sync),
    /// Planned writes: none, a calculation writes nothing.
    overlay: Overlay,
    principal: Option<&'a Principal>,
    asof: String,
    deadline: Instant,
    used: u64,
    depth: usize,
    /// The computed cells being evaluated: their cell ids and calls, outermost first.
    stack: Vec<(String, String)>,
    built: BTreeMap<String, Arc<Built>>,
    lineage: Vec<Entry>,
    /// The lineage's keys: a call and its receipt, so that each is listed once.
    seen: HashSet<(String, Option<String>)>,
}

/// The call of a cell: `<name>(<arg>: <literal>, ...)`, as a formula writes it.
fn call_of(name: &str, args: &Args) -> String {
    let parts: Vec<String> = args.iter().map(|(k, v)| format!("{k}: {}", literal(v))).collect();
    format!("{name}({})", parts.join(", "))
}

/// A scalar as a formula literal: a string quoted, the rest as their lexical form.
fn literal(v: &Scalar) -> String {
    match v.ty() {
        Type::String => format!("\"{}\"", v.lexical().replace('\\', "\\\\").replace('"', "\\\"")),
        _ => v.lexical().to_string(),
    }
}

/// The publisher of a function name: its host (the first segment).
fn publisher_of(name: &str) -> &str {
    name.split('/').next().unwrap_or("")
}

/// The store's errors as a world error.
fn store_err(e: impl std::fmt::Display) -> CalcError {
    CalcError::Store(e.to_string())
}

impl<'a> Run<'a> {
    fn new(
        inner: &'a Inner,
        store: &'a (dyn Store + Send + Sync),
        asof: &str,
        principal: Option<&'a Principal>,
    ) -> Run<'a> {
        Run {
            inner,
            store,
            overlay: Overlay::default(),
            principal,
            asof: asof.to_string(),
            deadline: Instant::now() + inner.config.budget,
            used: 0,
            depth: 0,
            stack: Vec::new(),
            built: BTreeMap::new(),
            lineage: Vec::new(),
            seen: HashSet::new(),
        }
    }

    /// Records a read cell, once per call and receipt.
    fn note(&mut self, name: &str, args: &Args, receipt: Option<&str>, trust: &str) {
        let call = call_of(name, args);
        if !self.seen.insert((call.clone(), receipt.map(str::to_string))) {
            return;
        }
        self.lineage.push(Entry {
            call,
            args: args.clone(),
            receipt: receipt.map(str::to_string),
            publisher: publisher_of(name).to_string(),
            trust: trust.to_string(),
        });
    }

    /// The receipt with `hash`, parsed.
    fn receipt(&self, hash: &str) -> Result<Receipt, CalcError> {
        let bytes =
            self.store.receipt(hash).ok_or_else(|| CalcError::Store(format!("receipt {hash} is not stored")))?;
        Receipt::from_json(&bytes).map_err(store_err)
    }

    /// The trust a receipt this mesh serves has (5.5), as the envelope labels it.
    fn trust_of(&self, r: &Receipt) -> String {
        envelope::trust_label_of(r, &self.inner.config.host).as_str().to_string()
    }

    /// The licence or payment refusal of a function the calculation reads, when a rule has one
    /// and the principal does not hold it (the policy `whatif` applies to its target). `cell_id`
    /// is the cell a payment is for (a slice's function name, for a slice).
    fn refusal(&self, name: &str, cell_id: &str) -> Option<CalcError> {
        match self.inner.config.policy.rule(name)? {
            Rule::Principals(ps) => {
                if self.principal.is_some_and(|p| ps.contains(&p.id)) {
                    None
                } else {
                    Some(CalcError::Refused(envelope::answer_reply(&Answer::unlicensed(), Cache::NoStore)))
                }
            }
            Rule::Price { amount, currency } => {
                let authorise = format!("{}/pay/{}", self.inner.config.origin(), cell_id);
                let quote = datumesh_core::Quote { amount: amount.clone(), currency: currency.clone(), authorise };
                let a = Answer::payment_required(quote);
                Some(CalcError::Refused(envelope::answer_reply(&a, Cache::NoStore)))
            }
        }
    }

    /// Fails with the budget once its deadline has passed.
    fn tick(&self) -> Result<(), Code> {
        if Instant::now() > self.deadline {
            Err(Code::Budget)
        } else {
            Ok(())
        }
    }

    /// The compiled function `hash` named `name`: its signature and body, and its compiled form
    /// when it is computed (as a test run compiles it, with its callees bound at the asof).
    fn compile(&mut self, name: &str, hash: &str) -> Result<Arc<Built>, CalcError> {
        if let Some(b) = self.built.get(hash) {
            return Ok(b.clone());
        }
        let bytes = testrun::descriptor_of(self.store, &self.overlay, hash)
            .map_err(CalcError::Store)?
            .ok_or_else(|| CalcError::Store(format!("descriptor {hash} is not stored")))?;
        let v = json::parse(&bytes).map_err(store_err)?;
        let function = Function::from_value(&v).map_err(|e| CalcError::Store(e.0))?;
        let compiled = if function.lang == "formula" {
            let mut env = Env::new(name, function.signature.clone());
            for callee in &function.inputs {
                if callee == name {
                    continue;
                }
                if let Some((h, _)) =
                    testrun::binding_of(self.store, &self.overlay, callee, &self.asof).map_err(CalcError::Store)?
                {
                    let cb = testrun::descriptor_of(self.store, &self.overlay, &h)
                        .map_err(CalcError::Store)?
                        .ok_or_else(|| CalcError::Store(format!("descriptor {h} is not stored")))?;
                    let cv = json::parse(&cb).map_err(store_err)?;
                    env.add_function(callee, &cv).map_err(|e| CalcError::Store(e.0))?;
                }
            }
            Some(check_descriptor(name, &v, &env).map_err(store_err)?)
        } else {
            None
        };
        let b = Arc::new(Built { function: Arc::new(function), compiled });
        self.built.insert(hash.to_string(), b.clone());
        Ok(b)
    }

    /// The members of a stored function's slice as of the asof, each with its receipt.
    fn stored_members(
        &mut self,
        slice: &SliceKey,
        hash: &str,
        ty_of: &Function,
    ) -> Result<Vec<(Args, Version)>, CalcError> {
        let mut obj = BTreeMap::new();
        for (k, p) in &slice.args {
            let v = match p {
                PatternArg::Value(s) => {
                    let ty = ty_of.signature.args.get(k).map(|d| d.ty).unwrap_or(s.ty());
                    Value::Object(vec![
                        ("type".into(), Value::String(ty.as_str().into())),
                        ("value".into(), Value::String(s.lexical().into())),
                    ])
                }
                PatternArg::Open => Value::String("*".into()),
                PatternArg::Range(..) => {
                    return Err(CalcError::Unsupported("a range over a stored slice is not read by calc".into()))
                }
            };
            obj.insert(k.clone(), v);
        }
        let (ns, f) = slice.name.rsplit_once('/').unwrap_or(("", slice.name.as_str()));
        let sv = Value::Object(vec![
            ("ns".into(), Value::String(ns.into())),
            ("fn".into(), Value::String(f.into())),
            ("fn_hash".into(), Value::String(hash.into())),
            ("args".into(), Value::Object(obj.into_iter().collect())),
        ]);
        let core = Slice::from_value(&sv).map_err(store_err)?;
        let t = self.asof.clone();
        // At most the fuel left plus one is read: more members than that is the budget (CONTRACT-round-0.7-P HP8).
        let remaining = FUEL.saturating_sub(self.used) as usize;
        let values = self.store.slice_page(&core, &t, None, remaining.saturating_add(1)).map_err(store_err)?;
        if values.len() > remaining {
            return Err(CalcError::Budget);
        }
        let mut out = Vec::new();
        for sv in values {
            self.tick().map_err(|_| CalcError::Budget)?;
            let args = testrun::plain_args(&sv.cell.args);
            let receipt = sv.version.receipt_hash.clone();
            let r = self.receipt(&receipt)?;
            let trust = self.trust_of(&r);
            self.note(&slice.name, &args, Some(&receipt), &trust);
            let version = Version { value: sv.value, receipt_hash: receipt, tx: sv.version.tx, asof: sv.version.asof };
            out.push((args, version));
        }
        Ok(out)
    }
}

impl World for Run<'_> {
    type Error = CalcError;

    fn function(&mut self, name: &str, at: Option<&str>) -> Result<Option<Binding>, CalcError> {
        let at = at.map_or_else(|| self.asof.clone(), str::to_string);
        let Some((hash, start)) =
            testrun::binding_of(self.store, &self.overlay, name, &at).map_err(CalcError::Store)?
        else {
            return Ok(None);
        };
        let b = self.compile(name, &hash)?;
        Ok(Some(Binding { function: b.function.clone(), start }))
    }

    fn read(&mut self, cell: &CellKey, at: Option<&str>) -> Result<Option<Version>, Stop<CalcError>> {
        self.charge(1)?;
        let t = at.map_or_else(|| self.asof.clone(), str::to_string);
        let Some((hash, _)) = testrun::binding_of(self.store, &self.overlay, &cell.name, &t)
            .map_err(|e| Stop::World(CalcError::Store(e)))?
        else {
            return Err(Stop::Code(Code::Absent));
        };
        let id = testrun::cell_id(&cell.name, &hash, &cell.args);
        if let Some(e) = self.refusal(&cell.name, &id) {
            return Err(Stop::World(e));
        }
        let b = self.compile(&cell.name, &hash).map_err(Stop::World)?;
        let call = call_of(&cell.name, &cell.args);
        let Some(c) = &b.compiled else {
            // A stored cell: its version as of the read's time, with the receipt that answers.
            let Some(v) = self.store.version_asof(&id, &t).map_err(|e| Stop::World(store_err(e)))? else {
                return Err(Stop::Code(Code::Absent));
            };
            let r = self.receipt(&v.receipt_hash).map_err(Stop::World)?;
            let trust = self.trust_of(&r);
            self.note(&cell.name, &cell.args, Some(&v.receipt_hash), &trust);
            return Ok(Some(Version {
                value: r.value().clone(),
                receipt_hash: v.receipt_hash,
                tx: v.tx,
                asof: v.asof,
            }));
        };
        // A computed cell: evaluated in the scope, never stored. Its lineage receipt is the stored
        // one that answers at the time, when that one says the same value.
        if let Some(i) = self.stack.iter().position(|(s, _)| *s == id) {
            let mut path: Vec<String> = self.stack[i..].iter().map(|(_, c)| c.clone()).collect();
            path.push(call);
            return Err(Stop::World(CalcError::Cycle(path)));
        }
        if self.depth >= DEPTH {
            return Err(Stop::World(CalcError::Depth));
        }
        self.stack.push((id.clone(), call));
        self.depth += 1;
        let r = eval::evaluate(c, &b.function, &cell.args, &t, self);
        self.depth -= 1;
        self.stack.pop();
        let out = r?;
        let mut stored = None;
        if let Some(v) = self.store.computed_asof(&id, &t).map_err(|e| Stop::World(store_err(e)))? {
            let r = self.receipt(&v.receipt_hash).map_err(Stop::World)?;
            if r.value() == &out.value {
                stored = Some((v.receipt_hash, self.trust_of(&r)));
            }
        }
        let (receipt, trust) = match stored {
            Some((h, t)) => (Some(h), t),
            None => (None, "computed".to_string()),
        };
        self.note(&cell.name, &cell.args, receipt.as_deref(), &trust);
        Ok(Some(Version { value: out.value, receipt_hash: receipt.unwrap_or_default(), tx: t.clone(), asof: t }))
    }

    fn members(&mut self, slice: &SliceKey) -> Result<Vec<(Args, Version)>, Stop<CalcError>> {
        self.charge(1)?;
        let t = self.asof.clone();
        let Some((hash, _)) = testrun::binding_of(self.store, &self.overlay, &slice.name, &t)
            .map_err(|e| Stop::World(CalcError::Store(e)))?
        else {
            return Ok(Vec::new());
        };
        if let Some(e) = self.refusal(&slice.name, &slice.name) {
            return Err(Stop::World(e));
        }
        let b = self.compile(&slice.name, &hash).map_err(Stop::World)?;
        let members = if b.compiled.is_some() {
            // A computed function's slice: every cell of its extent, each evaluated, each with a value.
            let argsets = eval::extent_cells(&b.function, slice).map_err(Stop::Code)?;
            let mut out = Vec::with_capacity(argsets.len());
            for args in argsets {
                let key = CellKey { name: slice.name.clone(), args: args.clone() };
                let Some(v) = self.read(&key, None)? else { return Err(Stop::Code(Code::Absent)) };
                out.push((args, v));
            }
            out
        } else {
            let f = b.function.clone();
            self.stored_members(slice, &hash, &f).map_err(Stop::World)?
        };
        self.charge(members.len() as u64)?;
        Ok(members)
    }

    fn versions(&mut self, cell: &CellKey, from: &str, to: &str) -> Result<Vec<Version>, Stop<CalcError>> {
        self.charge(1)?;
        let t = self.asof.clone();
        let Some((hash, _)) = testrun::binding_of(self.store, &self.overlay, &cell.name, &t)
            .map_err(|e| Stop::World(CalcError::Store(e)))?
        else {
            return Err(Stop::Code(Code::Absent));
        };
        let id = testrun::cell_id(&cell.name, &hash, &cell.args);
        if let Some(e) = self.refusal(&cell.name, &id) {
            return Err(Stop::World(e));
        }
        let b = self.compile(&cell.name, &hash).map_err(Stop::World)?;
        if b.compiled.is_some() {
            return Err(Stop::World(CalcError::Unsupported(format!(
                "the versions of the computed function {} are not read by calc",
                cell.name
            ))));
        }
        let members = self.store.versions_between(&id, from, to, &t).map_err(|e| Stop::World(store_err(e)))?;
        let mut out = Vec::with_capacity(members.len());
        for m in members {
            let r = Receipt::from_json(&m.receipt).map_err(|e| Stop::World(store_err(e)))?;
            self.tick().map_err(|_| Stop::World(CalcError::Budget))?;
            let trust = self.trust_of(&r);
            self.note(&cell.name, &cell.args, Some(&m.version.receipt_hash), &trust);
            out.push(Version {
                value: r.value().clone(),
                receipt_hash: m.version.receipt_hash,
                tx: m.version.tx,
                asof: m.version.asof,
            });
        }
        self.charge(out.len() as u64)?;
        Ok(out)
    }

    fn charge(&mut self, n: u64) -> Result<(), Code> {
        self.tick()?;
        self.used = self.used.saturating_add(n);
        if self.used > FUEL {
            return Err(Code::Budget);
        }
        Ok(())
    }

    fn asof(&mut self) -> Result<String, Stop<CalcError>> {
        Ok(self.asof.clone())
    }

    fn blob(&mut self, sha256: &str, length: u64) -> Result<Option<Vec<u8>>, Stop<CalcError>> {
        let _ = length;
        let b = self.store.blob(sha256).map_err(|e| Stop::World(store_err(e)))?;
        Ok(b.map(|b| b.into_owned()))
    }
}

/// The reads a body names (each cell read's function name, at any depth), once each.
fn reads_of(ast: &Ast) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut todo = vec![ast.root()];
    while let Some(id) = todo.pop() {
        match ast.node(id) {
            Node::Num { .. } | Node::Str(_) | Node::Bool(_) | Node::Date(_) | Node::Instant(_) => {}
            Node::Var(_) | Node::Agg(_) | Node::Key(_) => {}
            Node::Op { args, .. } => todo.extend(args.iter().copied()),
            Node::If { cond, then, els } => todo.extend([*cond, *then, *els]),
            Node::Let { value, body, .. } => todo.extend([*value, *body]),
            Node::Read { fn_name, args, .. } => {
                out.insert(fn_name.clone());
                for (_, a) in args {
                    match a {
                        ReadArg::Expr(n) => todo.push(*n),
                        ReadArg::Range(lo, hi) => todo.extend([*lo, *hi]),
                        ReadArg::Open => {}
                    }
                }
            }
            Node::Dispatch { on, args, .. } => {
                todo.push(*on);
                todo.extend(args.iter().map(|(_, n)| *n));
            }
            Node::Call { args, named, .. } => {
                todo.extend(args.iter().copied());
                todo.extend(named.iter().map(|(_, n)| *n));
            }
            Node::Lambda { body, .. } => todo.push(*body),
        }
    }
    out
}

/// `{"outcome": ..., "error": <code>}` with `status`: a code the core's error vocabulary does not
/// carry (the formula's and the budget's), in the envelope of the other errors.
fn raw(status: u16, outcome: &str, code: &str) -> Reply {
    let body = Value::Object(vec![("outcome".into(), s(outcome)), ("error".into(), s(code))]);
    Reply::json(status, body.to_jcs(), Cache::NoStore)
}

fn s(x: &str) -> Value {
    Value::String(x.to_string())
}

/// A `bad_request` reply with `code`.
fn bad(code: ErrorCode) -> Reply {
    envelope::bad_request_reply(code)
}

/// The reply of a formula code that ends a calculation: a fault as `not_covered` with its
/// `fault`, the budget as `E_EVAL_BUDGET`, anything else as an `error` with its code.
fn code_reply(c: Code) -> Reply {
    if c == Code::Budget {
        return budget_reply();
    }
    if c.is_fault() {
        if let Some(f) = Fault::from_code(c.as_str()) {
            return envelope::answer_reply(&Answer::not_covered().with_fault(f), Cache::NoStore);
        }
    }
    raw(500, "error", c.as_str())
}

/// `413` `E_TOO_LARGE`: the formula, or the request, is larger than the calculation takes.
pub(crate) fn too_large_reply() -> Reply {
    raw(413, "bad_request", "E_TOO_LARGE")
}

/// `E_EVAL_BUDGET`: the mesh's budget ran out; there is no job to wait for.
fn budget_reply() -> Reply {
    raw(503, "error", "E_EVAL_BUDGET")
}

impl Inner {
    /// `POST /x/calc` (Build 2 Part 3 addition): the value of the body's formula as of its asof (else
    /// the mesh's now), evaluated in a scope with its lineage. Nothing is written.
    pub(crate) fn calc(&self, body: &[u8], principal: Option<&Principal>) -> Reply {
        let v = match json::parse(body) {
            Ok(v) => v,
            Err(e) => return bad(e.code()),
        };
        let Value::Object(members) = &v else { return bad(ErrorCode::Schema) };
        if members.iter().any(|(k, _)| !matches!(k.as_str(), "formula" | "asof")) {
            return bad(ErrorCode::Schema);
        }
        let Some(formula) = v.get("formula").and_then(Value::as_str) else { return bad(ErrorCode::Schema) };
        let asof = match v.get("asof") {
            None => None,
            Some(Value::String(a)) => Some(crate::instant::canonical_or_same(a, true)),
            Some(_) => return bad(ErrorCode::Schema),
        };
        if let Some(a) = &asof {
            if let Err(e) = datumesh_core::lexical::check_instant(a, "$") {
                return bad(e.code());
            }
        }
        if formula.len() > MAX_FORMULA {
            return too_large_reply();
        }
        let at = asof.unwrap_or_else(|| self.now());
        let ast = match canonical_ast(formula, None) {
            Ok(a) => a,
            Err(e) => return raw(400, "bad_request", e.code.as_str()),
        };
        let store: &(dyn Store + Send + Sync) = &*self.store;
        // The body's callees as they are bound at the asof, then the body's own type (the first
        // pass of a definition: the return type is what the body infers).
        let placeholder = Value::Object(vec![
            ("args".into(), Value::Object(vec![])),
            ("order".into(), Value::Array(vec![])),
            ("returns".into(), Value::Object(vec![("type".into(), s("number"))])),
        ]);
        let sig = match Signature::from_value(&placeholder) {
            Ok(sig) => sig,
            Err(_) => return envelope::error_reply(ErrorCode::EvalUnavailable),
        };
        let mut env = Env::new(&format!("{}/calc", self.config.host), sig);
        for name in reads_of(&ast) {
            match store.callee(&name, &at) {
                Ok(Some(c)) => {
                    if env.add_function(&name, &c.descriptor).is_err() {
                        return envelope::error_reply(ErrorCode::InputUnavailable);
                    }
                }
                Ok(None) => {}
                Err(_) => return envelope::error_reply(ErrorCode::InputUnavailable),
            }
        }
        let compiled: Compiled = match define(formula, &env) {
            Ok(c) => c,
            Err(e) => return raw(400, "bad_request", e.code.as_str()),
        };
        // The body as a function with no arguments and the inferred return type.
        let descriptor = Value::Object(vec![
            ("code".into(), Value::Object(vec![("body".into(), s(&compiled.body)), ("lang".into(), s("formula"))])),
            ("inputs".into(), Value::Array(compiled.inputs.iter().map(|i| s(i)).collect())),
            (
                "signature".into(),
                Value::Object(vec![
                    ("args".into(), Value::Object(vec![])),
                    ("order".into(), Value::Array(vec![])),
                    ("returns".into(), Value::Object(vec![("type".into(), s(compiled.returns.ty.as_str()))])),
                ]),
            ),
        ]);
        let function = match Function::from_value(&descriptor) {
            Ok(f) => f,
            Err(_) => return envelope::error_reply(ErrorCode::EvalUnavailable),
        };
        let mut run = Run::new(self, store, &at, principal);
        match eval::evaluate(&compiled, &function, &Args::new(), &at, &mut run) {
            Ok(o) => {
                let mut out = vec![
                    ("outcome".into(), s("calculation")),
                    ("kind".into(), s("calculation")),
                    ("value".into(), s(o.value.lexical())),
                    ("type".into(), s(compiled.returns.ty.as_str())),
                ];
                if let Some(u) = &compiled.returns.unit {
                    out.push(("unit".into(), s(u)));
                }
                out.push(("formula".into(), s(formula)));
                out.push(("canonical".into(), s(&ast.print())));
                out.push(("asof".into(), s(&at)));
                let lineage = run
                    .lineage
                    .iter()
                    .map(|e| {
                        let args = e.args.iter().map(|(k, v)| (k.clone(), s(v.lexical()))).collect();
                        Value::Object(vec![
                            ("call".into(), s(&e.call)),
                            ("args".into(), Value::Object(args)),
                            ("receipt".into(), e.receipt.as_deref().map_or(Value::Null, s)),
                            ("publisher".into(), s(&e.publisher)),
                            ("trust".into(), s(&e.trust)),
                        ])
                    })
                    .collect();
                out.push(("lineage".into(), Value::Array(lineage)));
                Reply::json(200, Value::Object(out).to_jcs(), Cache::NoStore)
            }
            Err(Stop::Code(c)) => code_reply(c),
            Err(Stop::World(e)) => match e {
                CalcError::Budget => budget_reply(),
                CalcError::Cycle(path) => self.cycle_reply(&path, Some(&at), None),
                CalcError::Refused(r) => r,
                CalcError::Depth => envelope::error_reply(ErrorCode::EvalUnavailable),
                CalcError::Unsupported(m) | CalcError::Store(m) => {
                    tracing::warn!(error = %m, "calc: the calculation could not be made");
                    envelope::error_reply(ErrorCode::EvalUnavailable)
                }
            },
        }
    }
}
