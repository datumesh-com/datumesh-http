//! describe (SPEC 16.8, 9.6) and short ids.
//!
//! `GET /describe/{id}`: `{id}` is a class name, an object id or a short id (12 hex digits), or an
//! object's key encoding (16.5); never a class hash. A cell id names no object (404). A class answers with the
//! `describe-class` document, an object with the `describe-object` one.
//!
//! # Known objects
//!
//! Objects are never stored, so the objects the mesh *knows* (the ones an object id or a short id can
//! name) are the objects of the declared list of each class the mesh has heard of: the stored cells of
//! the class's `extent.keys` function (or of its first method in name order, when it declares none), read
//! in pages of [`INDEX_PAGE`] by sort key (`slice_page`, CONTRACT-0.5.1 H1). A class holds at most `DATUMESH_INDEX_CLASS_MAX` objects and the
//! index at most `DATUMESH_INDEX_MAX`; past a cap the class is cut (its first objects in cell-id order are
//! kept, the cut is logged and counted in `datumesh_object_index_truncated_total`), and its other objects are
//! reached by key encoding. Computed cells are not read: a computed cell (and a stored version added to the
//! mesh) is put into the last build at once.
//!
//! The index is kept in memory, as a map of object id to object. A thread of the mesh builds it at start
//! (after `DATUMESH_INDEX_FIRST_BUILD_DELAY_SECS`) and then every `DATUMESH_INDEX_SECS`. Between two full builds
//! the same thread, on each tick ([`INDEX_TICK`]), indexes each class the mesh has heard of that the last build
//! does not hold (the `held` record of [`Built`]): that class only, within the same caps (the total cap counts
//! the objects the build already holds), merged into the last build. A class whose index came out empty is
//! held too, so it is not read again. Nothing is indexed on a request. A request for an object id reads the
//! last build and never builds one, nor reads a cell: before the first build it is `503` with `Retry-After` and
//! `no-store`; a hex id the build does not hold is `404` with the reason "not in the object index yet". A
//! describe of a class reads its descriptor only and hears of the class for the next index tick.
//!
//! The classes the index covers are the ones the mesh has *heard of*: a class a stored version or a
//! computed cell was added under, a class a request or a describe resolved, and the classes of the configured
//! aliases. A class published by another process and never named to this mesh is unknown until one of those
//! happens.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use datumesh_core::json::{self, Value};
use datumesh_core::{
    lexical, parse_object_key, short_id, Arg, Cell, ClassDescriptor, ClassKey, DescribeTests, ErrorCode,
    FunctionAnswer, KeyValue, ObjectKey, Scalar, Slice,
};
use datumesh_store::CatalogKind;

use crate::call::{Bindings, BoundClass};
use crate::envelope::{self, Cache, Reply};
use crate::mesh::Inner;

/// How often the index thread looks at the index (it builds one when it is due).
/// The most extension methods `describe` of a class lists (R11).
pub(crate) const EXTENSIONS_SHOWN: usize = 100;

pub(crate) const INDEX_TICK: Duration = Duration::from_millis(500);

/// The objects one page of a class's declared list reads from the store (`slice_page`; CONTRACT-0.5.1 H2: a page is 1,000).
pub(crate) const INDEX_PAGE: usize = 1_000;

/// An object the mesh knows. Its class hash is shared by every object of the class.
#[derive(Clone, Debug)]
struct Known {
    key: ObjectKey,
    class_hash: Arc<str>,
}

/// The index of known objects.
#[derive(Default)]
pub(crate) struct Index {
    /// The class names the next build holds (heard of by a stored version, a computed cell, an alias, or a describe).
    classes: BTreeSet<String>,
    built: Option<Built>,
}

pub(crate) struct Built {
    at: Instant,
    /// Object id to object, in id order (a short id is a range read).
    objects: BTreeMap<String, Known>,
    /// The classes this build has indexed (an empty one too): a heard-of class not here is indexed on the next tick.
    held: BTreeSet<String>,
}

