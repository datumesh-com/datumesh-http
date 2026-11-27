//! The catalogue job (CONTRACT-classes-round C3 to C5): `datumesh-mesh catalog-build` writes the classes `catalog/subject` and
//! `catalog/facet` (their methods inline, K7), and the cells of their objects, as stored cells signed by the mesh's key with the
//! licence the job is given; and the rule text (`catalog/rule()`, the page `/catalog/rule`).
//!
//! The job reads the store's catalogue entries that have facets (as of its clock). A rerun over an unchanged catalogue writes
//! nothing: each cell line has no `tx`, so publish's skip rule keeps a cell whose latest version states the same thing. A subject
//! that disappears has every method cell withdrawn with the reason "the subject is no longer in the catalogue" (R1, R10), its
//! labels too, and so does a facet value that disappears; `source(rank)` past the count of sources is withdrawn ("rank past the
//! count of sources"). A withdrawal is a cell's null version, never `false` or `""`. A cell never stored and not wanted has no line,
//! so a label that names two subjects is absent, not withdrawn.
//!
//! At most 256 subjects come from one entry, and a subject keeps at most 20 sources (R11).
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
//! Within a level (R5): a source whose class is a subclass of another source's class (at any depth) comes before it; the rest
//! by the instant the name was first bound (a class's own class binding), then the name, ascending. The rank one source is the
//! first of that order, and `source(k)` is the k-th.
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

use datumesh_core::{
    fold_label, Arg, Cell, ClassDescriptor, FunctionDescriptor, Receipt, Scalar, SigningKey, Slice, Type,
};
use datumesh_store::{CatalogKind, Facets, SliceValue, Store};
use serde_json::{json, Value};

use crate::publish::{publish_job, PublishError, PublishOptions};

