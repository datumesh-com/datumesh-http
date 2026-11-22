//! The catalogue job (CONTRACT-classes-round C3 to C5): `datumesh-mesh catalog-build` writes the classes `catalog/subject` and
//! `catalog/facet` (their methods inline, K7), and the cells of their objects, as stored cells signed by the mesh's key with the
//! licence the job is given; and the rule text (`catalog/rule()`, the page `/catalog/rule`).
//!
//! The job reads the store's catalogue entries that have facets (as of its clock). A rerun over an unchanged catalogue writes
//! nothing: each cell line has no `tx`, so publish's skip rule keeps a cell whose latest version states the same thing. A subject
//! that disappears gets `exists()` false, its `label()` and `sources()` zero or empty, and its labels withdrawn (`""`, K8); a
//! facet value that disappears gets `exists()` false.
//!
//! # The rank (C5)
//!
//! Per subject, each entry of the subject is level 1, 2 or 3:
//! * a *relay* is a name `<host>/relay/<domain>/...`; its domain is `<domain>` (level 2);
//! * a *view* is a function or class that is not a relay and whose inputs read a relay directly or through other views, to any
//!   depth, through any named function or class (level 2). Its source domains are the domains of the relays it reaches;
//! * the subject's source domains are the domains of its relays and of the relays its views reach;
//! * any other entry is level 1 when its publisher's host (the first segment of its name) equals one of those domains or is a
//!   sub-domain of one; when the subject has no source domain, every such entry is level 1 (its own host is its source domain);
//! * otherwise level 3.
//!
//! Within a level: the instant the name was first bound (a class's own class binding), then the name, ascending. The rank one
//! source is the first of that order, and `source(k)` is the k-th.
//!
//! # What is written, and what it is called
//!
//! * the class `<host>/catalog/subject`: keys `domain`, `thing`, `measure`, `region` (`none`), in that order; the lookup
//!   `from_label` (`label_fold`); the methods `exists`, `label`, `sources`, `source(rank)` (`rank` a number of unit `1`, default
//!   `1`, K13), `from_label`; the extent `keys` is `exists`;
//! * the class `<host>/catalog/facet`: keys `facet`, `value`; the methods `exists`, `label`, `subjects`; the extent `keys` is
//!   `exists`;
//! * the function `<host>/catalog/rule` (no argument, a blob: the text of the rule).
//!
//! The labels of a subject: `label()` is its facet values with `_` as a space, measure, thing, region, domain, joined by
//! ` · `, a `none` value left out, the region written out (`ea` euro area, `gb` United Kingdom, `us` United States, `world`
//! world). The lookup's labels are the folded `label()`, and `<measure> <region>` and `<thing> <measure> <region>` (folded), each
//! only when it names exactly one subject; a label that names two is not stored.
//!
//! Choices where the contract is silent (each is in the report): a subject needs all four keys (an entry without one has no
//! subjects); `subjects()` of a `frequency` value is the number of subjects of the entries that carry it.

use std::collections::{BTreeMap, BTreeSet};

use datumesh_core::{fold_label, Arg, ClassDescriptor, FunctionDescriptor, Receipt, SigningKey};
use datumesh_store::{Facets, Store};
use serde_json::{json, Value};

use crate::publish::{publish_job, PublishError, PublishOptions};

/// The rule of the catalogue (C5): the text of the page `/catalog/rule` and of the blob `catalog/rule()`.
pub(crate) const RULE: &str = "How the catalogue ranks the sources of a subject. \
A source is level one when it is published under the domain of the data: its publisher host is that domain or one of its parent \
domains. A source is level two when it is a relay of the data, a name under the relay path of the domain of the data and trusted \
as relayed, or a view built on such a relay: a function or a class whose inputs read a relay, directly or through other views. \
Every other source is level three. Within a level the source whose name was first bound comes first. A tie on the time of the \
first binding is broken by the name, in ascending order. The rank one source of a subject is the first source in this order, and \
source of rank k is the k-th. A subject with no relay and no view has no source domain, so its sources are level one under their \
own hosts. The job that writes the catalogue applies this rule to the facets the publishers give.";

