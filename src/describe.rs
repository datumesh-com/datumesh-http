//! describe (SPEC 16.8, 9.6) and short ids.
//!
//! `GET /describe/{id}`: `{id}` is a class name, a cell id, an object id or a short id (12 hex
//! digits), or an object's key encoding (16.5); never a class hash. A class answers with the
//! `describe-class` document, an object with the `describe-object` one.
//!
//! # Known objects
//!
//! Objects are never stored, so the objects the mesh *knows* (the ones an object id or a short id
//! can name) are the objects of the recorded cells of a class bound in the store (as of now): for
//! every method `N/m` of the class, stored or computed (a formula), every recorded cell with every
//! argument open. The stored cells come from the store's `slice_values` over `N/m`; the computed
//! ones from its `computed_cells` over the same slice, at most [`INDEX_LIMIT`] per method (reaching
//! it logs one warning and keeps what was read). The class's `extent` function is one of its
//! methods, so its recorded cells are covered the same way. Nothing is evaluated to build the index,
//! and no product of argument extents is formed.
//!
//! The index is kept in memory (object id to object, cell id to cell). It is built on the first
//! describe or short-id request, and built again when the mesh is told of a stored version
//! ([`Mesh::stored_version_added`](crate::Mesh::stored_version_added)), when it learns of a class,
//! when a formula cell is computed that the index does not hold yet, or after [`INDEX_TTL`].
//!
//! The store cannot list the classes it holds, so the classes the index covers are the ones the
//! mesh has *heard of*: a class a stored version was added under, a class a request or a describe
//! resolved, and the classes of the configured aliases. A class published by another process and
//! never named to this mesh is unknown until one of those happens.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use datumesh_core::json::{self, Value};
use datumesh_core::{
    lexical, parse_object_key, short_id, Arg, Cell, ClassDescriptor, ClassKey, DescribeTests, ErrorCode,
    FunctionAnswer, KeyValue, ObjectKey, Scalar, Slice,
};

use crate::call::{Bindings, BoundClass};
use crate::envelope::{self, Cache, Reply};
use crate::mesh::Inner;

/// How long an index is used before it is built again (a stored version another process added is
/// not announced to this mesh).
pub(crate) const INDEX_TTL: Duration = Duration::from_secs(30);

/// The most recorded cells of one method the index reads from the store (per method, per build).
pub(crate) const INDEX_LIMIT: usize = 1_000_000;

/// An object the mesh knows.
#[derive(Clone, Debug)]
struct Known {
    key: ObjectKey,
    class_hash: String,
}

/// The index of known objects.
#[derive(Default)]
pub(crate) struct Index {
    /// The class names to look at.
    classes: BTreeSet<String>,
    /// Cell ids announced since the index was built: the classes they are of are looked at.
    announced: Vec<String>,
    dirty: bool,
    built: Option<Built>,
}

struct Built {
    at: Instant,
    /// Object id to object.
    objects: BTreeMap<String, Known>,
    /// Cell id to the id of the object whose method cell it is.
    cells: BTreeMap<String, String>,
}

impl Index {
    /// A class the mesh has heard of.
    pub(crate) fn hear_of_class(&mut self, name: &str) {
        if self.classes.insert(name.to_string()) {
            self.dirty = true;
        }
    }

    /// A stored version was added for `cell_id`.
    pub(crate) fn announce(&mut self, cell_id: &str) {
        if self.announced.len() < 100_000 {
            self.announced.push(cell_id.to_string());
        }
        self.dirty = true;
    }

    /// A formula cell was computed: the index is built again unless it already holds the cell.
    pub(crate) fn note_computed(&mut self, cell: &Cell) {
        let id = cell.id();
        if self.built.as_ref().is_some_and(|b| b.cells.contains_key(&id)) {
            return;
        }
        self.hear_of_class(&cell.ns);
        self.announce(&id);
    }
}

/// The class a cell's receipt names: its function's namespace, which is the class name when the
/// function is a method.
fn class_of_cell(inner: &Inner, cell_id: &str) -> Option<String> {
    let far = "9999-12-31T23:59:59.999999Z";
    let (_, bytes) = inner.store.stored_asof_with_receipt(cell_id, far).ok().flatten()?;
    let r = datumesh_core::Receipt::from_json(&bytes).ok()?;
    Some(r.cell().ns.clone())
}

