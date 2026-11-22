//! Classes in the mesh, the call layer against the spec's vectors (spec-0.5.0, c8896ad): every
//! `call` vector whose environment has classes (`call/class-*` and `invalid/call-class-*`) is run
//! through `datumesh_http::call::resolve` with a [`Bindings`] built from the vector's environment
//! (its functions, classes, lookups and alias snapshot): the same cell URI, the same `resolved`
//! (compared as the vector carries it), or the same code. This is the mesh's side of 5.8: the formula
//! crate parses; `Bindings` must hand it the classes (`class`), the lookup (`lookup`) and the alias
//! snapshot (`aliases_tx`), and `Target::resolved` must carry what it resolved.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use datumesh_core::json::{self, Value};
use datumesh_core::{Arg, CellRef, ClassDescriptor, ErrorCode, FunctionDescriptor, Resolved, Scalar};
use datumesh_http::call::{self, Bindings, Bound, BoundClass, Rejection};

const NOW: &str = "2026-06-01T00:00:00.000000Z";

/// The vectors that c8896ad added to the class call vectors (`spec-0.5.0`'s second clarifications:
/// H3, a quoted key encoding decoded against its class; an extension method that does not take the
/// class keys first is `E_ARG_UNKNOWN`): datumesh-formula at a6fd557 answered them wrongly (`parse_call` accepted the first and
/// gave `E_NAME_UNBOUND` for the second); formula b6dd484 (the re-pin) answers them. They are the second
/// test below, apart from the first, which is about the mesh's own layer.
const FORMULA_REPIN: &[&str] =
    &["invalid/call-class-extension-without-keys", "invalid/call-class-quoted-key-normalisation"];

struct VectorEnv {
    functions: BTreeMap<String, Arc<FunctionDescriptor>>,
    classes: BTreeMap<String, Arc<ClassDescriptor>>,
    lookups: BTreeMap<String, BTreeMap<String, String>>,
    aliases_tx: Option<String>,
    /// The classes round (K5): flattened class descriptors held by hash only, no name bound (`class_descriptors`).
    held: Vec<Arc<ClassDescriptor>>,
    /// The classes round (K10): the values of the cells a chain reads, by canonical call without `?asof=`.
    cells: BTreeMap<String, String>,
}

impl VectorEnv {
    fn from_value(env: &Value) -> VectorEnv {
        let object = |k: &str| -> Vec<(String, Value)> {
            env.get(k).and_then(Value::as_object).map(|o| o.to_vec()).unwrap_or_default()
        };
        VectorEnv {
            functions: object("functions")
                .into_iter()
                .map(|(n, d)| (n, Arc::new(FunctionDescriptor::from_value(&d).unwrap())))
                .collect(),
            classes: object("classes")
                .into_iter()
                .map(|(n, d)| (n, Arc::new(ClassDescriptor::from_value(&d).unwrap())))
                .collect(),
            lookups: object("lookups")
                .into_iter()
                .map(|(c, m)| {
                    let labels = m
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(l, v)| (l.clone(), v.as_str().unwrap().to_string()))
                        .collect();
                    (c, labels)
                })
                .collect(),
            aliases_tx: env.get("aliases_tx").and_then(Value::as_str).map(str::to_string),
            held: match env.get("class_descriptors") {
                Some(Value::Array(ds)) => {
                    ds.iter().map(|d| Arc::new(ClassDescriptor::from_value(d).unwrap())).collect()
                }
                _ => Vec::new(),
            },
            cells: object("cells")
                .into_iter()
                .filter_map(|(call, k)| k.as_str().map(|k| (call, k.to_string())))
                .collect(),
        }
    }
}

impl Bindings for VectorEnv {
    fn binding(&self, name: &str, _asof: &str) -> Result<Option<Arc<Bound>>, ErrorCode> {
        Ok(self.functions.get(name).map(|d| Arc::new(Bound::new(d.hash(), BIND_TX, d.clone()))))
    }
    fn pinned(&self, fn_hash: &str) -> Result<Option<Arc<Bound>>, ErrorCode> {
        Ok(self.functions.values().find(|d| d.hash() == fn_hash).map(|d| Arc::new(Bound::new(fn_hash, "", d.clone()))))
    }
    fn class(&self, name: &str, _asof: &str) -> Result<Option<Arc<BoundClass>>, ErrorCode> {
        Ok(self
            .classes
            .get(name)
            .map(|c| Arc::new(BoundClass { class_hash: c.hash(), since: BIND_TX.to_string(), descriptor: c.clone() })))
    }
    fn lookup(&self, class: &str, label: &str, _asof: &str) -> Result<Option<String>, ErrorCode> {
        Ok(self.lookups.get(class).and_then(|m| m.get(label)).cloned())
    }
    fn aliases_tx(&self) -> Option<String> {
        self.aliases_tx.clone()
    }
    fn class_name_of(&self, class_hash: &str, _asof: &str) -> Result<Option<String>, ErrorCode> {
        Ok(self.classes.iter().find(|(_, d)| d.hash() == class_hash).map(|(n, _)| n.clone()))
    }
    fn class_by_hash(&self, class_hash: &str) -> Result<Option<ClassDescriptor>, ErrorCode> {
        let named = self.classes.values().find(|d| d.hash() == class_hash);
        Ok(named.or_else(|| self.held.iter().find(|d| d.hash() == class_hash)).map(|d| (**d).clone()))
    }
    fn chain(&self, cell: &CellRef, _asof: &str) -> Result<Option<String>, ErrorCode> {
        Ok(self.cells.get(&canonical_cell(cell)).cloned())
    }
}

