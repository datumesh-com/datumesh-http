//! `POST /whatif` (spec-0.6.0, 5.9; Build 2 Part 3, E1 and E2) and the cycle answer (E3).
//!
//! A scenario is the value a cell would have under overrides: it is labelled a scenario, carries no
//! receipt, no trust and no signature, and is never stored or served at `/cell`. The body is the
//! JSON of 5.9: `cell`, optional `asof`, `overrides` (value overrides `{cell, value}` and slice
//! overrides `{slice, add, remove}`), and optional `solve` (`vary`, `target`, `equals`, `lo`, `hi`).
//! Any other member is `E_SCHEMA`; so is a missing `cell` or `overrides`, and empty overrides
//! without `solve`.
use datumesh_core::json::{self, Value};
use datumesh_core::{Answer, Cell, CellRef, CycleFix, ErrorCode, Fault, Scalar, Scenario, Type};
use datumesh_engine::{EngineError, Overrides, ScenarioAnswer, ScenarioOverride, Solve};

use crate::apikeys::Principal;
use crate::call::{self, Target};
use crate::envelope::{self, Cache, Reply};
use crate::mesh::Inner;

/// A `bad_request` reply with `code`.
fn bad(code: ErrorCode) -> Reply {
    envelope::bad_request_reply(code)
}

/// The member `k` of `v` as a string, or `None` when it is absent; `Err` when it is not a string.
fn opt_text<'a>(v: &'a Value, k: &str) -> Result<Option<&'a str>, ErrorCode> {
    match v.get(k) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(ErrorCode::Schema),
    }
}

/// The typed scalar `{"type", "value"}` of an override, as the canonical checks read it.
fn typed(v: &Value) -> Result<Scalar, ErrorCode> {
    let (Some(ty), Some(value)) = (v.get("type").and_then(Value::as_str), v.get("value").and_then(Value::as_str))
    else {
        return Err(ErrorCode::Schema);
    };
    let ty = Type::from_name(ty).ok_or(ErrorCode::CanonType)?;
    Scalar::from_lexical(ty, value).map_err(|e| e.code())
}

/// A number of a solve: a lexical number.
fn number(text: &str) -> Result<Scalar, ErrorCode> {
    Scalar::from_lexical(Type::Number, text).map_err(|e| e.code())
}

/// The keys of an object, or `E_SCHEMA` unless they are exactly `want`.
fn exactly(v: &Value, want: &[&str]) -> Result<(), ErrorCode> {
    let Value::Object(m) = v else { return Err(ErrorCode::Schema) };
    if m.len() != want.len() || !want.iter().all(|k| m.iter().any(|(x, _)| x == k)) {
        return Err(ErrorCode::Schema);
    }
    Ok(())
}

/// A cell's JSON as a canonical override names it.
fn cell_value(c: &Cell) -> Value {
    c.to_value()
}

/// An override of a scenario as the answer's `overrides` array lists it (5.9).
fn override_value(o: &ScenarioOverride) -> Value {
    match o {
        ScenarioOverride::Value { cell, value } => {
            Value::Object(vec![("cell".into(), cell_value(cell)), ("value".into(), value.to_value())])
        }
        ScenarioOverride::Members { slice, members } => {
            // The echo (5.9): the members sorted by cell id, present even when empty, no `add` or `remove`.
            let mut sorted: Vec<&(Cell, Scalar)> = members.iter().collect();
            sorted.sort_by_key(|(c, _)| c.id());
            Value::Object(vec![
                ("slice".into(), cell_value(slice)),
                (
                    "members".into(),
                    Value::Array(
                        sorted
                            .into_iter()
                            .map(|(c, v)| {
                                Value::Object(vec![("cell".into(), cell_value(c)), ("value".into(), v.to_value())])
                            })
                            .collect(),
                    ),
                ),
            ])
        }
        ScenarioOverride::Slice { slice, add, remove } => Value::Object(vec![
            ("slice".into(), cell_value(slice)),
            (
                "add".into(),
                Value::Array(
                    add.iter()
                        .map(|(c, v)| {
                            Value::Object(vec![("cell".into(), cell_value(c)), ("value".into(), v.to_value())])
                        })
                        .collect(),
                ),
            ),
            ("remove".into(), Value::Array(remove.iter().map(cell_value).collect())),
        ]),
        _ => Value::Null,
    }
}