/// The file name of the rule text in the job's blobs.
const RULE_FILE: &str = "rule.txt";

/// The job's result: `cells` is the number of cell versions the run wrote (the classes, methods and bindings are not counted), and
/// `subjects` the subjects that exist now.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CatalogBuilt {
    /// Cell versions written by the run.
    pub cells: u64,
    /// Subjects that exist as of the run.
    pub subjects: u64,
}

/// A subject: its four keys, `(domain, thing, measure, region)`.
pub(crate) type SubjectKey = (String, String, String, String);

/// `<host>/catalog/subject`.
pub(crate) fn subject_class(host: &str) -> String {
    format!("{host}/catalog/subject")
}

/// `<host>/catalog/facet`.
pub(crate) fn facet_class(host: &str) -> String {
    format!("{host}/catalog/facet")
}

/// `<host>/catalog/rule`.
pub(crate) fn rule_fn(host: &str) -> String {
    format!("{host}/catalog/rule")
}

/// The key encoding of a subject (16.5: the names in ascending order, no space).
pub(crate) fn subject_key_text(host: &str, k: &SubjectKey) -> String {
    format!(r#"{}(domain:"{}",measure:"{}",region:"{}",thing:"{}")"#, subject_class(host), k.0, k.2, k.3, k.1)
}

/// The subject a key encoding names, when it is one of this host's subjects written as [`subject_key_text`] writes it.
pub(crate) fn parse_subject_key(host: &str, text: &str) -> Option<SubjectKey> {
    let body = text.strip_prefix(&format!("{}(", subject_class(host)))?.strip_suffix(')')?;
    let mut got: BTreeMap<&str, &str> = BTreeMap::new();
    for part in body.split(',') {
        let (k, v) = part.split_once(':')?;
        let v = v.strip_prefix('"')?.strip_suffix('"')?;
        if v.contains('"') || got.insert(k, v).is_some() {
            return None;
        }
    }
    if got.len() != 4 {
        return None;
    }
    Some((
        got.get("domain")?.to_string(),
        got.get("thing")?.to_string(),
        got.get("measure")?.to_string(),
        got.get("region")?.to_string(),
    ))
}

/// A subject's words as its label (C3, C4): `_` as a space, the region written out, `none` left out.
pub(crate) fn region_text(code: &str) -> String {
    match code {
        "ea" => "euro area".into(),
        "gb" => "United Kingdom".into(),
        "us" => "United States".into(),
        other => other.replace('_', " "),
    }
}

/// The label of a subject: measure, thing, region, domain, joined by ` · `.
pub(crate) fn subject_label(k: &SubjectKey) -> String {
    let parts = [k.2.replace('_', " "), k.1.replace('_', " "), region_text(&k.3), k.0.replace('_', " ")];
    parts.into_iter().filter(|p| p != "none").collect::<Vec<_>>().join(" \u{b7} ")
}

/// The label of a facet object: its value with `_` as a space; a region's code written out.
pub(crate) fn facet_label(facet: &str, value: &str) -> String {
    if facet == "region" {
        region_text(value)
    } else {
        value.replace('_', " ")
    }
}

/// The folded lookup labels of a subject: the folded `label()`, and `<measure> <region>` and `<thing> <measure> <region>` where
/// the region is not `none`, and the thing is not `none` for the last.
fn lookup_labels(k: &SubjectKey) -> Vec<String> {
    let mut out = vec![fold_label(&subject_label(k))];
    if k.3 != "none" {
        let m = k.2.replace('_', " ");
        out.push(fold_label(&format!("{m} {}", region_text(&k.3))));
        if k.1 != "none" {
            out.push(fold_label(&format!("{} {m} {}", k.1.replace('_', " "), region_text(&k.3))));
        }
    }
    out
}

/// The relay domain of a name `<host>/relay/<domain>/<more>`.
pub(crate) fn relay_domain(host: &str, name: &str) -> Option<String> {
    let rest = name.strip_prefix(host)?.strip_prefix('/')?;
    let mut it = rest.split('/');
    if it.next()? != "relay" {
        return None;
    }
    let domain = it.next()?;
    it.next()?;
    Some(domain.to_string())
}

/// One cell line of the job: a stored cell of a method, its arguments as typed values, its value and its licence.
fn cell_line(
    fn_name: &str,
    args: Vec<(&str, &str, Value)>,
    value: (&str, Value),
    licence: &str,
    summary: &str,
) -> String {
    let a: serde_json::Map<String, Value> =
        args.into_iter().map(|(k, t, v)| (k.to_string(), json!({"type": t, "value": v}))).collect();
    json!({"cell": {"fn": fn_name, "args": a}, "value": {"type": value.0, "value": value.1},
        "licence": licence, "summary": summary})
    .to_string()
}

/// The key arguments of a method: strings that are not normalised (an absent `key_normalisation` is `none`, 16.4), not valid
/// times.
fn key_args(keys: &[&str]) -> serde_json::Map<String, Value> {
    keys.iter().map(|k| (k.to_string(), json!({"type": "string", "valid": false}))).collect()
}

/// The keys of a class as its descriptor has them.
fn class_keys(keys: &[&str]) -> serde_json::Map<String, Value> {
    keys.iter().map(|k| (k.to_string(), json!({"key_normalisation": "none", "type": "string"}))).collect()
}

fn stored_hash(sig: &Value) -> String {
    let text = json!({"code": {"body": "", "lang": "stored"}, "inputs": [], "signature": sig}).to_string();
    FunctionDescriptor::from_json(text.as_bytes()).expect("a stored descriptor").hash()
}

/// The class `subject`'s descriptor, as an author writes it (K7: the methods inline, the extent is `exists`).
fn subject_class_line(host: &str) -> String {
    let class = subject_class(host);
    let keys = ["domain", "thing", "measure", "region"];
    let mut rank = key_args(&keys);
    rank.insert("rank".into(), json!({"type": "number", "unit": "1", "valid": false, "default": "1"}));
    let mut order_rank: Vec<&str> = keys.to_vec();
    order_rank.push("rank");
    let exists = json!({"args": key_args(&keys), "order": keys, "returns": "boolean"});
    let label = json!({"args": key_args(&keys), "order": keys, "returns": "string"});
    let sources = json!({"args": key_args(&keys), "order": keys, "returns": {"type": "number", "unit": "1"}});
    let source = json!({"args": rank, "order": order_rank, "returns": "string"});
    let from_label = json!({"args": {"label": {"key_normalisation": "label_fold", "type": "string", "valid": false}},
        "order": ["label"], "returns": {"classes": [class], "type": "string"}});
    let extent = stored_hash(&exists);
    let descriptor = json!({
        "keys": class_keys(&keys),
        "order": keys,
        "lookup": "from_label",
        "methods": {"exists": {"stored": exists}, "from_label": {"stored": from_label}, "label": {"stored": label},
            "source": {"stored": source}, "sources": {"stored": sources}},
        "extent": {"keys": extent},
        "steward": {"namespace": host, "process": format!("https://{host}/classes/change")},
    });
    json!({"class": class, "descriptor": descriptor}).to_string()
}

/// The class `facet`'s descriptor (C4).
fn facet_class_line(host: &str) -> String {
    let class = facet_class(host);
    let keys = ["facet", "value"];
    let exists = json!({"args": key_args(&keys), "order": keys, "returns": "boolean"});
    let label = json!({"args": key_args(&keys), "order": keys, "returns": "string"});
    let subjects = json!({"args": key_args(&keys), "order": keys, "returns": {"type": "number", "unit": "1"}});
    let extent = stored_hash(&exists);
    let descriptor = json!({
        "keys": class_keys(&keys),
        "order": keys,
        "methods": {"exists": {"stored": exists}, "label": {"stored": label}, "subjects": {"stored": subjects}},
        "extent": {"keys": extent},
        "steward": {"namespace": host, "process": format!("https://{host}/classes/change")},
    });
    json!({"class": class, "descriptor": descriptor}).to_string()
}

/// The function `catalog/rule` (no argument, a blob).
fn rule_line(host: &str) -> String {
    json!({"function": rule_fn(host), "signature": {"args": {}, "order": [], "returns": "blob"}}).to_string()
}

/// The entries of the graph: what each name reads, and the relays it reaches (C5), memoised.
struct Graph<'a, S: ?Sized> {
    store: &'a S,
    host: &'a str,
    now: &'a str,
    reads: BTreeMap<String, Vec<String>>,
    reach: BTreeMap<String, BTreeSet<String>>,
    visiting: BTreeSet<String>,
}