/// The object whose keys are `args`' class keys: a key declared as a class holds the encoding of
/// an object of that class.
fn object_of(class: &str, d: &ClassDescriptor, args: &datumesh_core::Args) -> Option<ObjectKey> {
    let mut keys = BTreeMap::new();
    for (name, decl) in &d.keys {
        let Arg::Value(v) = args.get(name)? else { return None };
        let value = match decl {
            ClassKey::Class(_) => match v {
                Scalar::String(enc) => KeyValue::Object(parse_object_key(enc).ok()?),
                _ => return None,
            },
            _ => KeyValue::Scalar(v.clone()),
        };
        keys.insert(name.clone(), value);
    }
    ObjectKey::new(class, keys).ok()
}

impl Inner {
    /// Remembers a class name for the index.
    pub(crate) fn hear_of_class(&self, name: &str) {
        self.known.lock().unwrap_or_else(|p| p.into_inner()).hear_of_class(name);
    }

    /// The recorded cells of the function `fn_hash` (its descriptor `d`, named `name`) with every
    /// argument open, as of `now`: its stored cells and its computed ones (at most [`INDEX_LIMIT`]).
    fn recorded_cells(&self, name: &str, fn_hash: &str, d: &datumesh_core::FunctionDescriptor, now: &str) -> Vec<Cell> {
        let Some((ns, f)) = name.rsplit_once('/') else { return vec![] };
        let args: datumesh_core::Args = d.signature.args.keys().map(|k| (k.clone(), Arg::Slice)).collect();
        let cell = Cell { ns: ns.into(), fn_name: f.into(), fn_hash: fn_hash.into(), args };
        let Ok(slice) = Slice::from_value(&cell.to_value()) else { return vec![] };
        let mut out = match self.store.slice_values(&slice, now) {
            Ok(vs) => vs.into_iter().map(|v| v.cell).collect(),
            Err(e) => {
                tracing::warn!(target: "datumesh_http", error = %e, "describe index: slice_values");
                vec![]
            }
        };
        match self.store.computed_cells(&slice, now, INDEX_LIMIT) {
            Ok(cells) => {
                if cells.len() >= INDEX_LIMIT {
                    tracing::warn!(target: "datumesh_http", "describe index: {name} reached the limit");
                }
                out.extend(cells);
            }
            Err(e) => tracing::warn!(target: "datumesh_http", error = %e, "describe index: computed_cells"),
        }
        out
    }

    /// Builds the index of known objects as of `now`.
    fn build_index(&self, ix: &mut Index, now: &str) {
        // The classes announced cells were of, and the classes of the configured aliases.
        for id in std::mem::take(&mut ix.announced) {
            if let Some(c) = class_of_cell(self, &id) {
                ix.classes.insert(c);
            }
        }
        for target in self.config.aliases.values() {
            if let Some((ns, _)) = target.rsplit_once('/') {
                ix.classes.insert(ns.to_string());
            }
        }
        let mut objects = BTreeMap::new();
        let mut cells = BTreeMap::new();
        for name in &ix.classes {
            let Ok(Some(c)) = self.class_binding(name, now) else { continue };
            let d = &c.descriptor;
            // Every method of the class, stored or computed (its `extent.keys` function among them).
            for (m, hash) in &d.methods {
                let name_of = format!("{name}/{m}");
                let Ok(fd) = self.descriptor(hash) else { continue };
                for cell in self.recorded_cells(&name_of, hash, &fd, now) {
                    let Some(object) = object_of(name, d, &cell.args) else { continue };
                    let Ok(id) = object.object_id(&c.class_hash) else { continue };
                    cells.insert(cell.id(), id.clone());
                    objects.insert(id, Known { key: object, class_hash: c.class_hash.clone() });
                }
            }
        }
        ix.built = Some(Built { at: Instant::now(), objects, cells });
        ix.dirty = false;
    }

    /// Runs `f` on the index, built first when it is missing, dirty or old.
    fn with_index<T>(&self, f: impl FnOnce(&Built) -> T) -> T {
        let mut ix = self.known.lock().unwrap_or_else(|p| p.into_inner());
        let stale = ix.built.as_ref().is_none_or(|b| b.at.elapsed() > INDEX_TTL);
        if stale || ix.dirty {
            let now = self.now();
            self.build_index(&mut ix, &now);
        }
        match &ix.built {
            Some(b) => f(b),
            None => f(&Built { at: Instant::now(), objects: BTreeMap::new(), cells: BTreeMap::new() }),
        }
    }

