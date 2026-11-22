//! `GET /find` (CONTRACT-build2-part2-read R3): the hybrid search of the catalogue.
//!
//! The answer ranks, in this order, an exact match (a name, label, alias or value equal to the query
//! or one of its tokens), a text match (every query word in a name, label, description or argument
//! line: the store's `text_find`), and a vector match (the nearest descriptions and dictionary values
//! by the embedding service's vectors, when the service answers and the store has vectors).
//!
//! The query is also read as a request, `resolution`: a function, plus values for its keys. A string
//! key is filled only by an exact or labelled dictionary match (value, label or alias, after NFC and
//! case folding); a number key takes the numbers of the query in signature order; a range is refused.
//! Words of the catalogue (names, labels, descriptions) and the fixed list in `find_stopwords.txt`
//! never take part in matching a value. An embedding match alone gives up to three labelled
//! candidates for a key, and fills nothing.
//!
//! A query with no results and no resolution is written to the store's miss log (its text only).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use datumesh_core::json::{self, Value};
use datumesh_core::{ClassDescriptor, FunctionDescriptor};
use datumesh_store::{CatalogEntry, CatalogKind, Hit, Store};
use unicode_normalization::UnicodeNormalization;

use crate::embed::{EmbedClient, MODEL};
use crate::mesh::Inner;

/// The default `limit` of `/find` and the most it takes.
pub(crate) const DEFAULT_LIMIT: usize = 10;
pub(crate) const MAX_LIMIT: usize = 50;

/// The fixed list of generic words (`find_stopwords.txt`).
const STOPWORDS: &str = include_str!("find_stopwords.txt");

/// The most text hits asked of the store per query.
const TEXT_HITS: usize = 50;

/// The most candidates, and labelled values per key, a resolution gives.
const CANDIDATES: usize = 3;

/// Dash-like characters that make a range (`3:16-18`, `16–18`).
fn is_dash(c: char) -> bool {
    matches!(c, '-' | '\u{2010}'..='\u{2015}' | '\u{2212}')
}

/// NFC of `s`.
pub(crate) fn nfc(s: &str) -> String {
    s.nfc().collect()
}

/// NFC and lower case: the form values are compared in.
fn lower(s: &str) -> String {
    nfc(s).to_lowercase()
}

pub(crate) fn stopwords() -> HashSet<String> {
    STOPWORDS.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(lower).collect()
}

fn add_words(g: &mut HashSet<String>, text: &str) {
    for w in nfc(text).split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        g.insert(w.to_lowercase());
    }
}

/// The most catalogue entries the generic words are read from (CONTRACT-round-0.7-P HP6: one bounded read, on the
/// find thread's cycle).
const GENERIC_ENTRIES: usize = 100_000;

/// The generic words: the fixed list, and every word of every catalogue name, label and description. One read of
/// the entries (`catalog_entries_all`), not one lookup per entry.
pub(crate) fn generic_words(store: &dyn Store) -> HashSet<String> {
    let mut g = stopwords();
    if let Ok(entries) = store.catalog_entries_all(GENERIC_ENTRIES) {
        for (_, name, label, description) in &entries {
            add_words(&mut g, name);
            add_words(&mut g, label);
            add_words(&mut g, description);
        }
    }
    g
}

/// Words of a query that are read: the first of them (CONTRACT-round-0.7-P HP6).
const QUERY_WORDS: usize = 8;

/// Most words a span of the query has (CONTRACT-round-0.7-P HP6).
const SPAN_WORDS: usize = 4;

/// The query cut to its first [`QUERY_WORDS`] words.
fn first_words(qn: &str) -> String {
    qn.split_whitespace().take(QUERY_WORDS).collect::<Vec<_>>().join(" ")
}

/// Whether the store has the dictionary value `value` of `key` of `owner` (exact, CONTRACT-0.4.9 H7).
pub(crate) fn has_value<S: Store + ?Sized>(store: &S, owner: &str, key: &str, value: &str) -> bool {
    store.has_key_value(owner, key, value).unwrap_or(false)
}

/// The label and aliases the store has for the dictionary value `value` of `key` of `owner`, if any.
pub(crate) fn value_row<S: Store + ?Sized>(
    store: &S,
    owner: &str,
    key: &str,
    value: &str,
) -> Option<(String, Vec<String>)> {
    let hits = store.text_find(&nfc(value), 1000).ok()?;
    hits.into_iter().find_map(|h| match h {
        Hit::Value { owner: o, key: k, value: v, label, aliases, .. } if o == owner && k == key && v == value => {
            Some((label, aliases))
        }
        _ => None,
    })
}

/// The catalogue entry of `name`, found by its name (an exact lookup, CONTRACT-0.4.9 H4).
pub(crate) fn entry_by_name<S: Store + ?Sized>(store: &S, name: &str) -> Option<CatalogEntry> {
    store.catalog_entry(name).ok().flatten()
}

/// What a query is read against: the store, the mesh's now and the generic words.
pub(crate) struct Ctx<'a> {
    store: &'a dyn Store,
    now: String,
    generic: Arc<HashSet<String>>,
}

pub(crate) fn ctx_of(inner: &Inner) -> Ctx<'_> {
    let generic = inner.generic.read().map(|g| g.clone()).unwrap_or_default();
    Ctx { store: &*inner.store, now: inner.now(), generic }
}

/// The arguments of a function as `(name, type)`, in signature order.
type Args = Vec<(String, String)>;

fn arg_list(sig: &Value) -> Args {
    let Some(Value::Object(args)) = sig.get("args") else { return Vec::new() };
    let order: Vec<String> = match sig.get("order") {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        _ => args.iter().map(|(k, _)| k.clone()).collect(),
    };
    order
        .iter()
        .filter_map(|k| {
            let (_, v) = args.iter().find(|(n, _)| n == k)?;
            let ty = v.get("type").and_then(Value::as_str).unwrap_or("value");
            Some((k.clone(), ty.to_string()))
        })
        .collect()
}