/// The rule of the catalogue (C5, R5, R7): the text of the page `/catalog/rule` and of the blob `catalog/rule()`.
pub(crate) const RULE: &str = "How the catalogue ranks the sources of a subject. \
A source is level one when it is published under the domain of the data: its publisher host is that domain or a sub-domain of it. A parent \
domain does not count, and no public suffix list is used. A source is level two when it is a relay of the data, a name under the relay path of the domain of the data and trusted \
as relayed, or a view built on such a relay: a function or a class whose inputs read a relay, directly or through other views. \
Every other source is level three. Within a level, a source whose class is a subclass of the class of another source, at any depth, \
ranks above that parent class. Then the source whose name was first bound comes first, and a tie on the time of the first binding \
is broken by the name, in ascending order. The rank one source of a subject is the first source in this order, and source of \
rank k is the k-th; a subject keeps at most twenty sources. A subject with no relay and no view has no source domain, so its sources \
are level one under their own hosts. The job that writes the catalogue applies this rule to the facets the publishers give. \
Addresses: a cell address and a class name carry no facet value and no rank; a page path may carry a facet value, as \
the domain pages of the catalogue do. Listed names are ranked by the same rule: a listed name is described here, not served here.";

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
pub(crate) fn lookup_labels(k: &SubjectKey) -> Vec<String> {
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

/// The most subjects one catalogue entry gives (R11): its facet product is cut to the first of this many, in the order of the
/// product (domain, thing, measure, region).
pub(crate) const SUBJECTS_PER_ENTRY: usize = 256;

/// The most sources a subject keeps (R11): `sources()` is at most this, and the ranks past it have no cell.
pub(crate) const SOURCES_PER_SUBJECT: usize = 20;

/// The subjects one page of the catalogue reads (CONTRACT-0.5.1 H2): a statement of the job or the view visits at most this
/// many subjects' cells of one method.
pub(crate) const PAGE: usize = 1000;

/// The default most subjects the catalogue lists (`DATUMESH_CATALOG_MAX`, CONTRACT-0.5.1 H2).
pub const DEFAULT_CATALOG_MAX: usize = 100_000;

/// The arguments of a subject class's keys, in the order of the class.
pub(crate) const SUBJECT_KEYS: [&str; 4] = ["domain", "thing", "measure", "region"];

/// The reason of a withdrawal of a subject or facet value that is gone (R10).
pub(crate) const GONE: &str = "the subject is no longer in the catalogue";
/// The reason of a withdrawal of a rank past the count of sources (R10).
pub(crate) const PAST: &str = "rank past the count of sources";
/// The reason of a withdrawal of a label that names no single subject any more (R10).
pub(crate) const LABEL_NO_SUBJECT: &str = "the label names no single subject";

/// An entry of the catalogue with a binding, as the rank sees it.
struct Entry {
    name: String,
    first: String,
    relay: bool,
    view: bool,
    reach: BTreeSet<String>,
    /// The class hash of a class entry.
    class: Option<String>,
    /// The hashes of the classes the entry's class extends, at any depth (R5).
    ancestors: BTreeSet<String>,
}

/// The live part of the catalogue: its subjects with their sources in rank order, and the facet objects with their subjects.
struct Model {
    subjects: BTreeMap<SubjectKey, Vec<String>>,
    facets: BTreeMap<(String, String), BTreeSet<SubjectKey>>,
}

/// The parent classes of a class, at any depth (R5): the hashes its `parent` chain names.
fn ancestors<S: Store + ?Sized>(store: &S, class_hash: &str) -> Result<BTreeSet<String>, PublishError> {
    let mut out = BTreeSet::new();
    let mut cur = class_hash.to_string();
    // a class chain is acyclic (a parent is stored before its child); the bound is a guard
    for _ in 0..64 {
        let Some(bytes) = store.class(&cur).map_err(err)? else { break };
        let cd = ClassDescriptor::from_json(&bytes).map_err(err)?;
        let Some(p) = cd.parent else { break };
        if !out.insert(p.clone()) {
            break;
        }
        cur = p;
    }
    Ok(out)
}

/// The order of a subject's entries (R5): `base` is sorted by level, first binding and name; then an entry whose class is a
/// subclass of another entry's class in the same level comes before it, the rest keeping the base order (a topological order
/// with the base order as the priority, so an unrelated entry keeps its place). Returns the entries' indices.
fn subclass_order(base: &[(u8, &str, &str, usize)], entries: &[Entry]) -> Vec<usize> {
    let n = base.len();
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut left = vec![0usize; n];
    for i in 0..n {
        let (li, _, _, ei) = base[i];
        let anc = &entries[ei].ancestors;
        if anc.is_empty() {
            continue;
        }
        for j in 0..n {
            let (lj, _, _, ej) = base[j];
            if i == j || lj != li {
                continue;
            }
            if entries[ej].class.as_ref().is_some_and(|c| anc.contains(c)) {
                succ[i].push(j);
                left[j] += 1;
            }
        }
    }
    let mut avail: BTreeSet<usize> = (0..n).filter(|&i| left[i] == 0).collect();
    let mut done = vec![false; n];
    let mut out = Vec::with_capacity(n);
    while let Some(i) = avail.pop_first() {
        done[i] = true;
        out.push(base[i].3);
        for &j in &succ[i] {
            left[j] -= 1;
            if left[j] == 0 && !done[j] {
                avail.insert(j);
            }
        }
    }
    // a class chain is acyclic, so every entry is placed; the guard keeps any left in base order
    out.extend((0..n).filter(|&i| !done[i]).map(|i| base[i].3));
    out
}

fn model<S: Store + ?Sized>(store: &S, host: &str, now: &str, cap: usize) -> Result<Model, PublishError> {
    let mut g = Graph { store, host, now, reads: BTreeMap::new(), reach: BTreeMap::new(), visiting: BTreeSet::new() };
    let listed = store.catalog_entries_with_facets().map_err(err)?;
    // The listed names (L4): a listed name joins the subjects of its own listing, and is bound here "or listed"; its first
    // time is the instant it was listed.
    let mut listings: BTreeMap<String, datumesh_store::Listing> = BTreeMap::new();
    for h in store.listed_hosts().map_err(err)? {
        for l in store.listings_of(&h.host).map_err(err)? {
            listings.insert(l.name.clone(), l);
        }
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut facets: BTreeMap<(String, String), BTreeSet<SubjectKey>> = BTreeMap::new();
    let mut members: BTreeMap<SubjectKey, Vec<usize>> = BTreeMap::new();
    for e in &listed {
        let listing = listings.get(&e.name);
        let keys: Vec<SubjectKey> = match listing {
            Some(l) => l
                .subjects
                .iter()
                .map(|s| (s.domain.clone(), s.thing.clone(), s.measure.clone(), s.region.clone()))
                .collect(),
            None => subject_keys(&e.facets),
        };
        let relay = relay_domain(host, &e.name).is_some();
        let reach = if e.first_bound.is_some() { g.reach(&e.name)? } else { BTreeSet::new() };
        let view = !relay && !reach.is_empty();
        let class = if matches!(e.kind, CatalogKind::Class) { e.hash.clone() } else { None };
        let ancestors = match &class {
            Some(h) => ancestors(store, h)?,
            None => BTreeSet::new(),
        };
        let idx = entries.len();
        entries.push(Entry {
            name: e.name.clone(),
            first: match listing {
                Some(l) => l.listed_at.clone(),
                None => e.first_bound.clone().unwrap_or_default(),
            },
            relay,
            view,
            reach,
            class,
            ancestors,
        });
        let bound = e.first_bound.is_some() || listing.is_some();
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
        let level = |i: usize| -> u8 {
            let e = &entries[i];
            if e.relay || e.view {
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
            }
        };
        let mut base: Vec<(u8, &str, &str, usize)> =
            idxs.iter().map(|&i| (level(i), entries[i].first.as_str(), entries[i].name.as_str(), i)).collect();
        base.sort();
        let order = subclass_order(&base, &entries);
        let names: Vec<String> = order.iter().take(SOURCES_PER_SUBJECT).map(|&i| entries[i].name.clone()).collect();
        subjects.insert(k.clone(), names);
    }
    // The catalogue lists at most `cap` subjects, the first in key order (H2); the rest are not written.
    if subjects.len() > cap {
        tracing::warn!(target: "datumesh_http", subjects = subjects.len(), cap,
            "catalogue job: the subjects are cut at DATUMESH_CATALOG_MAX");
        let kept = subjects.split_off(&subjects.keys().nth(cap).cloned().unwrap_or_default());
        drop(kept);
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

/// The subjects of an entry's facets: the cross product of its domain, thing, measure and region names (C3), the first
/// [`SUBJECTS_PER_ENTRY`] of it (R11).
fn subject_keys(facets: &Facets) -> Vec<SubjectKey> {
    let get = |k: &str| facets.get(k).cloned().unwrap_or_default();
    let (d, t, m, r) = (get("domain"), get("thing"), get("measure"), get("region"));
    let mut out = Vec::new();
    'all: for dd in &d {
        for tt in &t {
            for mm in &m {
                for rr in &r {
                    if out.len() == SUBJECTS_PER_ENTRY {
                        break 'all;
                    }
                    out.push((dd.clone(), tt.clone(), mm.clone(), rr.clone()));
                }
            }
        }
    }
    out
}

/// Whether stored receipt bytes are a withdrawal (a `withdrawn` member, R1).
fn is_withdrawal(bytes: &[u8]) -> bool {
    datumesh_core::json::parse(bytes).is_ok_and(|v| v.get("withdrawn").is_some())
}

pub(crate) fn subject_of_args(args: &BTreeMap<String, String>) -> Option<SubjectKey> {
    Some((
        args.get("domain")?.clone(),
        args.get("thing")?.clone(),
        args.get("measure")?.clone(),
        args.get("region")?.clone(),
    ))
}

/// The lexical arguments of a cell (its value arguments; an open one is left out).
pub(crate) fn lexical_args(cell: &Cell) -> BTreeMap<String, String> {
    cell.args
        .iter()
        .filter_map(|(k, a)| match a {
            Arg::Value(s) => Some((k.clone(), s.lexical().to_string())),
            Arg::Slice => None,
        })
        .collect()
}

/// A catalogue class as the store holds it now (CONTRACT-0.5.1 H2): its extent and its methods' cells are read in pages
/// by slice, and by key, never by a read of a whole method.
pub(crate) struct Class<'a, S: ?Sized> {
    store: &'a S,
    name: String,
    now: String,
    methods: BTreeMap<String, String>,
}

impl<'a, S: Store + ?Sized> Class<'a, S> {
    /// The class `name` as of `now`; `None` when it is not bound.
    pub(crate) fn open(store: &'a S, name: &str, now: &str) -> Result<Option<Self>, PublishError> {
        let Some(hash) = store.resolve_class(name, now).map_err(err)? else { return Ok(None) };
        let bytes = store.class(&hash).map_err(err)?.ok_or_else(|| err(format!("class {hash} is not stored")))?;
        let cd = ClassDescriptor::from_json(&bytes).map_err(err)?;
        Ok(Some(Class { store, name: name.to_string(), now: now.to_string(), methods: cd.methods }))
    }

    /// The next page of the members of method `method` with the arguments `open` open, after the sort key `after`, at most
    /// `limit` (the store's slice pages, S2). A method the class lacks has no members.
    pub(crate) fn page(
        &self,
        method: &str,
        open: &[&str],
        after: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<SliceValue>, PublishError> {
        let Some(fh) = self.methods.get(method) else { return Ok(Vec::new()) };
        let args: datumesh_core::Args = open.iter().map(|k| (k.to_string(), Arg::Slice)).collect();
        let cell = Cell { ns: self.name.clone(), fn_name: method.to_string(), fn_hash: fh.clone(), args };
        let slice = Slice::from_value(&cell.to_value()).map_err(err)?;
        self.store.slice_page(&slice, &self.now, after, limit).map_err(err)
    }

    /// The value of method `method` at each of `keys` (each a list of argument names and values), as the latest version
    /// holds it now: `None` for a cell with no value (withdrawn, or never stored). One statement a call, of at most
    /// `keys.len()` cells.
    pub(crate) fn values(
        &self,
        method: &str,
        keys: &[Vec<(String, Scalar)>],
    ) -> Result<Vec<Option<String>>, PublishError> {
        let Some(fh) = self.methods.get(method) else { return Ok(vec![None; keys.len()]) };
        let ids: Vec<String> = keys
            .iter()
            .map(|k| {
                let args: datumesh_core::Args = k.iter().map(|(n, s)| (n.clone(), Arg::Value(s.clone()))).collect();
                Cell { ns: self.name.clone(), fn_name: method.to_string(), fn_hash: fh.clone(), args }.id()
            })
            .collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let versions = self.store.latest_many(&refs).map_err(err)?;
        let live: Vec<&str> =
            versions.iter().flatten().filter(|v| !v.withdrawn).map(|v| v.receipt_hash.as_str()).collect();
        let mut bytes = self.store.receipts_many(&live).map_err(err)?.into_iter();
        let mut out = Vec::with_capacity(versions.len());
        for v in &versions {
            let value = match v {
                Some(v) if !v.withdrawn => match bytes.next().flatten() {
                    Some(b) if !is_withdrawal(&b) => {
                        let r = Receipt::from_json(&b).map_err(err)?;
                        Some(r.value().lexical().to_string())
                    }
                    _ => None,
                },
                _ => None,
            };
            out.push(value);
        }
        Ok(out)
    }
}

/// A lookup label as the key argument of `from_label`.
pub(crate) fn label_scalars(l: &str) -> Vec<(String, Scalar)> {
    vec![("label".to_string(), Scalar::from_lexical(Type::String, l).expect("a string is a scalar"))]
}

/// The key arguments of a subject, as scalars (for [`Class::values`]), and its rank when given.
pub(crate) fn subject_scalars(k: &SubjectKey, rank: Option<usize>) -> Vec<(String, Scalar)> {
    let mut out: Vec<(String, Scalar)> = [("domain", &k.0), ("thing", &k.1), ("measure", &k.2), ("region", &k.3)]
        .into_iter()
        .map(|(n, v)| (n.to_string(), Scalar::from_lexical(Type::String, v).expect("a string is a scalar")))
        .collect();
    if let Some(r) = rank {
        out.push(("rank".into(), Scalar::from_lexical(Type::Number, &r.to_string()).expect("a number is a scalar")));
    }
    out
}

/// A cell of the catalogue: its method's full name and its arguments' lexical values.
type CellKey = (String, BTreeMap<String, String>);

/// A cell the job wants with a value: its method, its arguments, its value and its summary.
struct Want {
    fn_name: String,
    args: BTreeMap<String, String>,
    typ: &'static str,
    value: String,
    summary: &'static str,
}

impl Want {
    fn key(&self) -> CellKey {
        (self.fn_name.clone(), self.args.clone())
    }

    fn line(&self, licence: &str) -> String {
        let args: Vec<(&str, &str, Value)> = self
            .args
            .iter()
            .map(|(k, v)| (k.as_str(), if k == "rank" { "number" } else { "string" }, json!(v)))
            .collect();
        let value = if self.typ == "boolean" { json!(self.value == "true") } else { json!(self.value) };
        cell_line(&self.fn_name, args, (self.typ, value), licence, self.summary)
    }
}

fn subject_args(s: &SubjectKey) -> BTreeMap<String, String> {
    [("domain", &s.0), ("thing", &s.1), ("measure", &s.2), ("region", &s.3)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

/// The line that withdraws a cell (R1): its address and the reason; no value.
fn withdrawal_line(fn_name: &str, args: &BTreeMap<String, String>, reason: &str, licence: &str) -> String {
    let a: serde_json::Map<String, Value> = args
        .iter()
        .map(|(k, v)| {
            let t = if k == "rank" { "number" } else { "string" };
            (k.clone(), json!({"type": t, "value": v}))
        })
        .collect();
    json!({"cell": {"fn": fn_name, "args": a}, "withdrawn": reason, "licence": licence,
        "summary": "A catalogue cell that no longer has a value."})
    .to_string()
}

/// The reason a valued cell the job no longer wants is withdrawn for (R10).
fn withdrawal_reason(class: &str, fclass: &str, key: &CellKey, prior: &str, m: &Model) -> &'static str {
    let (fn_name, args) = key;
    let live = |s: &SubjectKey| m.subjects.contains_key(s);
    if fn_name == &format!("{class}/from_label") {
        return match parse_subject_key_any(prior) {
            Some(s) if live(&s) => LABEL_NO_SUBJECT,
            _ => GONE,
        };
    }
    if fn_name == &format!("{class}/source") {
        let s = subject_of_args(args);
        return match s {
            Some(s) if live(&s) => PAST,
            _ => GONE,
        };
    }
    let _ = fclass;
    GONE
}

/// The subject a key encoding of any host's subject class names (a label's prior value).
fn parse_subject_key_any(text: &str) -> Option<SubjectKey> {
    let open = text.find('(')?;
    let host = text[..open].strip_suffix("/catalog/subject")?;
    parse_subject_key(host, text)
}

/// The cell lines of the catalogue as the store should hold them now (C3, C4, R10): every cell the job wants with its value, and
/// a withdrawal for each valued cell it no longer wants. A cell that is not stored and is not wanted has no line.
fn cell_lines<S: Store + ?Sized>(
    host: &str,
    licence: &str,
    m: &Model,
    store: &S,
    now: &str,
) -> Result<Vec<String>, PublishError> {
    let class = subject_class(host);
    let fclass = facet_class(host);
    let mut wants: Vec<Want> = Vec::new();
    for (s, sources) in &m.subjects {
        let count = sources.len();
        let key = |method: &str| format!("{class}/{method}");
        wants.push(Want {
            fn_name: key("exists"),
            args: subject_args(s),
            typ: "boolean",
            value: "true".into(),
            summary: "Whether a catalogue subject exists.",
        });
        wants.push(Want {
            fn_name: key("label"),
            args: subject_args(s),
            typ: "string",
            value: subject_label(s),
            summary: "The label of a catalogue subject.",
        });
        wants.push(Want {
            fn_name: key("sources"),
            args: subject_args(s),
            typ: "number",
            value: count.to_string(),
            summary: "The number of sources of a catalogue subject.",
        });
        for (i, src) in sources.iter().enumerate() {
            let mut args = subject_args(s);
            args.insert("rank".into(), (i + 1).to_string());
            wants.push(Want {
                fn_name: key("source"),
                args,
                typ: "string",
                value: src.clone(),
                summary: "The source of a rank of a catalogue subject.",
            });
        }
    }
    // The lookup: each label of a live subject that names only it. A label that names two is not stored (C3).
    let mut by_label: BTreeMap<String, Vec<&SubjectKey>> = BTreeMap::new();
    for s in m.subjects.keys() {
        for l in lookup_labels(s) {
            by_label.entry(l).or_default().push(s);
        }
    }
    for (l, subjects) in &by_label {
        if let [s] = subjects.as_slice() {
            wants.push(Want {
                fn_name: format!("{class}/from_label"),
                args: [("label".to_string(), l.clone())].into_iter().collect(),
                typ: "string",
                value: subject_key_text(host, s),
                summary: "The subject a label names.",
            });
        }
    }
    // The facet objects: each value of an entry.
    for (facet, value) in m.facets.keys() {
        let subjects = &m.facets[&(facet.clone(), value.clone())];
        let fargs: BTreeMap<String, String> =
            [("facet".to_string(), facet.clone()), ("value".to_string(), value.clone())].into_iter().collect();
        wants.push(Want {
            fn_name: format!("{fclass}/exists"),
            args: fargs.clone(),
            typ: "boolean",
            value: "true".into(),
            summary: "Whether a facet value exists.",
        });
        wants.push(Want {
            fn_name: format!("{fclass}/label"),
            args: fargs.clone(),
            typ: "string",
            value: facet_label(facet, value),
            summary: "The label of a facet value.",
        });
        wants.push(Want {
            fn_name: format!("{fclass}/subjects"),
            args: fargs,
            typ: "number",
            value: subjects.len().to_string(),
            summary: "The number of subjects that have a facet value.",
        });
    }
    let mut lines: Vec<String> = wants.iter().map(|w| w.line(licence)).collect();
    let wanted: BTreeSet<CellKey> = wants.iter().map(Want::key).collect();
    // A valued cell the job no longer wants is withdrawn (R10); the stored cells are read a page at a time (H2).
    let gone = |key: CellKey, prior: &str| -> Option<String> {
        if wanted.contains(&key) {
            return None;
        }
        let reason = withdrawal_reason(&class, &fclass, &key, prior, m);
        Some(withdrawal_line(&key.0, &key.1, reason, licence))
    };
    if let Some(sc) = Class::open(store, &class, now)? {
        let mut after: Option<Vec<u8>> = None;
        loop {
            let page = sc.page("exists", &SUBJECT_KEYS, after.as_deref(), PAGE)?;
            after = page.last().map(|v| v.sort_key.clone());
            let subjects: Vec<(SubjectKey, String)> = page
                .iter()
                .filter_map(|v| Some((subject_of_args(&lexical_args(&v.cell))?, v.value.lexical().to_string())))
                .collect();
            let keys: Vec<Vec<(String, Scalar)>> = subjects.iter().map(|(k, _)| subject_scalars(k, None)).collect();
            for (k, value) in &subjects {
                lines.extend(gone((format!("{class}/exists"), subject_args(k)), value));
            }
            // The lookup labels of these subjects, read by key (H2: never a slice of one argument).
            let labels_of: Vec<String> = subjects.iter().flat_map(|(k, _)| lookup_labels(k)).collect();
            for chunk in labels_of.chunks(PAGE) {
                let lk: Vec<Vec<(String, Scalar)>> = chunk.iter().map(|l| label_scalars(l)).collect();
                let values = sc.values("from_label", &lk)?;
                for (l, v) in chunk.iter().zip(&values) {
                    if let Some(v) = v {
                        let args: BTreeMap<String, String> = [("label".to_string(), l.clone())].into_iter().collect();
                        lines.extend(gone((format!("{class}/from_label"), args), v));
                    }
                }
            }
            let labels = sc.values("label", &keys)?;
            let counts = sc.values("sources", &keys)?;
            for (((k, _), label), count) in subjects.iter().zip(&labels).zip(&counts) {
                if let Some(v) = label {
                    lines.extend(gone((format!("{class}/label"), subject_args(k)), v));
                }
                if let Some(v) = count {
                    lines.extend(gone((format!("{class}/sources"), subject_args(k)), v));
                }
            }
            // The sources: each rank up to the count the store holds now (before this run's writes), rank by rank.
            let stored_count = |i: usize| counts[i].as_deref().and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);
            let most = (0..subjects.len()).map(stored_count).max().unwrap_or(0).min(SOURCES_PER_SUBJECT);
            for r in 1..=most {
                let idx: Vec<usize> = (0..subjects.len()).filter(|&i| stored_count(i) >= r).collect();
                let rank_keys: Vec<Vec<(String, Scalar)>> =
                    idx.iter().map(|&i| subject_scalars(&subjects[i].0, Some(r))).collect();
                let values = sc.values("source", &rank_keys)?;
                for (&i, v) in idx.iter().zip(&values) {
                    if let Some(v) = v {
                        let mut args = subject_args(&subjects[i].0);
                        args.insert("rank".into(), r.to_string());
                        lines.extend(gone((format!("{class}/source"), args), v));
                    }
                }
            }
            if page.len() < PAGE {
                break;
            }
        }
    }
    if let Some(fc) = Class::open(store, &fclass, now)? {
        let mut after: Option<Vec<u8>> = None;
        loop {
            let page = fc.page("exists", &["facet", "value"], after.as_deref(), PAGE)?;
            after = page.last().map(|v| v.sort_key.clone());
            let facets: Vec<(BTreeMap<String, String>, String)> =
                page.iter().map(|v| (lexical_args(&v.cell), v.value.lexical().to_string())).collect();
            let keys: Vec<Vec<(String, Scalar)>> = facets
                .iter()
                .map(|(a, _)| {
                    ["facet", "value"]
                        .into_iter()
                        .map(|n| {
                            (n.to_string(), Scalar::from_lexical(Type::String, &a[n]).expect("a string is a scalar"))
                        })
                        .collect()
                })
                .collect();
            for (a, value) in &facets {
                lines.extend(gone((format!("{fclass}/exists"), a.clone()), value));
            }
            let labels = fc.values("label", &keys)?;
            let subjects = fc.values("subjects", &keys)?;
            for ((a, _), (label, n)) in facets.iter().zip(labels.iter().zip(&subjects)) {
                if let Some(v) = label {
                    lines.extend(gone((format!("{fclass}/label"), a.clone()), v));
                }
                if let Some(v) = n {
                    lines.extend(gone((format!("{fclass}/subjects"), a.clone()), v));
                }
            }
            if page.len() < PAGE {
                break;
            }
        }
    }
    Ok(lines)
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
    let cap =
        crate::config::catalog_max_of(std::env::var("DATUMESH_CATALOG_MAX").ok()).map_err(|e| err(format!("{e}")))?;
    let m = model(store, host, &now, cap)?;
    let mut lines = vec![subject_class_line(host), facet_class_line(host), rule_line(host)];
    lines.extend(cell_lines(host, licence, &m, store, &now)?);
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