impl Index {
    /// A class the mesh has heard of: the next build holds its objects. Nothing is indexed now.
    pub(crate) fn hear_of_class(&mut self, name: &str) {
        self.classes.insert(name.to_string());
    }
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

/// The slice of every cell of the function `name_of` (`<class>/<method>`, hash `fn_hash`, descriptor `d`), every
/// argument open.
fn method_slice(name_of: &str, fn_hash: &str, d: &datumesh_core::FunctionDescriptor) -> Option<Slice> {
    let (ns, f) = name_of.rsplit_once('/')?;
    let args: datumesh_core::Args = d.signature.args.keys().map(|k| (k.clone(), Arg::Slice)).collect();
    let cell = Cell { ns: ns.into(), fn_name: f.into(), fn_hash: fn_hash.into(), args };
    Slice::from_value(&cell.to_value()).ok()
}

impl Inner {
    /// Remembers a class name for the index.
    pub(crate) fn hear_of_class(&self, name: &str) {
        self.known.lock().unwrap_or_else(|p| p.into_inner()).hear_of_class(name);
    }

    /// The index thread's tick: builds the index of known objects as of `now` when it is missing or older than
    /// `DATUMESH_INDEX_SECS`; otherwise indexes the heard-of classes the last build does not hold and merges them
    /// into it. A request never calls this. The store is read without the lock held, so a request is not held
    /// up by a build.
    pub(crate) fn refresh_index(&self) {
        let (mut classes, missing) = {
            let ix = self.known.lock().unwrap_or_else(|p| p.into_inner());
            match ix.built.as_ref() {
                Some(b) if b.at.elapsed() <= self.config.index_secs => {
                    let missing: Vec<String> = ix.classes.iter().filter(|c| !b.held.contains(*c)).cloned().collect();
                    if missing.is_empty() {
                        return;
                    }
                    (BTreeSet::new(), Some((missing, b.objects.len())))
                }
                _ => (ix.classes.clone(), None),
            }
        };
        let now = self.now();
        match missing {
            Some((missing, base)) => self.add_classes(&missing, base, &now),
            None => {
                let built = self.build_index(&mut classes, &now);
                let mut ix = self.known.lock().unwrap_or_else(|p| p.into_inner());
                ix.classes.extend(classes);
                ix.built = Some(built);
            }
        }
    }

    /// Indexes the classes `names` (heard of, not held by the last build) with the same caps, and merges their
    /// objects into the last build. `base` is the number of objects the build holds (the total cap counts them).
    fn add_classes(&self, names: &[String], base: usize, now: &str) {
        let (class_max, index_max) = (self.config.index_class_max, self.config.index_max);
        let mut added = BTreeMap::new();
        for name in names {
            if self.index_class(name, now, class_max, index_max, base, &mut added) {
                self.metrics.index_truncated();
            }
        }
        let mut ix = self.known.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(b) = ix.built.as_mut() {
            for (id, k) in added {
                if b.objects.len() >= index_max {
                    break;
                }
                b.objects.entry(id).or_insert(k);
            }
            b.held.extend(names.iter().cloned());
        }
    }

    /// Builds the index from the classes heard of: each one's declared list, within the caps.
    fn build_index(&self, classes: &mut BTreeSet<String>, now: &str) -> Built {
        // The classes of the configured aliases are heard of from the start.
        for target in self.config.aliases.values() {
            if let Some((ns, _)) = target.rsplit_once('/') {
                classes.insert(ns.to_string());
            }
        }
        let mut objects = BTreeMap::new();
        let (class_max, index_max) = (self.config.index_class_max, self.config.index_max);
        for name in classes.iter() {
            if self.index_class(name, now, class_max, index_max, 0, &mut objects) {
                self.metrics.index_truncated();
            }
        }
        Built { at: Instant::now(), objects, held: classes.clone() }
    }