/// The bound function of `name`: its hash and arguments.
fn fn_sig(ctx: &Ctx, name: &str) -> Option<(String, Args)> {
    let b = ctx.store.resolve_binding(name, &ctx.now).ok()??;
    let bytes = ctx.store.descriptor(&b.fn_hash)?;
    let d = FunctionDescriptor::from_json(&bytes[..]).ok()?;
    Some((b.fn_hash, arg_list(&d.signature.to_value())))
}

fn descriptor_json(store: &dyn Store, hash: &str) -> Value {
    store.descriptor(hash).and_then(|d| json::parse(&d[..]).ok()).unwrap_or(Value::Null)
}

fn template(name: &str, args: &Args) -> String {
    let parts: Vec<String> = args.iter().map(|(k, t)| format!("{k}: <{t}>")).collect();
    format!("{name}({})", parts.join(", "))
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A value of a key, filled from the query.
#[derive(Clone, Debug)]
pub(crate) struct Fill {
    value: String,
    label: String,
    numeric: bool,
}

fn filled_call(name: &str, args: &Args, fills: &BTreeMap<String, Fill>) -> String {
    let parts: Vec<String> = args
        .iter()
        .map(|(k, t)| match fills.get(k) {
            Some(f) if f.numeric => format!("{k}: {}", f.value),
            Some(f) => format!("{k}: \"{}\"", escape(&f.value)),
            None => format!("{k}: <{t}>"),
        })
        .collect();
    format!("{name}({})", parts.join(", "))
}

/// The human label of a filled call: the string labels in order, then the numbers as `c:v`.
fn fill_label(args: &Args, fills: &BTreeMap<String, Fill>) -> String {
    let mut words = Vec::new();
    let mut nums = Vec::new();
    for (k, _) in args {
        if let Some(f) = fills.get(k) {
            if f.numeric {
                nums.push(f.value.clone());
            } else {
                words.push(f.label.clone());
            }
        }
    }
    let mut s = words.join(" ");
    if !nums.is_empty() {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&nums.join(":"));
    }
    s
}