/// A cell's canonical call without `?asof=`: its full name and its arguments, each labelled, in ascending order of
/// names (16.7), as the `cells` of a vector's environment write it.
fn canonical_cell(cell: &CellRef) -> String {
    let literal = |a: &Arg| match a {
        Arg::Slice => "*".to_string(),
        Arg::Value(Scalar::String(t)) | Arg::Value(Scalar::Blob(t)) => format!("\"{}\"", t.replace('"', "\"\"")),
        Arg::Value(s) => s.lexical().to_string(),
    };
    let parts: Vec<String> = cell.args.iter().map(|(k, a)| format!("{k}: {}", literal(a))).collect();
    format!("{}/{}({})", cell.ns, cell.fn_name, parts.join(", "))
}

/// `resolved` as the envelope carries it.
fn resolved_json(r: &Resolved) -> Value {
    let mut members = vec![("call".to_string(), Value::String(r.call.clone()))];
    if let Some(t) = &r.aliases_tx {
        members.push(("aliases_tx".to_string(), Value::String(t.clone())));
    }
    let items = r
        .resolutions
        .iter()
        .map(|x| {
            let mut m = vec![
                ("from".to_string(), Value::String(x.from.clone())),
                ("kind".to_string(), Value::String(x.kind.as_str().to_string())),
                ("to".to_string(), Value::String(x.to.clone())),
            ];
            if let Some(a) = &x.asof {
                m.push(("asof".to_string(), Value::String(a.clone())));
            }
            Value::Object(m)
        })
        .collect();
    members.push(("resolutions".to_string(), Value::Array(items)));
    Value::Object(members)
}

fn code_of(r: &Rejection) -> String {
    match r {
        Rejection::BadRequest(c) | Rejection::Error(c) => c.as_str().to_string(),
        Rejection::NotCovered => "E_FN_UNAVAILABLE".to_string(),
    }
}

#[test]
fn the_class_call_vectors_resolve_as_they_say() {
    run(false);
}

/// The vectors of c8896ad that wait for the formula re-pin (see [`FORMULA_REPIN`]).
#[test]
fn the_vectors_of_the_formula_repin_resolve_as_they_say() {
    run(true);
}

fn run(repin: bool) {
    let mut files = Vec::new();
    for dir in ["call", "invalid"] {
        for e in std::fs::read_dir(spec_dir().join("vectors").join(dir)).unwrap() {
            files.push(e.unwrap().path());
        }
    }
    files.sort();
    let (mut valid, mut invalid, mut skipped, mut failures) = (0, 0, Vec::new(), Vec::new());
    for p in files {
        let v = json::parse(&std::fs::read(&p).unwrap()).unwrap();
        let env = v.get("env");
        if v.get("kind").and_then(Value::as_str) != Some("call") || env.and_then(|e| e.get("classes")).is_none() {
            continue;
        }
        let id = v.get("id").and_then(Value::as_str).unwrap().to_string();
        if FORMULA_REPIN.contains(&id.as_str()) != repin {
            continue;
        }
        if id.contains("no-now") {
            // A call with neither an asof nor a clock: the mesh always has its now (16.7).
            skipped.push(id);
            continue;
        }
        let env = VectorEnv::from_value(env.unwrap());
        let aliases: BTreeMap<String, String> = v
            .get("env")
            .and_then(|e| e.get("aliases"))
            .and_then(Value::as_object)
            .map(|o| o.iter().map(|(k, x)| (k.clone(), x.as_str().unwrap().to_string())).collect())
            .unwrap_or_default();
        let input = v.get("input").and_then(Value::as_str).unwrap();
        let r = call::resolve(input, NOW, &aliases, &env, false);
        if v.get("valid") == Some(&Value::Bool(true)) {
            match r {
                Ok(t) => {
                    let want_uri = v.get("uri").and_then(Value::as_str).unwrap();
                    if t.uri != want_uri {
                        failures.push(format!("{id}: uri {} for {want_uri}", t.uri));
                    }
                    if let Some(want) = v.get("resolved") {
                        match &t.resolved {
                            Some(got) if resolved_json(got).to_jcs() == want.to_jcs() => {}
                            other => failures.push(format!(
                                "{id}: resolved {:?} for {}",
                                other.as_ref().map(|r| String::from_utf8(resolved_json(r).to_jcs()).unwrap()),
                                String::from_utf8(want.to_jcs()).unwrap()
                            )),
                        }
                    }
                    valid += 1;
                }
                Err(e) => failures.push(format!("{id}: {e:?}")),
            }
        } else {
            let want = v.get("error").and_then(Value::as_str).unwrap();
            match r {
                Err(e) if code_of(&e) == want => {}
                // A fault is answered `not_covered` with `fault`: `resolve` returns `Rejection::NotCovered` and hands the
                // fault to the mesh through the crate-private `take_fault`, which this harness cannot read. The envelope's
                // `fault` is pinned over HTTP by cr_chain.rs (k10_a_key_outside_the_declared_classes_is_a_fault).
                Err(Rejection::NotCovered) if want == "E_FORMULA_DISPATCH_KEY" => {}
                other => failures
                    .push(format!("{id}: {:?} for {want}", other.map(|t| t.uri.clone()).map_err(|e| code_of(&e)))),
            }
            invalid += 1;
        }
    }
    let out = format!(
        "class call vectors: {valid} valid, {invalid} invalid; skipped {skipped:?}; failures {}\n",
        failures.len()
    );
    std::fs::write(out_dir().join("classes-call-vectors.txt"), &out).unwrap();
    assert!(failures.is_empty(), "{out}{}", failures.join("\n"));
    if !repin {
        assert!(valid >= 8 && invalid >= 10, "{out}");
    } else {
        assert_eq!(valid + invalid, FORMULA_REPIN.len(), "{out}");
    }
}