fn err<E: std::fmt::Display>(e: E) -> PublishError {
    PublishError { line: 0, detail: format!("catalogue: {e}"), kind: crate::publish::PublishErrorKind::Line }
}

impl<S: Store + ?Sized> Graph<'_, S> {
    /// The names a name reads: a function's inputs, or a class's methods' inputs.
    fn reads_of(&mut self, name: &str) -> Result<Vec<String>, PublishError> {
        if let Some(r) = self.reads.get(name) {
            return Ok(r.clone());
        }
        let mut out: BTreeSet<String> = BTreeSet::new();
        let mut fns: Vec<String> = Vec::new();
        if let Some(b) = self.store.resolve_binding(name, self.now).map_err(err)? {
            fns.push(b.fn_hash);
        } else if let Some(c) = self.store.resolve_class(name, self.now).map_err(err)? {
            if let Some(bytes) = self.store.class(&c).map_err(err)? {
                let cd = ClassDescriptor::from_json(&bytes).map_err(err)?;
                fns.extend(cd.methods.values().cloned());
            }
        }
        for h in fns {
            if let Some(bytes) = self.store.descriptor(&h) {
                let fd = FunctionDescriptor::from_json(&bytes).map_err(err)?;
                out.extend(fd.inputs);
            }
        }
        let out: Vec<String> = out.into_iter().collect();
        self.reads.insert(name.to_string(), out.clone());
        Ok(out)
    }

    /// The relay domains a name reaches: its own, for a relay; else those of what it reads (through views, to any depth).
    fn reach(&mut self, name: &str) -> Result<BTreeSet<String>, PublishError> {
        if let Some(r) = self.reach.get(name) {
            return Ok(r.clone());
        }
        if !self.visiting.insert(name.to_string()) {
            return Ok(BTreeSet::new());
        }
        let mut out = BTreeSet::new();
        if let Some(d) = relay_domain(self.host, name) {
            out.insert(d);
        } else {
            for m in self.reads_of(name)? {
                out.extend(self.reach(&m)?);
            }
        }
        self.visiting.remove(name);
        self.reach.insert(name.to_string(), out.clone());
        Ok(out)
    }
}