fn is_numeric(t: &str) -> bool {
    let parts: Vec<&str> = t.split(':').collect();
    (1..=2).contains(&parts.len()) && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn number_tokens(t: &str) -> Vec<String> {
    t.split(':').map(|p| p.parse::<u64>().map(|n| n.to_string()).unwrap_or_else(|_| p.to_string())).collect()
}

fn has_digit(t: &str) -> bool {
    t.chars().any(|c| c.is_ascii_digit())
}

/// Whether the query has a dash between two numbers (`3:16-18`, `16–18`, `1-5`).
fn is_range(q: &str) -> bool {
    let c: Vec<char> = q.chars().collect();
    (0..c.len()).any(|i| {
        is_dash(c[i])
            && c[..i].iter().rev().find(|x| !x.is_whitespace()).is_some_and(|p| p.is_ascii_digit())
            && c[i + 1..].iter().find(|x| !x.is_whitespace()).is_some_and(|n| n.is_ascii_digit())
    })
}

/// The spans `(start, end)` of `n` tokens, longest first, then by start.
fn spans(n: usize) -> Vec<(usize, usize)> {
    let mut v: Vec<(usize, usize)> =
        (0..n).flat_map(|i| (i + 1..=n.min(i + SPAN_WORDS)).map(move |j| (i, j))).collect();
    v.sort_by(|a, b| (b.1 - b.0).cmp(&(a.1 - a.0)).then(a.0.cmp(&b.0)));
    v
}

/// A dictionary value a span matches exactly: its owner (a bound function), the key and the value.
struct VMatch {
    owner: String,
    key: String,
    value: String,
    label: String,
}

/// The dictionary values of string keys that `span` equals (value, label or alias, after NFC and
/// case folding). The store's text search finds the candidates; this keeps the exact ones.
fn value_matches(ctx: &Ctx, span: &str) -> Vec<VMatch> {
    let want = lower(span);
    let Ok(hits) = ctx.store.text_find(&nfc(span), 100) else { return Vec::new() };
    let mut out = Vec::new();
    for h in hits {
        let Hit::Value { owner, key, value, label, aliases, .. } = h else { continue };
        let exact = lower(&value) == want || lower(&label) == want || aliases.iter().any(|a| lower(a) == want);
        if !exact {
            continue;
        }
        let string_key = fn_sig(ctx, &owner).is_some_and(|(_, a)| a.iter().any(|(k, t)| *k == key && t == "string"));
        if !string_key {
            continue;
        }
        let label = if label.is_empty() { value.clone() } else { label };
        out.push(VMatch { owner, key, value, label });
    }
    out
}

/// The exact value matches of the spans of `toks` that are not made of generic words alone, per
/// owner and key; the longest span of a key wins.
fn span_fills(ctx: &Ctx, toks: &[String]) -> BTreeMap<String, BTreeMap<String, Fill>> {
    let mut out: BTreeMap<String, BTreeMap<String, Fill>> = BTreeMap::new();
    for (i, j) in spans(toks.len()) {
        let part = &toks[i..j];
        if part.iter().all(|t| ctx.generic.contains(&lower(t))) {
            continue;
        }
        for m in value_matches(ctx, &part.join(" ")) {
            out.entry(m.owner).or_default().entry(m.key).or_insert(Fill {
                value: m.value,
                label: m.label,
                numeric: false,
            });
        }
    }
    out
}

/// A labelled value a key may take, from the embedding service.
#[derive(Clone, Debug)]
struct Labelled {
    value: String,
    label: String,
    score: f64,
}

/// A function the query may resolve to, with what it fills and what is missing.
#[derive(Clone, Debug)]
struct Cand {
    name: String,
    args: Args,
    fills: BTreeMap<String, Fill>,
    missing: Vec<String>,
    keys: Vec<(String, Vec<Labelled>)>,
}

impl Cand {
    fn full(&self) -> bool {
        self.missing.is_empty()
    }
}

fn make_cand(
    name: &str,
    args: Args,
    mut fills: BTreeMap<String, Fill>,
    tail: &[String],
    string_fill: bool,
) -> Option<Cand> {
    if string_fill {
        let nums: Vec<String> = args.iter().filter(|(_, t)| t == "number").map(|(k, _)| k.clone()).collect();
        if tail.len() > nums.len() {
            return None;
        }
        for (k, n) in nums.iter().zip(tail) {
            fills.insert(k.clone(), Fill { value: n.clone(), label: n.clone(), numeric: true });
        }
    }
    let missing: Vec<String> = args.iter().filter(|(k, _)| !fills.contains_key(k)).map(|(k, _)| k.clone()).collect();
    Some(Cand { name: name.to_string(), args, fills, missing, keys: Vec::new() })
}

/// What the query reads as before any embedding.
enum Read {
    /// A range, refused (`E_RANGE_REFUSED`).
    Range,
    /// The candidates of the exact dictionary matches, the numbers at the end of the query, and the
    /// residual words (the non-generic words before the numbers, for the embedding).
    Request { cands: Vec<Cand>, tail: Vec<String>, residual: String },
}

fn read_query(ctx: &Ctx, qn: &str) -> Read {
    let toks: Vec<String> = qn.split_whitespace().map(str::to_string).collect();
    if is_range(qn) {
        if let Some(idx) = toks.iter().position(|t| has_digit(t)) {
            if !span_fills(ctx, &toks[..idx]).is_empty() {
                return Read::Range;
            }
        }
    }
    let mut k = toks.len();
    while k > 0 && is_numeric(&toks[k - 1]) {
        k -= 1;
    }
    let tail: Vec<String> = toks[k..].iter().flat_map(|t| number_tokens(t)).collect();
    let prefix = &toks[..k];
    // A: the words before the numbers; B: the whole query, when numbers follow (`Code 07`).
    let a = span_fills(ctx, prefix);
    let b = if tail.is_empty() { BTreeMap::new() } else { span_fills(ctx, &toks) };
    let owners: BTreeSet<String> = a.keys().chain(b.keys()).cloned().collect();
    let mut cands = Vec::new();
    for owner in owners {
        let Some((_, args)) = fn_sig(ctx, &owner) else { continue };
        let mut fills: BTreeMap<String, Fill> = a.get(&owner).cloned().unwrap_or_default();
        for (key, f) in b.get(&owner).cloned().unwrap_or_default() {
            fills.entry(key).or_insert(f);
        }
        if fills.is_empty() {
            continue;
        }
        let from_prefix = a.get(&owner).is_some_and(|m| !m.is_empty());
        if let Some(c) = make_cand(&owner, args, fills, &tail, from_prefix) {
            cands.push(c);
        }
    }
    let residual: Vec<String> = prefix.iter().filter(|t| !ctx.generic.contains(&lower(t))).cloned().collect();
    Read::Request { cands, tail, residual: residual.join(" ") }
}

/// The result of one function: its value under the catalogue's label and description.
fn function_value(
    ctx: &Ctx,
    lk: &mut Lookup,
    name: &str,
    filled: Option<(String, String)>,
    mt: &str,
    score: f64,
) -> Option<Value> {
    let (hash, args) = fn_sig(ctx, name)?;
    let since = ctx.store.resolve_binding(name, &ctx.now).ok()??.tx;
    let entry = lk.entry(ctx.store, name);
    let (call, label) = filled.unwrap_or_else(|| (template(name, &args), default_label(name, entry.as_ref())));
    let description = entry.as_ref().map(|e| e.description.clone()).unwrap_or_default();
    Some(Value::Object(vec![
        ("kind".into(), Value::String("function".into())),
        ("name".into(), Value::String(name.into())),
        ("label".into(), Value::String(label)),
        ("description".into(), Value::String(description)),
        ("call".into(), Value::String(call)),
        ("match".into(), Value::String(mt.into())),
        ("score".into(), json_number(score)),
        ("descriptor".into(), descriptor_json(ctx.store, &hash)),
        ("fn_hash".into(), Value::String(hash)),
        ("since".into(), Value::String(since)),
    ]))
}

fn method_value(ctx: &Ctx, lk: &mut Lookup, name: &str, mt: &str, score: f64) -> Option<Value> {
    let (class, _) = name.rsplit_once('/')?;
    ctx.store.resolve_class_binding(class, &ctx.now).ok()??;
    let (hash, args) = fn_sig(ctx, name)?;
    let since = ctx.store.resolve_binding(name, &ctx.now).ok()??.tx;
    let entry = lk.entry(ctx.store, name);
    let description = entry.as_ref().map(|e| e.description.clone()).unwrap_or_default();
    Some(Value::Object(vec![
        ("kind".into(), Value::String("method".into())),
        ("name".into(), Value::String(name.into())),
        ("class".into(), Value::String(class.into())),
        ("label".into(), Value::String(default_label(name, entry.as_ref()))),
        ("description".into(), Value::String(description)),
        ("call".into(), Value::String(template(name, &args))),
        ("match".into(), Value::String(mt.into())),
        ("score".into(), json_number(score)),
        ("descriptor".into(), descriptor_json(ctx.store, &hash)),
        ("fn_hash".into(), Value::String(hash)),
        ("since".into(), Value::String(since)),
    ]))
}

/// The arguments after the class's keys, and what the method returns (CONTRACT-1.7a H3).
fn method_call_shape(store: &dyn Store, fn_hash: &str, keys: &[String]) -> (Value, Value) {
    let Some(bytes) = store.descriptor(fn_hash) else { return (Value::Null, Value::Null) };
    let Ok(d) = FunctionDescriptor::from_json(&bytes[..]) else { return (Value::Null, Value::Null) };
    let sig = d.signature.to_value();
    let args = match sig.get("args") {
        Some(Value::Object(m)) => Value::Object(
            m.iter()
                .filter(|(k, _)| !keys.contains(k))
                .map(|(k, v)| {
                    let v = match v {
                        Value::Object(f) => Value::Object(
                            f.iter()
                                .filter(|(fk, fv)| fk != "unit" || matches!(fv, Value::String(_)))
                                .cloned()
                                .collect(),
                        ),
                        other => other.clone(),
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        _ => Value::Object(vec![]),
    };
    (args, sig.get("returns").cloned().unwrap_or(Value::Null))
}

/// A class as a result: its descriptor, its methods and their shapes (CONTRACT-1.7a H3).
fn class_value(ctx: &Ctx, lk: &mut Lookup, name: &str, mt: &str, score: f64) -> Option<Value> {
    let cb = ctx.store.resolve_class_binding(name, &ctx.now).ok()??;
    let bytes = ctx.store.class(&cb.class_hash).ok().flatten()?;
    let cd = ClassDescriptor::from_json(&bytes[..]).ok()?;
    let keys: Vec<String> = cd.keys.keys().cloned().collect();
    let mut methods = Vec::new();
    for (m, fh) in &cd.methods {
        let full = format!("{name}/{m}");
        let (args, returns) = method_call_shape(ctx.store, fh, &keys);
        let mut obj = vec![
            ("fn".into(), Value::String(full.clone())),
            ("fn_hash".into(), Value::String(fh.clone())),
            ("args".into(), args),
            ("returns".into(), returns),
        ];
        if let Some(d) = lk.entry(ctx.store, &full).map(|e| e.description) {
            obj.push(("description".into(), Value::String(d)));
        }
        methods.push((m.clone(), Value::Object(obj)));
    }
    let entry = lk.entry(ctx.store, name);
    let description = entry.as_ref().map(|e| e.description.clone()).unwrap_or_default();
    let call = format!("{name}({})", class_key_list(&bytes));
    Some(Value::Object(vec![
        ("kind".into(), Value::String("class".into())),
        ("name".into(), Value::String(name.into())),
        ("label".into(), Value::String(default_label(name, entry.as_ref()))),
        ("description".into(), Value::String(description)),
        ("call".into(), Value::String(call)),
        ("match".into(), Value::String(mt.into())),
        ("score".into(), json_number(score)),
        ("class_hash".into(), Value::String(cb.class_hash.clone())),
        ("descriptor".into(), json::parse(&bytes[..]).unwrap_or(Value::Null)),
        ("methods".into(), Value::Object(methods)),
        ("since".into(), Value::String(cb.tx)),
    ]))
}

/// The keys of a class in order, as call placeholders.
fn class_key_list(bytes: &[u8]) -> String {
    let Ok(v) = json::parse(bytes) else { return String::new() };
    let Some(Value::Object(keys)) = v.get("keys") else { return String::new() };
    let parts: Vec<String> = keys
        .iter()
        .map(|(k, x)| {
            let ty = x.get("type").and_then(Value::as_str).unwrap_or("value");
            format!("{k}: <{ty}>")
        })
        .collect();
    parts.join(", ")
}

fn default_label(name: &str, entry: Option<&CatalogEntry>) -> String {
    match entry {
        Some(e) if !e.label.is_empty() => e.label.clone(),
        _ => name.rsplit('/').next().unwrap_or(name).to_string(),
    }
}

fn json_number(x: f64) -> Value {
    json::parse(format!("{x:.6}").as_bytes()).unwrap_or(Value::Null)
}

/// Catalogue entries by name, looked up once per answer.
#[derive(Default)]
struct Lookup {
    entries: HashMap<String, Option<CatalogEntry>>,
}

impl Lookup {
    fn entry(&mut self, store: &dyn Store, name: &str) -> Option<CatalogEntry> {
        if let Some(e) = self.entries.get(name) {
            return e.clone();
        }
        let e = entry_by_name(store, name);
        self.entries.insert(name.to_string(), e.clone());
        e
    }
}

/// A result in the answer, with its rank (0 exact, 1 text, 2 vector) and kind order.
struct Scored {
    rank: u8,
    score: f64,
    kind: u8,
    name: String,
    value: Value,
}

#[derive(Default)]
struct Board {
    best: BTreeMap<String, Scored>,
}

impl Board {
    fn offer(&mut self, s: Option<Scored>) {
        let Some(s) = s else { return };
        let keep = match self.best.get(&s.name) {
            Some(old) => old.rank < s.rank || (old.rank == s.rank && old.score >= s.score),
            None => false,
        };
        if !keep {
            self.best.insert(s.name.clone(), s);
        }
    }
}

fn scored(rank: u8, score: f64, kind: u8, name: &str, value: Option<Value>) -> Option<Scored> {
    value.map(|value| Scored { rank, score, kind, name: name.to_string(), value })
}

/// Exact: the name or label, or a token of the query, equals `want` (both NFC, case-folded).
fn exact_for(qn: &str, toks: &[String], candidates: &[&str]) -> bool {
    let whole = lower(qn);
    candidates.iter().map(|c| lower(c)).any(|c| c == whole || toks.contains(&c))
}

fn kind_order(k: CatalogKind) -> u8 {
    match k {
        CatalogKind::Class => 0,
        CatalogKind::Method => 1,
        CatalogKind::Function => 2,
    }
}

/// The cheap, text-only answer for an empty query: the bound entries by name.
fn listing(ctx: &Ctx, lk: &mut Lookup, limit: usize) -> Vec<Value> {
    let Ok(s) = ctx.store.catalog_summary() else { return Vec::new() };
    let mut entries = s.entries;
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = Vec::new();
    for e in entries {
        let v = match e.kind {
            CatalogKind::Function => function_value(ctx, lk, &e.name, None, "text", 1.0),
            CatalogKind::Method => method_value(ctx, lk, &e.name, "text", 1.0),
            CatalogKind::Class => class_value(ctx, lk, &e.name, "text", 1.0),
        };
        if let Some(v) = v {
            out.push(v);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

/// The vector candidates of the keys: the nearest dictionary values of string keys for the query's
/// residual words, up to three per key, attached to the candidates of their functions (a function
/// that is not yet a candidate becomes one, with the query's numbers in its number keys).
fn vector_keys(ctx: &Ctx, ec: &EmbedClient, residual: &str, tail: &[String], cands: &mut Vec<Cand>) {
    let Ok(vs) = ec.embed("query: ", &[residual.to_string()]) else { return };
    let Some(v) = vs.first() else { return };
    let Ok(hits) = ctx.store.vector_find(MODEL, v, 30) else { return };
    let mut groups: BTreeMap<(String, String), Vec<Labelled>> = BTreeMap::new();
    for h in hits {
        let Hit::Value { owner, key, value, label, score, .. } = h else { continue };
        let string_key = fn_sig(ctx, &owner).is_some_and(|(_, a)| a.iter().any(|(k, t)| *k == key && t == "string"));
        if !string_key {
            continue;
        }
        let label = if label.is_empty() { value.clone() } else { label };
        let g = groups.entry((owner, key)).or_default();
        if g.len() < CANDIDATES {
            g.push(Labelled { value, label, score });
        }
    }
    for ((owner, key), labelled) in groups {
        let idx = match cands.iter().position(|c| c.name == owner) {
            Some(i) => i,
            None => {
                let Some((_, args)) = fn_sig(ctx, &owner) else { continue };
                let Some(c) = make_cand(&owner, args, BTreeMap::new(), tail, false) else { continue };
                cands.push(c);
                cands.len() - 1
            }
        };
        cands[idx].keys.push((key, labelled));
    }
}

/// The resolution member of the answer, and the exact call when there is one.
struct Resolved {
    member: Option<Value>,
    exact: Option<Cand>,
}

fn resolution_of(ctx: &Ctx, lk: &mut Lookup, cands: &[Cand]) -> Resolved {
    if cands.len() == 1 && cands[0].full() {
        let c = &cands[0];
        let call = filled_call(&c.name, &c.args, &c.fills);
        let label = fill_label(&c.args, &c.fills);
        let member = Value::Object(vec![
            ("status".into(), Value::String("exact".into())),
            ("call".into(), Value::String(call)),
            ("label".into(), Value::String(label)),
        ]);
        return Resolved { member: Some(member), exact: Some(c.clone()) };
    }
    if cands.is_empty() {
        return Resolved { member: None, exact: None };
    }
    let mut ordered: Vec<&Cand> = cands.iter().filter(|c| c.full()).collect();
    ordered.extend(cands.iter().filter(|c| !c.full()));
    let mut list = Vec::new();
    for c in ordered.iter().take(CANDIDATES) {
        let mut obj = vec![("call".into(), Value::String(filled_call(&c.name, &c.args, &c.fills)))];
        let mut label = fill_label(&c.args, &c.fills);
        if label.is_empty() {
            label = default_label(&c.name, lk.entry(ctx.store, &c.name).as_ref());
        }
        obj.push(("label".into(), Value::String(label)));
        if !c.missing.is_empty() {
            obj.push((
                "missing_keys".into(),
                Value::Array(c.missing.iter().map(|k| Value::String(k.clone())).collect()),
            ));
        }
        list.push(Value::Object(obj));
    }
    let mut keys = Vec::new();
    for c in cands {
        for (key, labelled) in &c.keys {
            let cs: Vec<Value> = labelled
                .iter()
                .take(CANDIDATES)
                .map(|l| {
                    Value::Object(vec![
                        ("value".into(), Value::String(l.value.clone())),
                        ("label".into(), Value::String(l.label.clone())),
                        ("score".into(), json_number(l.score)),
                    ])
                })
                .collect();
            keys.push(Value::Object(vec![
                ("key".into(), Value::String(key.clone())),
                ("function".into(), Value::String(c.name.clone())),
                ("candidates".into(), Value::Array(cs)),
            ]));
        }
    }
    let mut member =
        vec![("status".into(), Value::String("ambiguous".into())), ("candidates".into(), Value::Array(list))];
    if !keys.is_empty() {
        member.push(("keys".into(), Value::Array(keys)));
    }
    Resolved { member: Some(Value::Object(member)), exact: None }
}

/// The class candidates of a query: every class whose own entry or one of whose methods matches each
/// word (the words of a class are its name, label and description, and its methods' names and
/// descriptions, taken together).
fn class_coverage(ctx: &Ctx, lk: &mut Lookup, qn: &str, toks: &[String], limit: usize, board: &mut Board) {
    let mut per_word: Vec<HashMap<String, (CatalogKind, f64)>> = Vec::new();
    for w in toks {
        let mut m = HashMap::new();
        if let Ok(hits) = ctx.store.text_find(w, 200) {
            for h in hits {
                if let Hit::Entry { entry, score, .. } = h {
                    m.insert(entry.name.clone(), (entry.kind, score));
                }
            }
        }
        per_word.push(m);
    }
    let mut classes: BTreeSet<String> = BTreeSet::new();
    for m in &per_word {
        for (name, (kind, _)) in m {
            match kind {
                CatalogKind::Class => {
                    classes.insert(name.clone());
                }
                CatalogKind::Method => {
                    if let Some((c, _)) = name.rsplit_once('/') {
                        classes.insert(c.to_string());
                    }
                }
                CatalogKind::Function => {}
            }
        }
    }
    for c in classes {
        let Ok(Some(cb)) = ctx.store.resolve_class_binding(&c, &ctx.now) else { continue };
        let Some(bytes) = ctx.store.class(&cb.class_hash).ok().flatten() else { continue };
        let Ok(cd) = ClassDescriptor::from_json(&bytes[..]) else { continue };
        let mut names = vec![c.clone()];
        names.extend(cd.methods.keys().map(|m| format!("{c}/{m}")));
        let mut total = 0.0;
        let mut covered = true;
        for m in &per_word {
            let best = names
                .iter()
                .filter_map(|n| m.get(n).map(|(_, s)| *s))
                .fold(None, |acc: Option<f64>, s| Some(acc.map_or(s, |a| a.max(s))));
            match best {
                Some(s) => total += s,
                None => {
                    covered = false;
                    break;
                }
            }
        }
        if !covered || toks.is_empty() {
            continue;
        }
        let score = total / toks.len() as f64;
        let label = lk.entry(ctx.store, &c).map(|e| e.label).unwrap_or_default();
        let toks_owned: Vec<String> = qn.split_whitespace().map(str::to_string).collect();
        let mt = if exact_for(qn, &toks_owned, &[c.as_str(), label.as_str()]) { "exact" } else { "text" };
        let rank = if mt == "exact" { 0 } else { 1 };
        board.offer(scored(rank, score, 0, &c, class_value(ctx, lk, &c, mt, score)));
    }
    let _ = limit;
}

/// Each result names its `publisher`, the host of its namespace, and, for a relayed function, its
/// `source`, the domain the values come from, as a separate member (spec-0.6.1, 9.6, 16.8). The
/// relay is the one `describe` names (the function page's), so the two agree.
fn annotate(inner: &Inner, results: &mut [Value]) {
    for r in results.iter_mut() {
        let Value::Object(members) = r else { continue };
        let text = |k: &str| members.iter().find(|(m, _)| m == k).and_then(|(_, v)| v.as_str()).map(str::to_string);
        let Some(name) = text("name") else { continue };
        let (kind, fn_hash) = (text("kind"), text("fn_hash"));
        let publisher = crate::describe::publisher_of(&name).to_string();
        members.push(("publisher".into(), Value::String(publisher)));
        // The entry's facets, when it has them (CONTRACT-classes-round C2).
        let entry_kind = match kind.as_deref() {
            Some("function") => Some(CatalogKind::Function),
            Some("class") => Some(CatalogKind::Class),
            _ => None,
        };
        if let Some(k) = entry_kind {
            let facets = inner.store.catalog_entry_ext(k, &name).ok().flatten().map(|e| e.facets).unwrap_or_default();
            if !facets.is_empty() {
                members.push(("facets".into(), crate::describe::facets_value(&facets)));
            }
        }
        if kind.as_deref() != Some("function") {
            continue;
        }
        let Some(fn_hash) = fn_hash else { continue };
        if let Some(relay) = crate::fnpage::relay_of(inner, &name, &fn_hash) {
            members.push(("source".into(), Value::String(relay.name)));
        }
    }
}

/// `GET /find?q=` (R3): the hybrid answer of `query`, at most `limit` results.
pub(crate) fn answer(inner: &Inner, query: &str, limit: usize) -> Value {
    let ctx = ctx_of(inner);
    let mut lk = Lookup::default();
    let qn = nfc(query.trim());
    let qn = first_words(&qn);
    if qn.is_empty() {
        // The empty query is answered from the find thread's build (see `empty_answer`); this is never reached
        // from the handler, which routes it there.
        return Value::Object(vec![
            ("query".into(), Value::String(query.into())),
            ("vector".into(), Value::Bool(false)),
            ("results".into(), Value::Array(Vec::new())),
        ]);
    }
    let toks: Vec<String> = qn.split_whitespace().map(str::to_string).collect();
    let toks_lower: Vec<String> = toks.iter().map(|t| lower(t)).collect();
    let mut board = Board::default();

    // 1. The text match: functions and methods by the whole query, values by the owner function.
    if let Ok(hits) = ctx.store.text_find(&qn, TEXT_HITS) {
        for h in hits {
            match h {
                Hit::Entry { entry, score, .. } => {
                    let exact = exact_for(&qn, &toks, &[entry.name.as_str(), entry.label.as_str()]);
                    let (rank, mt) = if exact { (0, "exact") } else { (1, "text") };
                    let v = match entry.kind {
                        CatalogKind::Function => function_value(&ctx, &mut lk, &entry.name, None, mt, score),
                        CatalogKind::Method => method_value(&ctx, &mut lk, &entry.name, mt, score),
                        CatalogKind::Class => continue,
                    };
                    board.offer(scored(rank, score, kind_order(entry.kind), &entry.name, v));
                }
                Hit::Value { owner, value, label, aliases, score, .. } => {
                    let mut names = vec![value.as_str(), label.as_str()];
                    names.extend(aliases.iter().map(String::as_str));
                    let exact = exact_for(&qn, &toks, &names);
                    let (rank, mt) = if exact { (0, "exact") } else { (1, "text") };
                    let v = function_value(&ctx, &mut lk, &owner, None, mt, score);
                    board.offer(scored(rank, score, 2, &owner, v));
                }
                _ => {}
            }
        }
    }
    // 2. Classes: every word in the class's own entry or in one of its methods.
    class_coverage(&ctx, &mut lk, &qn, &toks_lower, limit, &mut board);

    // 3. The value resolution (the request reading of the query), then 4. the vector match: the
    // nearest descriptions and values, to fill the remaining places, and the keys the embedding
    // alone matches (candidates, filling nothing).
    let mut read = read_query(&ctx, &qn);
    let mut vector_ok = false;
    if let (Some(ec), true) = (inner.embed.as_ref(), ctx.store.vector_available()) {
        if let Ok(vs) = ec.embed("query: ", std::slice::from_ref(&qn)) {
            vector_ok = true;
            if let Some(v) = vs.first() {
                if let Ok(hits) = ctx.store.vector_find(MODEL, v, limit.max(TEXT_HITS)) {
                    for h in hits {
                        match h {
                            Hit::Entry { entry, score, .. } => {
                                let v = match entry.kind {
                                    CatalogKind::Function => {
                                        function_value(&ctx, &mut lk, &entry.name, None, "vector", score)
                                    }
                                    CatalogKind::Method => method_value(&ctx, &mut lk, &entry.name, "vector", score),
                                    CatalogKind::Class => class_value(&ctx, &mut lk, &entry.name, "vector", score),
                                };
                                board.offer(scored(2, score, kind_order(entry.kind), &entry.name, v));
                            }
                            Hit::Value { owner, score, .. } => {
                                let v = function_value(&ctx, &mut lk, &owner, None, "vector", score);
                                board.offer(scored(2, score, 2, &owner, v));
                            }
                            _ => {}
                        }
                    }
                }
            }
            if let Read::Request { cands, tail, residual } = &mut read {
                let exact_already = cands.len() == 1 && cands[0].full();
                if !exact_already && !tail.is_empty() && !residual.is_empty() {
                    vector_keys(&ctx, ec, residual, tail, cands);
                }
            }
        }
    }
    let resolved = match &read {
        Read::Range => Resolved {
            member: Some(Value::Object(vec![
                ("status".into(), Value::String("range".into())),
                ("error".into(), Value::String("E_RANGE_REFUSED".into())),
            ])),
            exact: None,
        },
        Read::Request { cands, .. } => resolution_of(&ctx, &mut lk, cands),
    };
    // 5. The exact resolution is a result of its own, with its filled call and label.
    if let Some(c) = resolved.exact.clone() {
        let call = filled_call(&c.name, &c.args, &c.fills);
        let label = fill_label(&c.args, &c.fills);
        let v = function_value(&ctx, &mut lk, &c.name, Some((call, label)), "exact", 1.0);
        board.offer(scored(0, 1.0, 2, &c.name, v));
    }

    // 6. Ranked: exact, text, vector; then by score, kind and name. At most `limit`.
    let mut all: Vec<Scored> = board.best.into_values().collect();
    all.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then(b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.kind.cmp(&b.kind))
            .then(a.name.cmp(&b.name))
    });
    all.truncate(limit);
    let mut results: Vec<Value> = all.into_iter().map(|s| s.value).collect();
    annotate(inner, &mut results);

    // 7. The demand record: no result and no resolution.
    if results.is_empty() && resolved.member.is_none() {
        if let Err(e) = ctx.store.log_miss(query.trim()) {
            tracing::warn!(error = %e, "find: the miss is not logged");
        }
    }
    let mut out = vec![
        ("query".into(), Value::String(query.into())),
        ("vector".into(), Value::Bool(vector_ok)),
        ("results".into(), Value::Array(results)),
    ];
    if let Some(r) = resolved.member {
        out.push(("resolution".into(), r));
    }
    Value::Object(out)
}

/// The answers the find thread builds (CONTRACT-0.4.9 H4): the catalogue summary of `GET /find?summary=1`, and the
/// listing of the empty query (the first [`MAX_LIMIT`] entries). A request reads the last build.
pub(crate) struct Built {
    summary: Value,
    listing: Vec<Value>,
}

/// Builds the summary and the listing now (the find thread's work; never a request's).
pub(crate) fn build(inner: &Inner) -> Built {
    let ctx = ctx_of(inner);
    let summary = summary_value(inner, &ctx);
    let mut lk = Lookup::default();
    let mut listing = listing(&ctx, &mut lk, MAX_LIMIT);
    annotate(inner, &mut listing);
    Built { summary, listing }
}

/// The last build of the find thread's answers, or `None` before its first build.
fn built(inner: &Inner) -> Option<Arc<Built>> {
    inner.find.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// `GET /find?summary=1` (R2): the catalogue as an operator's summary, for the tool descriptions. `None` before
/// the first build (`503`).
pub(crate) fn summary(inner: &Inner) -> Option<Value> {
    built(inner).map(|b| b.summary.clone())
}

/// The answer to the empty query: the first `limit` of the last build's listing. `None` before the first build.
pub(crate) fn empty_answer(inner: &Inner, limit: usize) -> Option<Value> {
    let b = built(inner)?;
    let results: Vec<Value> = b.listing.iter().take(limit).cloned().collect();
    Some(Value::Object(vec![
        ("query".into(), Value::String(String::new())),
        ("vector".into(), Value::Bool(false)),
        ("results".into(), Value::Array(results)),
    ]))
}

/// Runs one build of the find answers and makes it the one requests read (the find thread), and counts the open
/// dictionary keys for the metrics (CONTRACT-0.4.9 H7).
pub(crate) fn refresh(inner: &Inner) {
    let b = Arc::new(build(inner));
    *inner.find.lock().unwrap_or_else(|p| p.into_inner()) = Some(b);
    inner.metrics.set_open_keys(count_open_keys(inner));
}

/// The open dictionary keys the store holds (S9, `open_key_count`); 0 when the store cannot count them.
fn count_open_keys(inner: &Inner) -> u64 {
    let store: &dyn Store = &*inner.store;
    store.open_key_count().unwrap_or(0)
}

/// The catalogue summary (see [`summary`]), built by [`build`].
fn summary_value(inner: &Inner, ctx: &Ctx<'_>) -> Value {
    let mut lk = Lookup::default();
    let Ok(s) = ctx.store.catalog_summary() else {
        return Value::Object(vec![
            ("summary".into(), Value::Bool(true)),
            ("mesh".into(), Value::String(inner.config.host.clone())),
        ]);
    };
    let mut dicts: BTreeMap<String, Vec<(String, Value)>> = BTreeMap::new();
    for d in &s.dictionaries {
        let values: Vec<Value> = d
            .values
            .iter()
            .map(|v| {
                Value::Object(vec![
                    ("value".into(), Value::String(v.value.clone())),
                    ("label".into(), Value::String(v.label.clone())),
                ])
            })
            .collect();
        let obj = Value::Object(vec![
            ("distinct".into(), json_number(d.n_values as f64)),
            ("values".into(), Value::Array(values)),
        ]);
        dicts.entry(d.owner.clone()).or_default().push((d.key.clone(), obj));
    }
    let mut entries = s.entries.clone();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let (mut classes, mut functions) = (Vec::new(), Vec::new());
    // The store counts a function's cells by its hash; functions of one descriptor (stored functions of
    // one signature) share it, so a hash bound to several listed names is counted again by name (S6).
    let mut hash_of: BTreeMap<String, String> = BTreeMap::new();
    let mut named: BTreeMap<String, usize> = BTreeMap::new();
    for e in entries.iter().filter(|e| e.kind == CatalogKind::Function) {
        if let Some(b) = ctx.store.resolve_binding(&e.name, &ctx.now).ok().flatten() {
            *named.entry(b.fn_hash.clone()).or_default() += 1;
            hash_of.insert(e.name.clone(), b.fn_hash);
        }
    }
    for e in &entries {
        let bound = match e.kind {
            CatalogKind::Function => hash_of.contains_key(&e.name),
            CatalogKind::Class => ctx.store.resolve_class_binding(&e.name, &ctx.now).ok().flatten().is_some(),
            CatalogKind::Method => false,
        };
        if !bound {
            continue;
        }
        let description = lk.entry(ctx.store, &e.name).map(|x| x.description).unwrap_or_default();
        let keys = dicts.get(&e.name).cloned().unwrap_or_default();
        let cells = match hash_of.get(&e.name) {
            Some(h) if named.get(h).is_some_and(|n| *n > 1) => {
                cells_of_name(ctx.store, &e.name, h).unwrap_or(e.n_cells.max(0) as u64)
            }
            _ => e.n_cells.max(0) as u64,
        };
        let item = Value::Object(vec![
            ("name".into(), Value::String(e.name.clone())),
            ("label".into(), Value::String(e.label.clone())),
            ("description".into(), Value::String(description)),
            ("cells".into(), json_number(cells as f64)),
            ("keys".into(), Value::Object(keys)),
        ]);
        match e.kind {
            CatalogKind::Class => classes.push(item),
            _ => functions.push(item),
        }
    }
    let mut summary = vec![
        ("summary".into(), Value::Bool(true)),
        ("mesh".into(), Value::String(inner.config.host.clone())),
        ("classes".into(), Value::Array(classes)),
        ("functions".into(), Value::Array(functions)),
    ];
    // The catalogue's held text (CONTRACT-classes-round C8), from its last build; absent with no facets.
    if let Some(held) = crate::catalog_view::held(inner) {
        summary.push(("held".into(), Value::String(held)));
    }
    Value::Object(summary)
}

/// The stored cells of the function `name` (hash `hash`), counted by name (one indexed count, S6).
fn cells_of_name(store: &dyn Store, name: &str, hash: &str) -> Option<u64> {
    let (ns, f) = name.rsplit_once('/')?;
    store.cell_count(ns, f, hash).ok()
}

/// Index maintenance (R3): the generic words, the value counts, and the embeddings of the entries
/// and values that have none for the model. Idempotent; a failing embedding service stops the pass.
/// The entries embedded in one call of the index (0.4.1: 16, was 32).
const INDEX_BATCH: usize = 16;

pub(crate) fn reindex(inner: &Inner) {
    let Ok(_guard) = inner.reindex.lock() else { return };
    let store: &dyn Store = &*inner.store;
    // The value counts first `DATUMESH_VALUE_COUNTS_SECS` after start, then at most once per that interval
    // (CONTRACT-0.4.9 H4; CONTRACT-round-0.7-P HP6: not in the first build).
    let due = {
        let mut at = inner.value_counts_at.lock().unwrap_or_else(|p| p.into_inner());
        let due = at.is_none_or(|t| t.elapsed() >= inner.config.value_counts_refresh);
        if due {
            *at = Some(Instant::now());
        }
        due
    };
    if due {
        let _ = store.refresh_value_counts();
    }
    let generic = generic_words(store);
    if let Ok(mut g) = inner.generic.write() {
        *g = Arc::new(generic);
    }
    let (Some(ec), true) = (inner.embed.as_ref(), store.vector_available()) else { return };
    loop {
        let pending = match store.pending_embeddings(MODEL, INDEX_BATCH) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "the index could not read the entries to embed");
                return;
            }
        };
        if pending.is_empty() {
            return;
        }
        let texts: Vec<String> = pending.iter().map(|p| p.text.clone()).collect();
        let vecs = match ec.embed_passages(&texts) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, pending = texts.len(), "the embed service did not embed the index's batch; retried in 30 s");
                return;
            }
        };
        let items: Vec<datumesh_store::Embedding> = pending
            .iter()
            .zip(vecs)
            .map(|(p, v)| datumesh_store::Embedding { ref_kind: p.ref_kind, ref_id: p.ref_id, vec: v })
            .collect();
        if let Err(e) = store.put_embeddings(MODEL, &items) {
            tracing::warn!(error = %e, "the index could not store a batch of embeddings");
            return;
        }
    }
}

/// The operator's catalogue file (`Config::catalog`) as entries of the store (R3): a name that the
/// publisher already described keeps the publisher's entry (the store never replaces a publisher's
/// entry with an operator's), and a name with none gets the operator's text.
pub(crate) fn sync_operator_catalog(inner: &Inner) {
    let store: &dyn Store = &*inner.store;
    let now = inner.now();
    for (name, desc) in &inner.config.catalog {
        if desc.trim().is_empty() || desc.len() > 1024 || name.is_empty() {
            continue;
        }
        let (kind, hash) = if let Ok(Some(cb)) = store.resolve_class_binding(name, &now) {
            (CatalogKind::Class, Some(cb.class_hash))
        } else {
            let parent = name.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
            let is_method = !parent.is_empty()
                && store
                    .resolve_class_binding(&parent, &now)
                    .ok()
                    .flatten()
                    .and_then(|cb| store.class(&cb.class_hash).ok().flatten())
                    .and_then(|bytes| ClassDescriptor::from_json(&bytes[..]).ok())
                    .is_some_and(|cd| cd.methods.contains_key(&name[parent.len() + 1..]));
            let hash = store.resolve_binding(name, &now).ok().flatten().map(|b| b.fn_hash);
            (if is_method { CatalogKind::Method } else { CatalogKind::Function }, hash)
        };
        let label = name.rsplit('/').next().unwrap_or(name).chars().take(120).collect::<String>();
        let entry = CatalogEntry {
            kind,
            name: name.clone(),
            hash,
            label,
            description: desc.clone(),
            args: BTreeMap::new(),
            examples: "[]".to_string(),
            source: datumesh_store::CatalogSource::Operator,
            signed_by: None,
        };
        if let Err(e) = store.upsert_catalog(std::slice::from_ref(&entry)) {
            tracing::warn!(name = %name, error = %e, "catalogue: an operator entry is not stored");
        }
    }
}