    /// The describe reply for `id` (see the module documentation).
    pub(crate) fn describe(&self, id: &str) -> Reply {
        let now = self.now();
        // A class name.
        if lexical::check_function_name(id, "$").is_ok() {
            return match Bindings::class(self, id, &now) {
                Ok(Some(c)) => self.class_answer(id, &c),
                Ok(None) => {
                    // A method of a class is not a function the mesh describes (16.8): its name is
                    // under the class's name.
                    let method = id
                        .rsplit_once('/')
                        .is_some_and(|(ns, _)| matches!(Bindings::class(self, ns, &now), Ok(Some(_))));
                    if method {
                        return not_found();
                    }
                    match Bindings::binding(self, id, &now) {
                        Ok(Some(f)) => self.function_answer(id, &f.fn_hash, &f.descriptor),
                        Ok(None) => not_found(),
                        Err(code) => envelope::error_reply(code),
                    }
                }
                Err(code) => envelope::error_reply(code),
            };
        }
        // An object's key encoding.
        if id.contains('(') {
            return match parse_object_key(id) {
                Ok(key) => self.object_by_key(key, &now),
                Err(e) => envelope::bad_request_reply(e.code()),
            };
        }
        let hex = |s: &str, n: usize| s.len() == n && lexical::check_hex(s, n, "$").is_ok();
        if hex(id, 64) {
            return self.with_index(|b| {
                let object = b.objects.get(id).or_else(|| b.cells.get(id).and_then(|o| b.objects.get(o)));
                match object {
                    Some(k) => self.object_answer(&k.key, &k.class_hash),
                    None => not_found(),
                }
            });
        }
        if hex(id, 12) {
            return self.with_index(|b| {
                let mut found: BTreeSet<&String> = BTreeSet::new();
                for o in b.objects.keys().filter(|o| o.starts_with(id)) {
                    found.insert(o);
                }
                for (c, o) in &b.cells {
                    if c.starts_with(id) {
                        found.insert(o);
                    }
                }
                let mut it = found.into_iter();
                match (it.next(), it.next()) {
                    (None, _) => not_found(),
                    (Some(o), None) => match b.objects.get(o) {
                        Some(k) => self.object_answer(&k.key, &k.class_hash),
                        None => not_found(),
                    },
                    (Some(_), Some(_)) => envelope::bad_request_reply(ErrorCode::ShortIdAmbiguous),
                }
            });
        }
        not_found()
    }

    /// The class answer (16.8): the flattened descriptor and the method table.
    /// The function answer of spec 16.8 (`spec-0.6.0`): the function, and its `tests` (Build 2 Part 3,
    /// E5): the tests held for its hash, their branch arms, the arms they take, and the failing ones.
    fn function_answer(&self, name: &str, fn_hash: &str, d: &datumesh_core::FunctionDescriptor) -> Reply {
        let Ok(answer) = FunctionAnswer::new(name, d) else {
            return envelope::error_reply(ErrorCode::InputUnavailable);
        };
        let counts = crate::testrun::counts(&*self.store, &self.config.host, fn_hash, &self.now());
        let Ok((count, branches, covered, counterexamples)) = counts else {
            return envelope::error_reply(ErrorCode::InputUnavailable);
        };
        let tests = DescribeTests { count, branches, covered, counterexamples };
        let Ok(answer) = answer.with_tests(tests) else {
            return envelope::error_reply(ErrorCode::InputUnavailable);
        };
        // The publisher is the host of the namespace (spec-0.6.1, 16.8), never a domain in the name.
        let Ok(mut answer) = answer.with_publisher(publisher_of(name)) else {
            return envelope::error_reply(ErrorCode::InputUnavailable);
        };
        // A relayed function names its source and the takeover page (spec-0.6.0, 16.8; core's
        // DescribeSource needs the takeover). The function page is not a member of the answer (16.8
        // is strictly checked): it is the `Link` header of the response (see `http::describe`).
        let now = self.now();
        if let Some(relay) = crate::fnpage::relay_of(self, name, fn_hash, d, &now) {
            if let Some(takeover) = relay.takeover {
                let source =
                    datumesh_core::DescribeSource { name: relay.name, relayed_by: self.config.host.clone(), takeover };
                if let Ok(a) = answer.clone().with_source(source) {
                    answer = a;
                }
            }
        }
        Reply::json(200, answer.to_json(), Cache::NoStore)
    }