/// An entry of the catalogue with a binding, as the rank sees it.
struct Entry {
    name: String,
    first: String,
    relay: bool,
    view: bool,
    reach: BTreeSet<String>,
}

/// The live part of the catalogue: its subjects with their sources in rank order, and the facet objects with their subjects.
struct Model {
    subjects: BTreeMap<SubjectKey, Vec<String>>,
    facets: BTreeMap<(String, String), BTreeSet<SubjectKey>>,
}

fn model<S: Store + ?Sized>(store: &S, host: &str, now: &str) -> Result<Model, PublishError> {
    let mut g = Graph { store, host, now, reads: BTreeMap::new(), reach: BTreeMap::new(), visiting: BTreeSet::new() };
    let listed = store.catalog_entries_with_facets().map_err(err)?;
    let mut entries: Vec<Entry> = Vec::new();
    let mut subjects_of: Vec<Vec<SubjectKey>> = Vec::new();
    let mut facets: BTreeMap<(String, String), BTreeSet<SubjectKey>> = BTreeMap::new();
    let mut members: BTreeMap<SubjectKey, Vec<usize>> = BTreeMap::new();
    for e in &listed {
        let keys = subject_keys(&e.facets);
        let relay = relay_domain(host, &e.name).is_some();
        let reach = if e.first_bound.is_some() { g.reach(&e.name)? } else { BTreeSet::new() };
        let view = !relay && !reach.is_empty();
        let idx = entries.len();
        entries.push(Entry {
            name: e.name.clone(),
            first: e.first_bound.clone().unwrap_or_default(),
            relay,
            view,
            reach,
        });
        let bound = e.first_bound.is_some();
        // A facet value's subjects: those of the entry that have the value under its facet (a frequency is no key, so every
        // subject of an entry that carries it).
        for (facet, values) in &e.facets {
            for v in values {
                let set = facets.entry((facet.clone(), v.clone())).or_default();
                set.extend(keys.iter().filter(|k| key_has(facet, v, k)).cloned());
            }
        }
        if bound {
            for k in &keys {
                members.entry(k.clone()).or_default().push(idx);
            }
        }
        subjects_of.push(keys);
    }
    let mut subjects = BTreeMap::new();
    for (k, idxs) in &members {
        let mut domains: BTreeSet<String> = BTreeSet::new();
        for &i in idxs {
            let e = &entries[i];
            if e.relay || e.view {
                domains.extend(e.reach.iter().cloned());
            }
        }
        let mut ranked: Vec<(u8, &str, &str)> = idxs
            .iter()
            .map(|&i| {
                let e = &entries[i];
                let level = if e.relay || e.view {
                    2
                } else if domains.is_empty() {
                    1
                } else {
                    let h = e.name.split('/').next().unwrap_or("");
                    if domains.iter().any(|d| h == d || h.ends_with(&format!(".{d}"))) {
                        1
                    } else {
                        3
                    }
                };
                (level, e.first.as_str(), e.name.as_str())
            })
            .collect();
        ranked.sort();
        subjects.insert(k.clone(), ranked.into_iter().map(|(_, _, n)| n.to_string()).collect());
    }
    // Facet objects count only the subjects that exist (every subject of an entry is one of `members` when it is bound).
    facets.retain(|_, set| {
        set.retain(|k| subjects.contains_key(k));
        true
    });
    Ok(Model { subjects, facets })
}