impl Inner {
    /// Resolves a request's cell text (a call or a URI rest) as of `at`, a slice when `slice`.
    fn whatif_cell(
        &self,
        text: &str,
        at: &str,
        principal: Option<&Principal>,
        slice: bool,
    ) -> Result<std::sync::Arc<Target>, Reply> {
        let t = call::resolve(text, at, &self.config.aliases, self, slice).map_err(|r| match r {
            // A string that is neither a call nor a cell URI's rest: a URI that does not parse.
            call::Rejection::BadRequest(ErrorCode::CanonName) => bad(ErrorCode::ParseUri),
            other => self.rejection(other, None),
        })?;
        let (refused, _) = self.policy(&t, principal);
        match refused {
            Some(r) => Err(r),
            None => Ok(t),
        }
    }

    /// A cell of the body, as a call or a cell URI's rest (a string), or as a cell object of 4.4
    /// (`{ns, fn, fn_hash, args}`, `*` for an open argument of a slice).
    fn cell_ref_of(&self, v: &Value, at: &str, principal: Option<&Principal>, slice: bool) -> Result<CellRef, Reply> {
        match v {
            Value::String(text) => self.whatif_cell(text, at, principal, slice).map(|t| t.reference.clone()),
            Value::Object(_) => {
                let c = Cell::from_value(v).map_err(|e| bad(e.code()))?;
                Ok(CellRef { ns: c.ns, fn_name: c.fn_name, args: c.args, asof: None })
            }
            _ => Err(bad(ErrorCode::Schema)),
        }
    }