    fn class_answer(&self, name: &str, c: &BoundClass) -> Reply {
        let Ok(descriptor) = json::parse(&c.descriptor.canonical()) else {
            return envelope::error_reply(ErrorCode::InputUnavailable);
        };
        let mut methods = Vec::new();
        for (m, hash) in &c.descriptor.methods {
            let Ok(fd) = self.descriptor(hash) else { return envelope::error_reply(ErrorCode::InputUnavailable) };
            let fv = fd.to_value();
            let sig = fv.get("signature");
            let args: Vec<(String, Value)> = match sig.and_then(|s| s.get("args")) {
                Some(Value::Object(a)) => {
                    a.iter().filter(|(k, _)| !c.descriptor.keys.contains_key(k)).cloned().collect()
                }
                _ => vec![],
            };
            let returns = sig.and_then(|s| s.get("returns")).cloned().unwrap_or(Value::Null);
            // A method's `description` comes from the catalog entry `{name}/{m}` when it is 1 to 1,024
            // bytes (16.8); otherwise the member is absent.
            let mut members = vec![("args".into(), Value::Object(args))];
            if let Some(text) =
                self.config.catalog.get(&format!("{name}/{m}")).filter(|t| (1..=1024).contains(&t.len()))
            {
                members.push(("description".into(), Value::String(text.clone())));
            }
            members.push(("fn".into(), Value::String(format!("{name}/{m}"))));
            members.push(("fn_hash".into(), Value::String(hash.clone())));
            members.push(("returns".into(), returns));
            methods.push((m.clone(), Value::Object(members)));
        }
        let doc = Value::Object(vec![
            ("class".into(), Value::String(name.into())),
            ("class_hash".into(), Value::String(c.class_hash.clone())),
            ("descriptor".into(), descriptor),
            ("kind".into(), Value::String("class".into())),
            ("methods".into(), Value::Object(methods)),
            ("publisher".into(), Value::String(publisher_of(name).into())),
        ]);
        Reply::json(200, doc.to_jcs(), Cache::Public(self.config.ttl.latest_max_age.as_secs()))
    }

    /// The object answer for a key encoding: the class bound now, the keys checked against it.
    fn object_by_key(&self, key: ObjectKey, now: &str) -> Reply {
        let c = match Bindings::class(self, &key.class, now) {
            Ok(Some(c)) => c,
            Ok(None) => return not_found(),
            Err(code) => return envelope::error_reply(code),
        };
        let d = &c.descriptor;
        let bad = |code| envelope::bad_request_reply(code);
        if key.keys.len() != d.keys.len() {
            return bad(ErrorCode::KeyEncoding);
        }
        for (name, decl) in &d.keys {
            let Some(v) = key.keys.get(name) else { return bad(ErrorCode::KeyEncoding) };
            match (decl, v) {
                (ClassKey::Class(want), KeyValue::Object(o)) if o.class == *want => {}
                (ClassKey::Scalar { ty, key_normalisation, .. }, KeyValue::Scalar(s)) if s.ty() == *ty => {
                    if let Some(rule) = key_normalisation {
                        if let Err(e) = rule.check(s.lexical()) {
                            return bad(e.code());
                        }
                    }
                }
                _ => return bad(ErrorCode::KeyEncoding),
            }
        }
        self.object_answer(&key, &c.class_hash)
    }

    /// The object answer (16.8) for `key` of the class with hash `class_hash`.
    fn object_answer(&self, key: &ObjectKey, class_hash: &str) -> Reply {
        let class: Option<Arc<ClassDescriptor>> =
            self.store.class(class_hash).ok().flatten().and_then(|b| ClassDescriptor::from_json(&b).ok()).map(Arc::new);
        let Some(class) = class else { return envelope::error_reply(ErrorCode::InputUnavailable) };
        let (Ok(object), Ok(id)) = (key.object_value(class_hash), key.object_id(class_hash)) else {
            return bad_key();
        };
        let Ok(short) = short_id(&id) else { return bad_key() };
        let doc = Value::Object(vec![
            ("call".into(), Value::String(key.call_form())),
            ("kind".into(), Value::String("object".into())),
            ("methods".into(), Value::Array(class.methods.keys().map(|m| Value::String(m.clone())).collect())),
            ("object".into(), object),
            ("object_id".into(), Value::String(id)),
            ("short_id".into(), Value::String(short)),
        ]);
        Reply::json(200, doc.to_jcs(), Cache::Public(self.config.ttl.latest_max_age.as_secs()))
    }
}

/// The publisher of a name: the host of its namespace, its first segment (spec-0.6.1, 2.3, 16.8).
pub(crate) fn publisher_of(name: &str) -> &str {
    name.split('/').next().unwrap_or(name)
}

fn bad_key() -> Reply {
    envelope::bad_request_reply(ErrorCode::KeyEncoding)
}

fn not_found() -> Reply {
    let body = Value::Object(vec![("error".into(), Value::String("no such class or object".into()))]);
    Reply::json(404, body.to_jcs(), Cache::Public(60))
}