/// Whether a subject has the value `v` under the facet `facet`: its domain, thing, measure or region is `v`. A frequency is no
/// key of a subject, so every subject of an entry that carries it has it.
fn key_has(facet: &str, v: &str, k: &SubjectKey) -> bool {
    match facet {
        "domain" => k.0 == v,
        "thing" => k.1 == v,
        "measure" => k.2 == v,
        "region" => k.3 == v,
        _ => true,
    }
}

/// The subjects of an entry's facets: the cross product of its domain, thing, measure and region names (C3).
fn subject_keys(facets: &Facets) -> Vec<SubjectKey> {
    let get = |k: &str| facets.get(k).cloned().unwrap_or_default();
    let (d, t, m, r) = (get("domain"), get("thing"), get("measure"), get("region"));
    let mut out = Vec::new();
    for dd in &d {
        for tt in &t {
            for mm in &m {
                for rr in &r {
                    out.push((dd.clone(), tt.clone(), mm.clone(), rr.clone()));
                }
            }
        }
    }
    out
}

/// One stored cell of a catalogue method: its args' lexical values, and the value's lexical form.
pub(crate) type CellRow = (BTreeMap<String, String>, String);

/// The receipts of a method's stored cells, as the args' lexical values and the value's lexical form.
pub(crate) fn cells_of<S: Store + ?Sized>(
    store: &S,
    class: &str,
    class_hash: Option<&str>,
    method: &str,
) -> Result<Vec<CellRow>, PublishError> {
    let Some(hash) = class_hash else { return Ok(Vec::new()) };
    let bytes = store.class(hash).map_err(err)?.ok_or_else(|| err(format!("class {hash} is not stored")))?;
    let cd = ClassDescriptor::from_json(&bytes).map_err(err)?;
    let Some(fh) = cd.methods.get(method) else { return Ok(Vec::new()) };
    let ns = class.to_string();
    let rows = store.stored_receipts_of(&ns, method, fh, usize::MAX).map_err(err)?;
    let mut out = Vec::with_capacity(rows.len());
    for bytes in rows {
        let r = Receipt::from_json(&bytes).map_err(err)?;
        let mut args = BTreeMap::new();
        for (k, a) in &r.cell().args {
            if let Arg::Value(s) = a {
                args.insert(k.clone(), s.lexical().to_string());
            }
        }
        out.push((args, r.value().lexical().to_string()));
    }
    Ok(out)
}