    /// `POST /whatif`: the scenario of the body's cell under its overrides (5.9).
    pub(crate) fn whatif(&self, body: &[u8], principal: Option<&Principal>) -> Reply {
        let v = match json::parse(body) {
            Ok(v) => v,
            Err(e) => return bad(e.code()),
        };
        let Value::Object(members) = &v else { return bad(ErrorCode::Schema) };
        for (k, _) in members {
            if !matches!(k.as_str(), "cell" | "asof" | "overrides" | "solve") {
                return bad(ErrorCode::Schema);
            }
        }
        let (asof, Some(cell_text), Some(overrides), solve) = (
            opt_text(&v, "asof"),
            v.get("cell").and_then(Value::as_str),
            v.get("overrides").and_then(|o| match o {
                Value::Array(a) => Some(a),
                _ => None,
            }),
            v.get("solve"),
        ) else {
            return bad(ErrorCode::Schema);
        };
        let asof = match asof {
            Ok(a) => a.map(str::to_string),
            Err(c) => return bad(c),
        };
        if overrides.is_empty() && solve.is_none() {
            return bad(ErrorCode::Schema);
        }
        if let Some(a) = &asof {
            if let Err(e) = datumesh_core::lexical::check_instant(a, "$") {
                return bad(e.code());
            }
        }
        let now = self.now();
        let at = asof.clone().unwrap_or_else(|| now.clone());
        let target = match self.whatif_cell(cell_text, &at, principal, false) {
            Ok(t) => t,
            Err(r) => return r,
        };
        let mut ov = Overrides::new();
        for item in overrides {
            let Value::Object(_) = item else { return bad(ErrorCode::Schema) };
            if item.get("slice").is_some() {
                // R3: `members` replaces the slice's whole member set and excludes `add` and `remove` (5.9).
                if item.get("members").is_some() {
                    if item.get("add").is_some() || item.get("remove").is_some() {
                        return bad(ErrorCode::SliceMember);
                    }
                    if let Err(c) = exactly(item, &["slice", "members"]) {
                        return bad(c);
                    }
                    let slice = match self.cell_ref_of(item.get("slice").unwrap_or(&Value::Null), &at, principal, true)
                    {
                        Ok(c) => c,
                        Err(r) => return r,
                    };
                    let Some(Value::Array(items)) = item.get("members") else { return bad(ErrorCode::Schema) };
                    let mut members = Vec::new();
                    for m in items {
                        if let Err(c) = exactly(m, &["cell", "value"]) {
                            return bad(c);
                        }
                        let cell = match self.cell_ref_of(m.get("cell").unwrap_or(&Value::Null), &at, principal, false)
                        {
                            Ok(c) => c,
                            Err(r) => return r,
                        };
                        let value = match typed(m.get("value").unwrap_or(&Value::Null)) {
                            Ok(s) => s,
                            Err(c) => return bad(c),
                        };
                        members.push((cell, value));
                    }
                    ov = ov.slice_members(slice, members);
                    continue;
                }
                if let Err(c) = exactly(item, &["slice", "add", "remove"]) {
                    return bad(c);
                }
                let slice = match self.cell_ref_of(item.get("slice").unwrap_or(&Value::Null), &at, principal, true) {
                    Ok(c) => c,
                    Err(r) => return r,
                };
                let add_items = match item.get("add") {
                    Some(Value::Array(a)) => a,
                    _ => return bad(ErrorCode::Schema),
                };
                let mut add = Vec::new();
                for a in add_items {
                    if let Err(c) = exactly(a, &["cell", "value"]) {
                        return bad(c);
                    }
                    let cell = match self.cell_ref_of(a.get("cell").unwrap_or(&Value::Null), &at, principal, false) {
                        Ok(c) => c,
                        Err(r) => return r,
                    };
                    let value = match typed(a.get("value").unwrap_or(&Value::Null)) {
                        Ok(s) => s,
                        Err(c) => return bad(c),
                    };
                    add.push((cell, value));
                }
                let remove_items = match item.get("remove") {
                    Some(Value::Array(r)) => r,
                    _ => return bad(ErrorCode::Schema),
                };
                let mut remove: Vec<CellRef> = Vec::new();
                for r in remove_items {
                    match self.cell_ref_of(r, &at, principal, false) {
                        Ok(c) => remove.push(c),
                        Err(reply) => return reply,
                    }
                }
                ov = ov.slice(slice, add, remove);
            } else {
                if let Err(c) = exactly(item, &["cell", "value"]) {
                    return bad(c);
                }
                let cell = match self.cell_ref_of(item.get("cell").unwrap_or(&Value::Null), &at, principal, false) {
                    Ok(c) => c,
                    Err(r) => return r,
                };
                let value = match typed(item.get("value").unwrap_or(&Value::Null)) {
                    Ok(s) => s,
                    Err(c) => return bad(c),
                };
                ov = ov.value(cell, value);
            }
        }
        let solve = match solve {
            None => None,
            Some(s) => {
                if let Err(c) = exactly(s, &["vary", "target", "equals", "lo", "hi"]) {
                    return bad(c);
                }
                let text = |k: &str| s.get(k).and_then(Value::as_str).ok_or(ErrorCode::Schema);
                let vary = match self.cell_ref_of(s.get("vary").unwrap_or(&Value::Null), &at, principal, false) {
                    Ok(c) => c,
                    Err(reply) => return reply,
                };
                let target_ref = match self.cell_ref_of(s.get("target").unwrap_or(&Value::Null), &at, principal, false)
                {
                    Ok(c) => c,
                    Err(reply) => return reply,
                };
                let num = |k: &str| text(k).and_then(|t| number(t).map_err(|_| ErrorCode::CanonNumber));
                let (equals, lo, hi) = match (num("equals"), num("lo"), num("hi")) {
                    (Ok(e), Ok(l), Ok(h)) => (e, l, h),
                    (Err(c), _, _) | (_, Err(c), _) | (_, _, Err(c)) => return bad(c),
                };
                Some(Solve::new(vary, target_ref, equals, lo, hi))
            }
        };
        match self.engine.whatif(&target.reference, asof.as_deref(), &ov, solve.as_ref()) {
            Ok(a) => self.scenario_reply(a),
            Err(e) => self.whatif_error(e),
        }
    }