    /// The objects of the class `name` into `objects`, from its declared list: the stored cells of its
    /// `extent.keys` function, or of its first method in name order when it declares none. Read in pages of
    /// [`INDEX_PAGE`] by sort key, so the first objects of a class are the ones kept when it is cut. `base` is the
    /// number of objects already in the build, which count against `index_max` with `objects`. Returns whether
    /// the class was cut at `class_max` objects or at `index_max` objects in all.
    fn index_class(
        &self,
        name: &str,
        now: &str,
        class_max: usize,
        index_max: usize,
        base: usize,
        objects: &mut BTreeMap<String, Known>,
    ) -> bool {
        let Ok(Some(c)) = self.class_binding(name, now) else { return false };
        let d = &c.descriptor;
        let declared = d.extent.as_ref().and_then(|e| e.keys.as_deref());
        let method =
            declared.and_then(|h| d.methods.iter().find(|(_, v)| v.as_str() == h)).or_else(|| d.methods.iter().next());
        let Some((m, hash)) = method else { return false };
        let name_of = format!("{name}/{m}");
        let Ok(fd) = self.descriptor(hash) else { return false };
        let Some(slice) = method_slice(&name_of, hash, &fd) else { return false };
        let class_hash: Arc<str> = Arc::from(c.class_hash.as_str());
        let mut mine = 0usize;
        let mut after: Option<Vec<u8>> = None;
        loop {
            let page = match self.store.slice_page(&slice, now, after.as_deref(), INDEX_PAGE) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(target: "datumesh_http", error = %e, class = name, "describe index: slice_page");
                    return false;
                }
            };
            let n = page.len();
            after = page.last().map(|v| v.sort_key.clone());
            for v in &page {
                let Some(object) = object_of(name, d, &v.cell.args) else { continue };
                let Ok(id) = object.object_id(&c.class_hash) else { continue };
                if objects.contains_key(&id) {
                    continue;
                }
                if mine == class_max || base + objects.len() >= index_max {
                    tracing::warn!(target: "datumesh_http", class = name, kept = mine,
                        "describe index: the class is cut at a cap (DATUMESH_INDEX_CLASS_MAX, DATUMESH_INDEX_MAX)");
                    return true;
                }
                objects.insert(id, Known { key: object, class_hash: class_hash.clone() });
                mine += 1;
            }
            if n < INDEX_PAGE {
                return false;
            }
        }
    }

    /// Puts the object of `cell` into the last build at once (O(1), within the caps): a formula cell just
    /// computed is known to describe straight away. The cell's class is heard of from now on; nothing is done
    /// before the first build (it will hold the class).
    pub(crate) fn index_cell_now(&self, cell: &Cell) {
        let class = cell.ns.clone();
        let now = self.now();
        let Ok(Some(c)) = self.class_binding(&class, &now) else { return };
        let Some(object) = object_of(&class, &c.descriptor, &cell.args) else { return };
        let Ok(id) = object.object_id(&c.class_hash) else { return };
        let index_max = self.config.index_max;
        let mut ix = self.known.lock().unwrap_or_else(|p| p.into_inner());
        ix.classes.insert(class);
        if let Some(b) = ix.built.as_mut() {
            if b.objects.contains_key(&id) || b.objects.len() < index_max {
                b.objects.insert(id, Known { key: object, class_hash: Arc::from(c.class_hash.as_str()) });
            }
        }
    }

    /// As [`Inner::index_cell_now`] for the stored version of `cell_id`, read from its receipt.
    pub(crate) fn index_stored_now(&self, cell_id: &str) {
        let far = "9999-12-31T23:59:59.999999Z";
        let Some((_, bytes)) = self.store.stored_asof_with_receipt(cell_id, far).ok().flatten() else { return };
        let Ok(r) = datumesh_core::Receipt::from_json(&bytes) else { return };
        self.index_cell_now(r.cell());
    }

    /// Runs `f` on the last build of the index, or `None` before the first build (a request never builds it).
    fn with_index<T>(&self, f: impl FnOnce(&Built) -> T) -> Option<T> {
        let ix = self.known.lock().unwrap_or_else(|p| p.into_inner());
        ix.built.as_ref().map(f)
    }

    /// The describe reply for `id` (see the module documentation).
    pub(crate) fn describe(&self, id: &str) -> Reply {
        // A listed name is described from its listing (CONTRACT-http-listing L6), before any binding.
        if let Some(l) = crate::listing::listed(&*self.store, id) {
            return crate::listing::describe_reply(&*self.store, &l);
        }
        let now = self.now();
        // The catalogue's root (CONTRACT-classes-round C7): one call, from the catalogue's last build.
        if id == format!("{}/catalog", self.config.host) {
            return crate::catalog_view::describe_root(self);
        }
        // A class name.
        if lexical::check_function_name(id, "$").is_ok() {
            return match Bindings::class(self, id, &now) {
                Ok(Some(c)) => {
                    self.hear_of_class(id);
                    self.class_answer(id, &c)
                }
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
        // An object by a label (K8, the classes round): `N[label]` answers the object's describe, with `resolved`.
        if label_form(id).is_some() {
            return self.describe_label(self, id, &now).unwrap_or_else(not_found);
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
            let answer = self.with_index(|b| match b.objects.get(id) {
                Some(k) => self.object_answer(&k.key, &k.class_hash),
                None => index_miss(),
            });
            return answer.unwrap_or_else(|| index_not_yet(self.config.index_secs.as_secs()));
        }
        if hex(id, 12) {
            let answer = self.with_index(|b| {
                // The objects whose id begins with `id`: a range read of the sorted map, the first two.
                let hits: Vec<&Known> = b
                    .objects
                    .range(id.to_string()..)
                    .take_while(|(o, _)| o.starts_with(id))
                    .take(2)
                    .map(|(_, k)| k)
                    .collect();
                match hits.as_slice() {
                    [] => index_miss(),
                    [k] => self.object_answer(&k.key, &k.class_hash),
                    _ => envelope::bad_request_reply(ErrorCode::ShortIdAmbiguous),
                }
            });
            return answer.unwrap_or_else(|| index_not_yet(self.config.index_secs.as_secs()));
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
        let unnamed = self.is_unnamed(fn_hash);
        let counts = crate::testrun::counts(&*self.store, &self.config.host, name, fn_hash, unnamed, &self.now());
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
        if let Some(relay) = crate::fnpage::relay_of(self, name, fn_hash) {
            if let Some(takeover) = relay.takeover {
                let source =
                    datumesh_core::DescribeSource { name: relay.name, relayed_by: self.config.host.clone(), takeover };
                if let Ok(a) = answer.clone().with_source(source) {
                    answer = a;
                }
            }
        }
        let body = self.with_facets(answer.to_json(), name, CatalogKind::Function);
        Reply::json(200, body, Cache::NoStore)
    }

    /// The body of a function or class answer with the entry's `facets` member, when the entry has facets (C2): an entry
    /// without them has no member.
    fn with_facets(&self, body: Vec<u8>, name: &str, kind: CatalogKind) -> Vec<u8> {
        let facets = self.store.catalog_entry_ext(kind, name).ok().flatten().map(|e| e.facets).unwrap_or_default();
        if facets.is_empty() {
            return body;
        }
        let Ok(Value::Object(mut members)) = json::parse(&body) else { return body };
        members.push(("facets".into(), facets_value(&facets)));
        Value::Object(members).to_jcs()
    }

    fn class_answer(&self, name: &str, c: &BoundClass) -> Reply {
        let Ok(descriptor) = json::parse(&c.descriptor.canonical()) else {
            return envelope::error_reply(ErrorCode::InputUnavailable);
        };
        // The parent chain, walked once for the whole class (H5): each ancestor's hash and descriptor, nearest first.
        let mut chain: Vec<(String, ClassDescriptor)> = Vec::new();
        let mut up = c.descriptor.parent.clone();
        while let Some(ph) = up {
            let Ok(Some(p)) = Bindings::class_by_hash(self, &ph) else { break };
            up = p.parent.clone();
            chain.push((ph, p));
        }
        // Each defining class's name, read once (H5).
        let mut names: BTreeMap<String, Option<String>> = BTreeMap::new();
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
            members.push(("fn".into(), Value::String(self.method_fn(name, c, m, &chain, &mut names))));
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
        let body = self.with_facets(doc.to_jcs(), name, CatalogKind::Class);
        Reply::json(200, body, Cache::Public(self.config.ttl.latest_max_age.as_secs()))
    }

    /// The object a label `N[label]` names (K8, 16.7): through `N`'s lookup, read as of `asof` (the request's, or `now`).
    /// The answer is the object's describe with `resolved` (the lookup). `None` when `id` is not the label form. An unknown
    /// or withdrawn label, and a class with no lookup, are `E_NAME_UNBOUND`.
    pub(crate) fn describe_label(&self, bindings: &dyn Bindings, id: &str, now: &str) -> Option<Reply> {
        let (class, written, asof) = label_form(id)?;
        let current = asof.is_none();
        let at = asof.unwrap_or_else(|| now.to_string());
        let unbound = || Some(envelope::name_unbound_reply(self.config.ttl.latest_max_age.as_secs()));
        let c = match bindings.class(&class, &at) {
            Ok(Some(c)) => c,
            Ok(None) => return unbound(),
            Err(code) => return Some(envelope::error_reply(code)),
        };
        let Some(lookup) = c.descriptor.lookup.clone() else { return unbound() };
        let folded = match bindings.binding(&format!("{class}/{lookup}"), &at) {
            Ok(Some(b))
                if matches!(
                    b.descriptor.arg_extras.get("label").and_then(|x| x.key_normalisation),
                    Some(datumesh_core::KeyNormalisation::LabelFold)
                ) =>
            {
                datumesh_core::fold_label(&written)
            }
            Ok(_) => written.clone(),
            Err(code) => return Some(envelope::error_reply(code)),
        };
        let enc = match bindings.lookup_read(&class, &folded, &at, current) {
            Ok(Some(enc)) if !enc.is_empty() => enc,
            Ok(_) => return unbound(),
            Err(code) => return Some(envelope::error_reply(code)),
        };
        let Ok(key) = datumesh_core::parse_object_key(&enc) else { return Some(bad_key()) };
        let Ok(resolution) = datumesh_core::Resolution::lookup(&written, &enc, &at) else { return Some(bad_key()) };
        let Ok(resolved) = datumesh_core::Resolved::new(&key.call_form(), vec![resolution]) else {
            return Some(envelope::error_reply(ErrorCode::InputUnavailable));
        };
        Some(self.object_by_key_with(key, &at, Some(resolved)))
    }

    /// The object answer for a key encoding: the class bound now, the keys checked against it.
    fn object_by_key(&self, key: ObjectKey, now: &str) -> Reply {
        self.object_by_key_with(key, now, None)
    }

    /// [`Inner::object_by_key`], with the request's `resolved` (a label's lookup) when there is one.
    fn object_by_key_with(&self, key: ObjectKey, now: &str, resolved: Option<datumesh_core::Resolved>) -> Reply {
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
        self.object_answer_with(&key, &c.class_hash, resolved)
    }

    /// The function a method `m` of the class `name` (bound as `c`) denotes (K5, 16.8): `D/m` when the method is inherited
    /// from the class D that defines it (the last class of the parent chain `chain`, nearest first, that has the method's
    /// hash in the unbroken run from the class), else `name/m`. `names` memoises the defining classes' names (H5).
    fn method_fn(
        &self,
        name: &str,
        c: &BoundClass,
        m: &str,
        chain: &[(String, ClassDescriptor)],
        names: &mut BTreeMap<String, Option<String>>,
    ) -> String {
        let Some(h) = c.descriptor.methods.get(m) else { return format!("{name}/{m}") };
        let mut defining: Option<&str> = None;
        for (ph, p) in chain {
            if p.methods.get(m) != Some(h) {
                break;
            }
            defining = Some(ph);
        }
        if let Some(dh) = defining {
            let dn = names
                .entry(dh.to_string())
                .or_insert_with(|| Bindings::class_name_of(self, dh, &self.now()).ok().flatten());
            if let Some(dn) = dn {
                return format!("{dn}/{m}");
            }
        }
        format!("{name}/{m}")
    }

    /// The object answer (16.8) for `key` of the class with hash `class_hash`.
    fn object_answer(&self, key: &ObjectKey, class_hash: &str) -> Reply {
        self.object_answer_with(key, class_hash, None)
    }

    /// [`Inner::object_answer`], with the request's `resolved` member when there is one.
    fn object_answer_with(
        &self,
        key: &ObjectKey,
        class_hash: &str,
        resolved: Option<datumesh_core::Resolved>,
    ) -> Reply {
        let class: Option<Arc<ClassDescriptor>> =
            self.store.class(class_hash).ok().flatten().and_then(|b| ClassDescriptor::from_json(&b).ok()).map(Arc::new);
        let Some(class) = class else { return envelope::error_reply(ErrorCode::InputUnavailable) };
        let (Ok(object), Ok(id)) = (key.object_value(class_hash), key.object_id(class_hash)) else {
            return bad_key();
        };
        let Ok(short) = short_id(&id) else { return bad_key() };
        let mut members = vec![
            ("call".into(), Value::String(key.call_form())),
            ("kind".into(), Value::String("object".into())),
            ("methods".into(), Value::Array(class.methods.keys().map(|m| Value::String(m.clone())).collect())),
            ("object".into(), object),
            ("object_id".into(), Value::String(id)),
            ("short_id".into(), Value::String(short)),
        ];
        // The functions that extend the object's class (K9, 16.8), by name; an answer with none has no member.
        let extensions = self.extensions(&key.class);
        if !extensions.is_empty() {
            members.push(("extensions".into(), Value::Array(extensions)));
        }
        if let Some(r) = resolved {
            // `resolved` as an answer writes it (16.7): core's member, which the envelope takes for any answer.
            let member = datumesh_core::Answer::not_covered()
                .with_resolved(r)
                .ok()
                .and_then(|a| a.to_value().get("resolved").cloned());
            if let Some(m) = member {
                members.push(("resolved".into(), m));
            }
        }
        let doc = Value::Object(members);
        Reply::json(200, doc.to_jcs(), Cache::Public(self.config.ttl.latest_max_age.as_secs()))
    }
}

impl Inner {
    /// The extension methods of the class `class` (K9): `{fn, fn_hash, publisher}` for each function whose catalogue entry
    /// extends it, by name, with the function hash it is bound to now. Empty when none does, or the store cannot say.
    fn extensions(&self, class: &str) -> Vec<Value> {
        // At most 100, the first by name (R11): the store reads that many.
        let Ok(list) = self.store.extensions_of_limit(class, EXTENSIONS_SHOWN) else { return Vec::new() };
        let now = self.now();
        let mut out = Vec::new();
        for e in list {
            let f = e.entry.name.clone();
            let Ok(Some(b)) = self.store.resolve_binding(&f, &now) else { continue };
            out.push(Value::Object(vec![
                ("fn".into(), Value::String(f.clone())),
                ("fn_hash".into(), Value::String(b.fn_hash)),
                ("publisher".into(), Value::String(publisher_of(&f).into())),
            ]));
        }
        out
    }
}

/// The label form `N[label]` (K8): the class name, the label as written (a quoted label unquoted), and the `asof` of a query
/// `?asof=<instant>`, when the id is one; `None` otherwise (a call with a method ends with `)`).
pub(crate) fn label_form(id: &str) -> Option<(String, String, Option<String>)> {
    let (path, query) = match id.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (id, None),
    };
    let open = path.find('[')?;
    let inner = path[open + 1..].strip_suffix(']')?;
    let class = &path[..open];
    if class.is_empty() || inner.is_empty() {
        return None;
    }
    let written = match inner.strip_prefix('"') {
        Some(q) => q.strip_suffix('"')?.replace("\"\"", "\""),
        None => inner.to_string(),
    };
    let asof = query.and_then(|q| q.strip_prefix("asof=")).map(|a| crate::instant::canonical_or_same(a, true));
    Some((class.to_string(), written, asof))
}

/// A catalogue entry's facets as a JSON object: each key to its names.
pub(crate) fn facets_value(facets: &datumesh_store::Facets) -> Value {
    Value::Object(
        facets
            .iter()
            .map(|(k, vs)| (k.clone(), Value::Array(vs.iter().map(|v| Value::String(v.clone())).collect())))
            .collect(),
    )
}

/// The publisher of a name: the host of its namespace, its first segment (spec-0.6.1, 2.3, 16.8).
pub(crate) fn publisher_of(name: &str) -> &str {
    name.split('/').next().unwrap_or(name)
}

fn bad_key() -> Reply {
    envelope::bad_request_reply(ErrorCode::KeyEncoding)
}

/// `503` for an object id before the index's first build.
fn index_not_yet(secs: u64) -> Reply {
    envelope::not_yet_reply(secs, "The index of objects")
}

/// `404` for a hex id the last build does not hold: the reason says so (CONTRACT-round-0.7-P HP1).
fn index_miss() -> Reply {
    let body = Value::Object(vec![
        ("error".into(), Value::String("no such object".into())),
        ("reason".into(), Value::String("not in the object index yet".into())),
    ]);
    Reply::json(404, body.to_jcs(), Cache::NoStore)
}

fn not_found() -> Reply {
    let body = Value::Object(vec![("error".into(), Value::String("no such class or object".into()))]);
    Reply::json(404, body.to_jcs(), Cache::Public(60))
}