pub(crate) fn subject_of_args(args: &BTreeMap<String, String>) -> Option<SubjectKey> {
    Some((
        args.get("domain")?.clone(),
        args.get("thing")?.clone(),
        args.get("measure")?.clone(),
        args.get("region")?.clone(),
    ))
}

/// What the store holds from earlier runs: the subjects, their highest rank, the labels and the facet objects.
#[derive(Default)]
struct Known {
    subjects: BTreeSet<SubjectKey>,
    ranks: BTreeMap<SubjectKey, u64>,
    labels: BTreeSet<String>,
    facets: BTreeSet<(String, String)>,
}

fn known<S: Store + ?Sized>(store: &S, host: &str, now: &str) -> Result<Known, PublishError> {
    let subject_hash = store.resolve_class(&subject_class(host), now).map_err(err)?;
    let facet_hash = store.resolve_class(&facet_class(host), now).map_err(err)?;
    let mut k = Known::default();
    for (args, _) in cells_of(store, &subject_class(host), subject_hash.as_deref(), "exists")? {
        if let Some(s) = subject_of_args(&args) {
            k.subjects.insert(s);
        }
    }
    for (args, _) in cells_of(store, &subject_class(host), subject_hash.as_deref(), "source")? {
        if let (Some(s), Some(r)) = (subject_of_args(&args), args.get("rank").and_then(|r| r.parse::<u64>().ok())) {
            let top = k.ranks.entry(s).or_insert(0);
            *top = (*top).max(r);
        }
    }
    for (args, _) in cells_of(store, &subject_class(host), subject_hash.as_deref(), "from_label")? {
        if let Some(l) = args.get("label") {
            k.labels.insert(l.clone());
        }
    }
    for (args, _) in cells_of(store, &facet_class(host), facet_hash.as_deref(), "exists")? {
        if let (Some(f), Some(v)) = (args.get("facet"), args.get("value")) {
            k.facets.insert((f.clone(), v.clone()));
        }
    }
    Ok(k)
}