    /// The scenario answer (5.9): the envelope the core's builder makes, checked by its rules.
    fn scenario_reply(&self, a: ScenarioAnswer) -> Reply {
        let overrides = Value::Array(a.overrides.iter().map(override_value).collect());
        let built = Scenario::new(a.cell, a.value, &a.asof, &overrides).and_then(|s| match a.solved {
            Some((vary, value)) => s.with_solved(vary, value),
            None => Ok(s),
        });
        match built {
            Ok(s) => envelope::answer_reply(&Answer::scenario(s), Cache::NoStore),
            Err(_) => envelope::error_reply(ErrorCode::EvalUnavailable),
        }
    }

    /// The reply of an engine error of `whatif`: a `not_covered` with its fault, a `bad_request`
    /// with its code, an `error` (a cycle with its path and fixes).
    fn whatif_error(&self, e: EngineError) -> Reply {
        match e {
            EngineError::NotCovered { fault } => {
                let a = match fault {
                    Some(f) => Answer::not_covered().with_fault(f),
                    None => Answer::not_covered(),
                };
                envelope::answer_reply(&a, Cache::NoStore)
            }
            EngineError::WhatifNoSolution => {
                envelope::answer_reply(&Answer::not_covered().with_fault(Fault::WhatifNoSolution), Cache::NoStore)
            }
            EngineError::Override(code) => match ErrorCode::from_code(&code) {
                Some(c) => bad(c),
                None => bad(ErrorCode::Schema),
            },
            EngineError::Cycle { path } => self.cycle_reply(&path, None, None),
            other => {
                let (request, code) = crate::mesh::engine_code(&other);
                if request {
                    bad(code)
                } else {
                    envelope::error_answer_reply(code, None)
                }
            }
        }
    }

    /// The `error` answer of a cycle (5.5, 15.12; Build 2 Part 3, E3): the path of calls in read
    /// order, and the fixes that apply: `asof` and `solver` always, `lag` when the cycle passes
    /// through a function with a `date` or `instant` argument, `step` through one with a `number`
    /// argument. `at` is the request's asof (the mesh's now when it has none).
    pub(crate) fn cycle_reply(&self, path: &[String], at: Option<&str>, fn_hash: Option<&str>) -> Reply {
        let at = at.map_or_else(|| self.now(), str::to_string);
        let (mut lag, mut step) = (false, false);
        for call in path {
            let name = call.split('(').next().unwrap_or("");
            let Ok(Some(b)) = self.store.resolve_binding(name, &at).map(|b| b.map(|b| b.fn_hash)) else { continue };
            let Ok(d) = self.descriptor(&b) else { continue };
            for decl in d.signature.args.values() {
                match decl.ty {
                    Type::Date | Type::Instant => lag = true,
                    Type::Number => step = true,
                    _ => {}
                }
            }
        }
        let mut fixes = vec![CycleFix::Asof];
        if lag {
            fixes.push(CycleFix::Lag);
        }
        if step {
            fixes.push(CycleFix::Step);
        }
        fixes.push(CycleFix::Solver);
        match datumesh_core::Cycle::new(path.to_vec(), fixes) {
            Ok(cycle) => match Answer::error(ErrorCode::EvalUnavailable).and_then(|a| a.with_cycle(cycle)) {
                Ok(a) => {
                    let a = envelope::with_fn_hash(a, fn_hash)
                        .with_status(envelope::error_status(ErrorCode::EvalUnavailable));
                    envelope::answer_reply(&a, Cache::NoStore)
                }
                Err(_) => envelope::error_reply(ErrorCode::EvalUnavailable),
            },
            Err(_) => envelope::error_reply(ErrorCode::EvalUnavailable),
        }
    }
}