/// The cell lines of the catalogue as the store should hold them now (C3, C4).
fn cell_lines(host: &str, licence: &str, m: &Model, k: &Known) -> Vec<String> {
    let class = subject_class(host);
    let fclass = facet_class(host);
    let mut out = Vec::new();
    let keys = |s: &SubjectKey| {
        vec![
            ("domain", "string", json!(s.0)),
            ("thing", "string", json!(s.1)),
            ("measure", "string", json!(s.2)),
            ("region", "string", json!(s.3)),
        ]
    };
    // Every subject that exists now, or existed before: the live ones with their values, the others withdrawn.
    let all: BTreeSet<SubjectKey> = m.subjects.keys().chain(k.subjects.iter()).cloned().collect();
    for s in &all {
        let live = m.subjects.get(s);
        let count = live.map_or(0, |v| v.len());
        out.push(cell_line(
            &format!("{class}/exists"),
            keys(s),
            ("boolean", json!(live.is_some())),
            licence,
            "Whether a catalogue subject exists.",
        ));
        let label = live.map(|_| subject_label(s)).unwrap_or_default();
        out.push(cell_line(
            &format!("{class}/label"),
            keys(s),
            ("string", json!(label)),
            licence,
            "The label of a catalogue subject.",
        ));
        out.push(cell_line(
            &format!("{class}/sources"),
            keys(s),
            ("number", json!(count.to_string())),
            licence,
            "The number of sources of a catalogue subject.",
        ));
        let top = (count as u64).max(k.ranks.get(s).copied().unwrap_or(0));
        for rank in 1..=top {
            let value = live.and_then(|v| v.get(rank as usize - 1)).cloned().unwrap_or_default();
            let mut args = keys(s);
            args.push(("rank", "number", json!(rank.to_string())));
            out.push(cell_line(
                &format!("{class}/source"),
                args,
                ("string", json!(value)),
                licence,
                "The source of a rank of a catalogue subject.",
            ));
        }
    }
    // The lookup: each label of a live subject that names only it, and each label stored before, withdrawn when not so now.
    let mut by_label: BTreeMap<String, Vec<&SubjectKey>> = BTreeMap::new();
    for s in m.subjects.keys() {
        for l in lookup_labels(s) {
            by_label.entry(l).or_default().push(s);
        }
    }
    let labels: BTreeSet<String> = by_label.keys().cloned().chain(k.labels.iter().cloned()).collect();
    for l in labels {
        let value = match by_label.get(&l) {
            Some(v) if v.len() == 1 => subject_key_text(host, v[0]),
            _ => String::new(),
        };
        out.push(cell_line(
            &format!("{class}/from_label"),
            vec![("label", "string", json!(l))],
            ("string", json!(value)),
            licence,
            "The subject a label names.",
        ));
    }
    // The facet objects: each value of an entry, and each stored before.
    let all_facets: BTreeSet<(String, String)> = m.facets.keys().cloned().chain(k.facets.iter().cloned()).collect();
    for (facet, value) in all_facets {
        let subjects = m.facets.get(&(facet.clone(), value.clone()));
        let live = subjects.is_some();
        let fargs = || vec![("facet", "string", json!(facet)), ("value", "string", json!(value))];
        out.push(cell_line(
            &format!("{fclass}/exists"),
            fargs(),
            ("boolean", json!(live)),
            licence,
            "Whether a facet value exists.",
        ));
        out.push(cell_line(
            &format!("{fclass}/label"),
            fargs(),
            ("string", json!(facet_label(&facet, &value))),
            licence,
            "The label of a facet value.",
        ));
        let n = subjects.map_or(0, |s| s.len());
        out.push(cell_line(
            &format!("{fclass}/subjects"),
            fargs(),
            ("number", json!(n.to_string())),
            licence,
            "The number of subjects that have a facet value.",
        ));
    }
    out
}

/// Runs the catalogue job (C5): reads the store's faceted entries as of the clock, and writes the classes, the cells and the rule
/// as one publish under the mesh's key, with the job flag (C1). `licence` is the licence of the cells (`DATUMESH_LICENCE`).
///
/// A run over an unchanged catalogue writes nothing (`cells == 0`). Errors are publish's: a refusal names its line.
pub fn catalog_build_with<S: Store + ?Sized>(
    store: &S,
    key: &SigningKey,
    host: &str,
    licence: &str,
    opts: &PublishOptions,
) -> Result<CatalogBuilt, PublishError> {
    let now = opts.clock.now();
    let m = model(store, host, &now)?;
    let k = known(store, host, &now)?;
    let mut lines = vec![subject_class_line(host), facet_class_line(host), rule_line(host)];
    lines.extend(cell_lines(host, licence, &m, &k));
    lines.push(
        json!({"cell": {"fn": rule_fn(host), "args": {}}, "value": {"blob": RULE_FILE, "media_type": "text/plain"},
            "licence": licence, "summary": "The rule of the catalogue: how the sources of a subject are ranked."})
        .to_string(),
    );
    let mut blobs = BTreeMap::new();
    blobs.insert(RULE_FILE.to_string(), RULE.as_bytes().to_vec());
    let owned = vec![host.to_string()];
    let published = publish_job(store, key, host, &owned, lines, &blobs, opts)?;
    Ok(CatalogBuilt { cells: published.versions as u64, subjects: m.subjects.len() as u64 })
}
