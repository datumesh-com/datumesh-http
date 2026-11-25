//! The catalogue for people and for the summaries (CONTRACT-classes-round C7 to C10, CONTRACT-round-0.7 R7 to R11): the root of
//! `describe` (`describe/<host>/catalog`), the `held` text of `find?summary=1`, the pages `/catalog`, `/catalog/<domain>` (paged at
//! [`DOMAIN_PAGE`] subjects, `?after=`), `/catalog/<domain>/subject?key=…` and `/catalog/rule`, and the class pages (`/fn/<class>`,
//! R9) of the catalogue's classes.
//!
//! The view is read from the cells the catalogue job wrote (`catalog_build`): the subjects that exist, their labels, their sources
//! in rank order. A thread of the mesh builds it after `DATUMESH_CATALOG_FIRST_BUILD_DELAY_SECS` (at once when unset) and then every
//! `DATUMESH_CATALOG_REFRESH_SECS` (default 300 s), and only when the catalogue changed since the last build (the generation: a
//! fingerprint of the faceted entries and their classes; a build that finds it unchanged builds nothing and counts nothing). A
//! request reads the last build and never reads the store for it: before the first build a page or the root is `503` with
//! `Retry-After` and `no-store` (the overview rule of the series pages).

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use datumesh_core::json::Value;
use datumesh_core::ClassDescriptor;
use datumesh_store::{CatalogKind, Store};

use crate::catalog_build::{
    facet_class, lexical_args, lookup_labels, parse_subject_key, relay_domain, subject_class, subject_key_text,
    subject_label, subject_of_args, subject_scalars, Class, SubjectKey, PAGE, RULE, SOURCES_PER_SUBJECT, SUBJECT_KEYS,
};
use crate::envelope::{self, Cache, Reply};
use crate::find::cells_of_name;
use crate::mesh::Inner;
use crate::series::Out;
use datumesh_core::Scalar;

/// The most subjects a domain's `top` lists (C7).
const TOP: usize = 5;

/// The most words of `held` (C8).
const HELD_WORDS: usize = 60;

/// The most measures of a domain `held` shows (C8, R8).
const HELD_MEASURES: usize = 3;

/// The subjects of one page of a domain (R11).
pub(crate) const DOMAIN_PAGE: usize = 200;

/// The example labels a class page lists (R9).
const LABEL_EXAMPLES: usize = 50;

/// The instruction that ends `held` (R8).
const HELD_TAIL: &str = "Ask find in plain words for what you need.";

const HTML: &str = "text/html; charset=utf-8";

/// One subject that exists, with its label and its sources in rank order.
struct Subject {
    key: SubjectKey,
    text: String,
    label: String,
    sources: Vec<String>,
}

/// A domain: its subjects, and its catalogue names.
struct Domain {
    name: String,
    /// Subject indices (into the view's list), by number of sources descending, then key text.
    subjects: Vec<usize>,
    names: usize,
}

/// A class of the catalogue, as the build reads it (R9).
struct ClassInfo {
    name: String,
    hash: String,
    facets: BTreeMap<String, Vec<String>>,
    label: String,
    description: Option<String>,
}

/// The catalogue as the last build read it.
pub(crate) struct Built {
    /// The root of `describe`, as JCS.
    root: Vec<u8>,
    /// The `held` text of the summary, when there are subjects.
    held: Option<String>,
    /// The pages, by path: `""` for `/catalog`, `rule`, `d:<domain>@<first index>`, `s:<key text>`.
    pages: BTreeMap<String, Vec<u8>>,
    /// Each domain's subjects' key texts, in page order.
    domain_order: BTreeMap<String, Vec<String>>,
    /// The class pages, by class name: HTML and JSON.
    classes: BTreeMap<String, ClassPage>,
    /// The generation the build read (see the module documentation).
    generation: u64,
}

/// A class page: its HTML and its JSON form (R9).
struct ClassPage {
    html: Vec<u8>,
    json: Vec<u8>,
}

/// What a build reads: the live subjects, the domains, the catalogue's classes and the example labels.
struct Read {
    subjects: Vec<Subject>,
    domains: Vec<Domain>,
    classes: Vec<ClassInfo>,
    /// The frequency of each faceted entry that has one (its first name), by entry name.
    frequency: BTreeMap<String, String>,
    /// The `source_name` of each faceted entry that has one, by entry name (R8).
    source_names: BTreeMap<String, String>,
    /// The hash of each faceted function entry, by entry name (its cells are counted for `held`, H4).
    fn_hashes: BTreeMap<String, String>,
    /// The hash of each faceted class entry, by entry name (its methods' cells are counted for `held`, 0.5.2).
    class_hashes: BTreeMap<String, String>,
    /// The valued lookup labels of the subject class, in label order.
    labels: Vec<String>,
    /// The generation of what was read: a fingerprint of the faceted entries, their facets and binding instants, and of the
    /// catalogue's classes (hashes, labels, descriptions). It changes exactly when the catalogue changed.
    generation: u64,
}

fn read(inner: &Inner, host: &str, now: &str) -> Option<Read> {
    let store: &dyn Store = &*inner.store;
    let class = subject_class(host);
    // The subjects that exist, a page at a time (H2): at most the cap's, the cut counted; then their labels and sources by key.
    let cap = inner.config.catalog_max;
    let mut subjects: Vec<Subject> = Vec::new();
    let mut lookup: Vec<String> = Vec::new();
    if let Some(sc) = Class::open(store, &class, now).ok()? {
        let mut live: Vec<SubjectKey> = Vec::new();
        let mut after: Option<Vec<u8>> = None;
        'pages: loop {
            let page = sc.page("exists", &SUBJECT_KEYS, after.as_deref(), PAGE).ok()?;
            after = page.last().map(|v| v.sort_key.clone());
            for v in &page {
                if v.value.lexical() != "true" {
                    continue;
                }
                let Some(k) = subject_of_args(&lexical_args(&v.cell)) else { continue };
                if live.len() == cap {
                    inner.metrics.catalog_truncated();
                    tracing::warn!(target: "datumesh_http", cap, "catalogue: the subjects are cut at DATUMESH_CATALOG_MAX");
                    break 'pages;
                }
                live.push(k);
            }
            if page.len() < PAGE {
                break;
            }
        }
        for chunk in live.chunks(PAGE) {
            let keys: Vec<Vec<(String, Scalar)>> = chunk.iter().map(|k| subject_scalars(k, None)).collect();
            let labels = sc.values("label", &keys).ok()?;
            let counts: Vec<usize> = sc
                .values("sources", &keys)
                .ok()?
                .iter()
                .map(|c| c.as_deref().and_then(|c| c.parse::<usize>().ok()).unwrap_or(0))
                .collect();
            let most = counts.iter().copied().max().unwrap_or(0).min(SOURCES_PER_SUBJECT);
            let mut sources: Vec<Vec<String>> = vec![Vec::new(); chunk.len()];
            for r in 1..=most {
                let idx: Vec<usize> = (0..chunk.len()).filter(|&i| counts[i] >= r).collect();
                let rank_keys: Vec<Vec<(String, Scalar)>> =
                    idx.iter().map(|&i| subject_scalars(&chunk[i], Some(r))).collect();
                for (&i, v) in idx.iter().zip(sc.values("source", &rank_keys).ok()?) {
                    sources[i].push(v.unwrap_or_default());
                }
            }
            for (i, k) in chunk.iter().enumerate() {
                subjects.push(Subject {
                    key: k.clone(),
                    text: subject_key_text(host, k),
                    label: labels[i].clone().unwrap_or_else(|| subject_label(k)),
                    sources: std::mem::take(&mut sources[i]),
                });
            }
        }
        // The lookup's examples: the labels that name one subject (as the job writes them), the first in label order (H2:
        // counted over the subjects read, not a slice of the lookup).
        let mut named: BTreeMap<String, usize> = BTreeMap::new();
        for s in &subjects {
            for l in lookup_labels(&s.key) {
                *named.entry(l).or_default() += 1;
            }
        }
        lookup = named.into_iter().filter(|(_, n)| *n == 1).map(|(l, _)| l).take(LABEL_EXAMPLES).collect();
    }
    subjects.sort_by(|a, b| a.text.cmp(&b.text));
    // The domains: their subjects, and the number of catalogue names that have the domain among their facets (C7).
    let mut names: BTreeMap<String, usize> = BTreeMap::new();
    let mut frequency: BTreeMap<String, String> = BTreeMap::new();
    let mut source_names: BTreeMap<String, String> = BTreeMap::new();
    let mut fn_hashes: BTreeMap<String, String> = BTreeMap::new();
    let mut class_hashes: BTreeMap<String, String> = BTreeMap::new();
    let mut class_facets: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut entries = store.catalog_entries_with_facets().ok()?;
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    for e in entries {
        format!("{:?}|{}|{:?}|{:?}|{:?}|{:?}", e.kind, e.name, e.hash, e.facets, e.first_bound, e.source_name)
            .hash(&mut hasher);
        if let Some(s) = &e.source_name {
            source_names.insert(e.name.clone(), s.clone());
        }
        if e.kind == CatalogKind::Function {
            if let Some(h) = &e.hash {
                fn_hashes.insert(e.name.clone(), h.clone());
            }
        }
        if e.kind == CatalogKind::Class {
            if let Some(h) = &e.hash {
                class_hashes.insert(e.name.clone(), h.clone());
            }
        }
        if let Some(f) = e.facets.get("frequency").and_then(|f| f.first()) {
            frequency.insert(e.name.clone(), f.clone());
        }
        for d in e.facets.get("domain").into_iter().flatten() {
            *names.entry(d.clone()).or_default() += 1;
        }
        if e.kind == CatalogKind::Class {
            class_facets.insert(e.name.clone(), e.facets.clone());
        }
    }
    // The catalogue's classes: the faceted ones and the job's own.
    let mut classes: Vec<ClassInfo> = Vec::new();
    let mut class_names: BTreeSet<String> = class_facets.keys().cloned().collect();
    class_names.insert(subject_class(host));
    class_names.insert(facet_class(host));
    for name in class_names {
        let Some(h) = store.resolve_class(&name, now).ok()? else { continue };
        let ext = store.catalog_entry_ext(CatalogKind::Class, &name).ok().flatten();
        let (label, description) = match &ext {
            Some(x) if !x.entry.label.is_empty() => (x.entry.label.clone(), Some(x.entry.description.clone())),
            Some(x) => (name.clone(), Some(x.entry.description.clone())),
            None => (name.clone(), None),
        };
        (&name, &h, &label, &description).hash(&mut hasher);
        classes.push(ClassInfo {
            facets: class_facets.get(&name).cloned().unwrap_or_default(),
            name,
            hash: h,
            label,
            description,
        });
    }
    let mut by_domain: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, s) in subjects.iter().enumerate() {
        by_domain.entry(s.key.0.clone()).or_default().push(i);
    }
    let mut domains: Vec<Domain> = by_domain
        .into_iter()
        .map(|(name, mut idx)| {
            idx.sort_by(|&a, &b| {
                subjects[b]
                    .sources
                    .len()
                    .cmp(&subjects[a].sources.len())
                    .then_with(|| subjects[a].text.cmp(&subjects[b].text))
            });
            let n = names.get(&name).copied().unwrap_or(0);
            Domain { name, subjects: idx, names: n }
        })
        .collect();
    domains.sort_by(|a, b| b.subjects.len().cmp(&a.subjects.len()).then_with(|| a.name.cmp(&b.name)));
    Some(Read {
        subjects,
        domains,
        classes,
        frequency,
        source_names,
        fn_hashes,
        class_hashes,
        labels: lookup,
        generation: hasher.finish(),
    })
}

/// A count as a JSON number.
fn num(n: usize) -> Value {
    datumesh_core::json::parse(n.to_string().as_bytes()).unwrap_or(Value::Null)
}

fn label_of_domain(d: &str) -> String {
    d.replace('_', " ")
}

/// The page of a source: a class's is its `describe`, a function's its function page (C9).
fn source_href(name: &str, is_class: bool) -> String {
    if is_class {
        format!("/describe/{name}")
    } else {
        format!("/fn/{name}")
    }
}

/// The text of a query value, percent-decoded (`+` is a space, as a form has it).
pub(crate) fn unpercent(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(x) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(x);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `qenc`: the query form of a key encoding (alphanumerics and `-._~:` kept, the rest percent-encoded, as the tests write it).
pub(crate) fn query_encode(v: &str) -> String {
    let mut out = String::new();
    for b in v.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A subject's page path, its key in the query.
pub(crate) fn subject_path(domain: &str, text: &str) -> String {
    format!("/catalog/{domain}/subject?key={}", query_encode(text))
}

fn html(title: &str, body: &str) -> Vec<u8> {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{}</style></head><body><main>{body}</main></body></html>\n",
        crate::fnpage::esc(title),
        "body{font:16px/1.5 system-ui,sans-serif;margin:0;padding:16px;color:#1d1d1f;background:#fff;overflow-wrap:anywhere}main{max-width:60rem;margin:0 auto}li{margin:0.25rem 0}h1{font-size:1.5rem}pre{overflow-x:auto;max-width:100%;box-sizing:border-box}"
    )
    .into_bytes()
}

fn domain_page_key(domain: &str, start: usize) -> String {
    format!("d:{domain}@{start}")
}

/// The pages of the catalogue: the root, the rule, each domain's pages of [`DOMAIN_PAGE`] subjects, and each subject's.
fn build_pages(subjects: &[Subject], domains: &[Domain], classes: &BTreeSet<String>) -> BTreeMap<String, Vec<u8>> {
    let mut pages = BTreeMap::new();
    // The root.
    let mut body = String::from("<h1>Catalogue</h1><p>The subjects the mesh holds, by domain.</p><ul>");
    for d in domains {
        body.push_str(&format!(
            "<li><a href=\"/catalog/{}\">{}</a> &middot; {} {}</li>",
            d.name,
            crate::fnpage::esc(&label_of_domain(&d.name)),
            d.subjects.len(),
            if d.subjects.len() == 1 { "subject" } else { "subjects" }
        ));
    }
    body.push_str("</ul><p><a href=\"/catalog/rule\">How the sources of a subject are ranked</a></p>");
    pages.insert(String::new(), html("Catalogue", &body));
    pages.insert("rule".into(), html("How the catalogue ranks sources", &rule_body()));
    for d in domains {
        for (n, chunk) in d.subjects.chunks(DOMAIN_PAGE).enumerate() {
            let start = n * DOMAIN_PAGE;
            let mut body = format!(
                "<h1>{}</h1><p>{} {}, each with its rank one source.</p><ul>",
                crate::fnpage::esc(&label_of_domain(&d.name)),
                d.subjects.len(),
                if d.subjects.len() == 1 { "subject" } else { "subjects" }
            );
            for &i in chunk {
                let s = &subjects[i];
                let src = s.sources.first().cloned().unwrap_or_default();
                body.push_str(&format!(
                    "<li><a href=\"{}\">{}</a> &middot; <a href=\"{}\">{}</a></li>",
                    crate::fnpage::esc(&subject_path(&d.name, &s.text)),
                    crate::fnpage::esc(&s.label),
                    source_href(&src, classes.contains(&src)),
                    crate::fnpage::esc(&src),
                ));
            }
            body.push_str("</ul>");
            if let Some(last) = chunk.last().map(|&i| &subjects[i]) {
                if start + chunk.len() < d.subjects.len() {
                    body.push_str(&format!(
                        "<p><a href=\"/catalog/{}?after={}\">More {}</a></p>",
                        d.name,
                        query_encode(&last.text),
                        crate::fnpage::esc(&label_of_domain(&d.name))
                    ));
                }
            }
            body.push_str("<p><a href=\"/catalog\">All domains</a></p>");
            pages.insert(domain_page_key(&d.name, start), html(&label_of_domain(&d.name), &body));
        }
        for &i in &d.subjects {
            let s = &subjects[i];
            let mut body = format!("<h1>{}</h1><p>Its sources, in rank order.</p><ol>", crate::fnpage::esc(&s.label));
            for src in &s.sources {
                body.push_str(&format!(
                    "<li><a href=\"{}\">{}</a></li>",
                    source_href(src, classes.contains(src)),
                    crate::fnpage::esc(src)
                ));
            }
            body.push_str(&format!(
                "</ol><p><a href=\"/catalog/{}\">{}</a></p>",
                d.name,
                crate::fnpage::esc(&label_of_domain(&d.name))
            ));
            pages.insert(format!("s:{}", s.text), html(&s.label, &body));
        }
    }
    pages
}

fn rule_body() -> String {
    format!(
        "<h1>How the catalogue ranks sources</h1><p>{}</p><p><a href=\"/catalog\">Catalogue</a></p>",
        crate::fnpage::esc(RULE)
    )
}

/// The label of a measure in `held` (R8): the table's words, plural where the table says; any other is its name with `_` as a
/// space.
fn measure_label(m: &str) -> String {
    match m {
        "exchange_rate" => "exchange rates".into(),
        "interest_rate" => "interest rates".into(),
        "yield" => "yields".into(),
        "fundamentals" => "company fundamentals".into(),
        "text" => "texts".into(),
        "provenance" => "provenance".into(),
        "rainfall" => "rainfall".into(),
        "licence" => "licences".into(),
        "ratio" => "ratios".into(),
        "date" => "dates".into(),
        other => other.replace('_', " "),
    }
}

/// The sentence of `held` over the domains' fragments: "This mesh holds A, B and C. <instruction>".
fn held_sentence(frags: &[String]) -> String {
    let list = match frags.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, init)) => format!("{} and {last}", init.join(", ")),
    };
    format!("This mesh holds {list}. {HELD_TAIL}")
}

/// The `held` text (C8, R8): each domain by size, with up to three measures: the measure's label, the frequency in front when it
/// is monthly, weekly or annual, and "from" the source names of the measure's rank-one sources (a source is named only when it is a
/// relay: its `source_name`, else its domain). Over [`HELD_WORDS`] words, the last domains' measures are dropped first (one at a
/// time, from the last domain that has any), then the last domains, until it fits.
fn held_text(
    host: &str,
    subjects: &[Subject],
    domains: &[Domain],
    frequency: &BTreeMap<String, String>,
    source_names: &BTreeMap<String, String>,
    cells: &BTreeMap<String, u64>,
) -> Option<String> {
    if subjects.is_empty() {
        return None;
    }
    // Each domain's measures, as phrases in order.
    let mut doms: Vec<(String, Vec<String>)> = Vec::new();
    for d in domains {
        let mut by_measure: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for &i in &d.subjects {
            by_measure.entry(subjects[i].key.2.as_str()).or_default().push(i);
        }
        // A measure's cells (H4): the stored cells of its rank-one entries, each entry once; then the name.
        let mut measures: Vec<(&str, u64, Vec<usize>)> = by_measure
            .into_iter()
            .map(|(m, idx)| {
                let rank_one: BTreeSet<&str> =
                    idx.iter().filter_map(|&i| subjects[i].sources.first().map(String::as_str)).collect();
                let n = rank_one.iter().map(|s| cells.get(*s).copied().unwrap_or(0)).sum();
                (m, n, idx)
            })
            .collect();
        measures.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        let mut phrases = Vec::new();
        for (m, _, mut idx) in measures.into_iter().take(HELD_MEASURES) {
            idx.sort_by(|&a, &b| subjects[a].text.cmp(&subjects[b].text));
            let mut sources: Vec<String> = Vec::new();
            let mut computed_names: Vec<String> = Vec::new();
            let mut freq: Option<&str> = None;
            for &i in &idx {
                let Some(src) = subjects[i].sources.first() else { continue };
                if freq.is_none() {
                    freq = frequency
                        .get(src)
                        .map(String::as_str)
                        .filter(|f| matches!(*f, "monthly" | "weekly" | "annual"));
                }
                // A relayed source is named by its `source_name`, else by its domain ("from"); any other source that declares a
                // `source_name` is "computed from" it (0.5.2, the rule of 2026-11-23); the rest are named by none.
                if let Some(domain) = relay_domain(host, src) {
                    let named = source_names.get(src).cloned().unwrap_or(domain);
                    if !sources.contains(&named) {
                        sources.push(named);
                    }
                } else if let Some(named) = source_names.get(src) {
                    if !computed_names.contains(named) {
                        computed_names.push(named.clone());
                    }
                }
            }
            let mut phrase = match freq {
                Some(f) => format!("{f} {}", measure_label(m)),
                None => measure_label(m),
            };
            if !sources.is_empty() {
                phrase.push_str(" from ");
                phrase.push_str(&join_names(&sources));
            }
            if !computed_names.is_empty() {
                phrase.push_str(" computed from ");
                phrase.push_str(&join_names(&computed_names));
            }
            phrases.push(phrase);
        }
        doms.push((label_of_domain(&d.name), phrases));
    }
    // Drop the measures of the last domains first, then the last domains.
    let mut shown: Vec<usize> = doms.iter().map(|(_, p)| p.len()).collect();
    let mut k = doms.len();
    let fragment = |i: usize, shown: &[usize]| -> String {
        let (label, phrases) = &doms[i];
        if shown[i] == 0 {
            label.clone()
        } else {
            format!("{label} ({})", phrases[..shown[i]].join(", "))
        }
    };
    loop {
        let frags: Vec<String> = (0..k).map(|i| fragment(i, &shown)).collect();
        let sentence = held_sentence(&frags);
        if sentence.split_whitespace().count() <= HELD_WORDS || k == 0 {
            return Some(sentence);
        }
        match (0..k).rev().find(|&i| shown[i] > 0) {
            Some(i) => shown[i] -= 1,
            None => k -= 1,
        }
    }
}

/// Plain words joined as "A, B and C".
fn join_names(names: &[String]) -> String {
    match names.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, init)) => format!("{} and {last}", init.join(", ")),
    }
}

/// The root of `describe` (C7), as JCS.
fn root_json(host: &str, origin: &str, domains: &[Domain], subjects: &[Subject]) -> Vec<u8> {
    let domain_values: Vec<Value> = domains
        .iter()
        .map(|d| {
            let top: Vec<Value> = d
                .subjects
                .iter()
                .take(TOP)
                .map(|&i| {
                    let s = &subjects[i];
                    Value::Object(vec![
                        ("subject".into(), Value::String(s.text.clone())),
                        ("label".into(), Value::String(s.label.clone())),
                        ("source".into(), Value::String(s.sources.first().cloned().unwrap_or_default())),
                    ])
                })
                .collect();
            Value::Object(vec![
                ("domain".into(), Value::String(d.name.clone())),
                ("label".into(), Value::String(label_of_domain(&d.name))),
                ("subjects".into(), num(d.subjects.len())),
                ("names".into(), num(d.names)),
                ("top".into(), Value::Array(top)),
            ])
        })
        .collect();
    Value::Object(vec![
        ("kind".into(), Value::String("catalog".into())),
        ("host".into(), Value::String(host.into())),
        ("domains".into(), Value::Array(domain_values)),
        ("rule".into(), Value::String(format!("{origin}/catalog/rule"))),
    ])
    .to_jcs()
}

/// The pages of the catalogue's classes (R9): each class's HTML and JSON, from what the build read.
fn class_pages(inner: &Inner, host: &str, now: &str, read: &Read) -> BTreeMap<String, ClassPage> {
    let store: &dyn Store = &*inner.store;
    let origin = inner.config.origin();
    let by_hash: BTreeMap<&str, &ClassInfo> = read.classes.iter().map(|c| (c.hash.as_str(), c)).collect();
    // The subclasses of each class: the catalogue's classes whose parent it is.
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut descriptors: BTreeMap<String, ClassDescriptor> = BTreeMap::new();
    for c in &read.classes {
        let Some(bytes) = store.class(&c.hash).ok().flatten() else { continue };
        let Ok(cd) = ClassDescriptor::from_json(&bytes) else { continue };
        if let Some(p) = &cd.parent {
            children.entry(p.clone()).or_default().push(c.name.clone());
        }
        descriptors.insert(c.hash.clone(), cd);
    }
    let name_of = |hash: &str| -> Option<String> {
        if let Some(c) = by_hash.get(hash) {
            return Some(c.name.clone());
        }
        store.class_names_bound_to(hash, now).ok()?.into_iter().next()
    };
    let job_class = subject_class(host);
    let mut out = BTreeMap::new();
    for c in &read.classes {
        let Some(cd) = descriptors.get(&c.hash) else { continue };
        // The keys, in the descriptor's order, with their types from its stored text.
        let key_types: BTreeMap<String, String> = store
            .class(&c.hash)
            .ok()
            .flatten()
            .and_then(|bytes| datumesh_core::json::parse(&bytes).ok())
            .and_then(|v| match v.get("keys") {
                Some(Value::Object(m)) => Some(m.clone()),
                _ => None,
            })
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| (k, v.get("type").and_then(Value::as_str).unwrap_or("class").to_string()))
            .collect();
        let keys: Vec<Value> = cd
            .order
            .iter()
            .map(|k| {
                let t = key_types.get(k).cloned().unwrap_or_default();
                Value::Object(vec![("name".into(), Value::String(k.clone())), ("type".into(), Value::String(t))])
            })
            .collect();
        // The methods, each with the page of the class that defines it (K5): an inherited method is the parent's.
        let mut methods: Vec<Value> = Vec::new();
        let mut method_pages: Vec<(String, String)> = Vec::new();
        for (m, h) in &cd.methods {
            let mut cur = cd.clone();
            let mut defining = c.name.clone();
            while let Some(ph) = cur.parent.clone() {
                let Some(pb) = store.class(&ph).ok().flatten() else { break };
                let Ok(pd) = ClassDescriptor::from_json(&pb) else { break };
                if pd.methods.get(m) != Some(h) {
                    break;
                }
                defining = name_of(&ph).unwrap_or(defining);
                cur = pd;
            }
            let page = format!("/fn/{defining}/{m}");
            method_pages.push((m.clone(), page.clone()));
            methods.push(Value::Object(vec![
                ("name".into(), Value::String(m.clone())),
                ("page".into(), Value::String(format!("{origin}{page}"))),
            ]));
        }
        let parent = cd.parent.as_deref().and_then(name_of);
        let mut subclasses: Vec<String> = children.get(&c.hash).cloned().unwrap_or_default();
        subclasses.sort();
        // The subjects it is a source of, with its rank.
        let mut subjects: Vec<(String, String, usize)> = Vec::new();
        for s in &read.subjects {
            for (i, src) in s.sources.iter().enumerate() {
                if *src == c.name {
                    subjects.push((s.text.clone(), s.label.clone(), i + 1));
                }
            }
        }
        // The objects: the stored counter of the class's first method.
        let objects = match cd.methods.iter().next() {
            Some((m, h)) => store.cell_count(&c.name, m, h).unwrap_or(0) as usize,
            None => 0,
        };
        let labels: Vec<String> = if c.name == job_class { read.labels.clone() } else { Vec::new() };
        let facets: Value = Value::Object(
            c.facets
                .iter()
                .map(|(k, v)| (k.clone(), Value::Array(v.iter().map(|x| Value::String(x.clone())).collect())))
                .collect(),
        );
        let steward = Value::Object(vec![
            ("namespace".into(), Value::String(cd.steward.namespace.clone())),
            ("process".into(), Value::String(cd.steward.process.clone())),
        ]);
        let mut members: Vec<(&str, Value)> = vec![
            ("name", Value::String(c.name.clone())),
            ("label", Value::String(c.label.clone())),
            ("description", c.description.clone().map_or(Value::Null, Value::String)),
            ("keys", Value::Array(keys.clone())),
            ("methods", Value::Array(methods.clone())),
        ];
        if let Some(p) = &parent {
            members.push(("parent", Value::String(p.clone())));
        }
        members.push(("subclasses", Value::Array(subclasses.iter().map(|x| Value::String(x.clone())).collect())));
        members.push(("facets", facets.clone()));
        members.push((
            "subjects",
            Value::Array(
                subjects
                    .iter()
                    .map(|(t, l, r)| {
                        Value::Object(vec![
                            ("subject".into(), Value::String(t.clone())),
                            ("label".into(), Value::String(l.clone())),
                            ("rank".into(), num(*r)),
                        ])
                    })
                    .collect(),
            ),
        ));
        members.push(("objects", num(objects)));
        members.push(("labels", Value::Array(labels.iter().map(|x| Value::String(x.clone())).collect())));
        members.push(("steward", steward));
        let json = Value::Object(members.into_iter().map(|(k, v)| (k.to_string(), v)).collect()).to_jcs();
        // The HTML: the same facts, with the links.
        let e = crate::fnpage::esc;
        let mut body = format!("<h1>{}</h1>", e(&c.label));
        body.push_str(&format!("<p><code>{}</code></p>", e(&c.name)));
        if let Some(d) = c.description.as_deref().filter(|d| !d.is_empty()) {
            body.push_str(&format!("<p>{}</p>", e(d)));
        }
        body.push_str("<h2>Keys</h2><ul>");
        for k in &cd.order {
            body.push_str(&format!("<li>{} ({})</li>", e(k), e(&key_types.get(k).cloned().unwrap_or_default())));
        }
        body.push_str("</ul><h2>Methods</h2><ul>");
        for (m, page) in &method_pages {
            body.push_str(&format!("<li><a href=\"{}\">{}</a></li>", e(page), e(m)));
        }
        body.push_str("</ul>");
        if let Some(p) = &parent {
            body.push_str(&format!("<p>Parent: <a href=\"/fn/{}\">{}</a></p>", e(p), e(p)));
        }
        body.push_str("<p>Subclasses: ");
        if subclasses.is_empty() {
            body.push_str("none");
        }
        for (i, s) in subclasses.iter().enumerate() {
            if i > 0 {
                body.push_str(", ");
            }
            body.push_str(&format!("<a href=\"/fn/{}\">{}</a>", e(s), e(s)));
        }
        body.push_str("</p>");
        if !c.facets.is_empty() {
            body.push_str("<h2>Facets</h2><ul>");
            for (k, v) in &c.facets {
                body.push_str(&format!("<li>{}: {}</li>", e(k), e(&v.join(", "))));
            }
            body.push_str("</ul>");
        }
        if !subjects.is_empty() {
            body.push_str("<h2>Subjects it is a source of</h2><ul>");
            for (t, l, r) in &subjects {
                let dom = parse_subject_key_domain(host, t).unwrap_or_default();
                body.push_str(&format!("<li><a href=\"{}\">{}</a>, rank {r}</li>", e(&subject_path(&dom, t)), e(l)));
            }
            body.push_str("</ul>");
        }
        body.push_str(&format!("<p>Objects: {objects}</p>"));
        if !labels.is_empty() {
            body.push_str("<h2>Example labels</h2><ul>");
            for l in &labels {
                body.push_str(&format!("<li>{}</li>", e(l)));
            }
            body.push_str("</ul>");
        }
        body.push_str(&format!(
            "<h2>Steward</h2><p>{} · <a href=\"{}\">{}</a></p>",
            e(&cd.steward.namespace),
            e(&cd.steward.process),
            e(&cd.steward.process)
        ));
        out.insert(c.name.clone(), ClassPage { html: html(&c.label, &body), json });
    }
    out
}

/// The domain of a subject's key text, when it is one of this host's subjects.
fn parse_subject_key_domain(host: &str, text: &str) -> Option<String> {
    parse_subject_key(host, text).map(|k| k.0)
}

/// The stored cells of a class: the cells of each of its methods' functions, counted by name (`class` is the class's name, and
/// `hash` its class hash), as the counter of each method reads them.
fn class_cells(store: &dyn Store, class: &str, hash: &str) -> u64 {
    let Some(bytes) = store.class(hash).ok().flatten() else { return 0 };
    let Ok(cd) = ClassDescriptor::from_json(&bytes) else { return 0 };
    cd.methods.iter().map(|(m, h)| store.cell_count(class, m, h).unwrap_or(0)).sum()
}

/// Builds the view from what [`read`] read (the catalogue thread's work, never a request's).
fn build_from(inner: &Inner, read: Read) -> Built {
    let host = inner.config.host.clone();
    let origin = inner.config.origin();
    let now = inner.now();
    let root = root_json(&host, &origin, &read.domains, &read.subjects);
    // The stored cells of each rank-one source (H4): a count per entry, read here, never by a request.
    // A class entry's cells are the stored cells of its methods' functions (0.5.2), read here once per build.
    let mut cells: BTreeMap<String, u64> = BTreeMap::new();
    for s in &read.subjects {
        if let Some(src) = s.sources.first() {
            if cells.contains_key(src) {
                continue;
            }
            let n = match (read.fn_hashes.get(src), read.class_hashes.get(src)) {
                (Some(h), _) => cells_of_name(&*inner.store, src, h).unwrap_or(0),
                (None, Some(h)) => class_cells(&*inner.store, src, h),
                (None, None) => 0,
            };
            cells.insert(src.clone(), n);
        }
    }
    let held = held_text(&host, &read.subjects, &read.domains, &read.frequency, &read.source_names, &cells);
    let class_names: BTreeSet<String> = read.classes.iter().map(|c| c.name.clone()).collect();
    let pages = build_pages(&read.subjects, &read.domains, &class_names);
    let domain_order = read
        .domains
        .iter()
        .map(|d| (d.name.clone(), d.subjects.iter().map(|&i| read.subjects[i].text.clone()).collect()))
        .collect();
    let classes = class_pages(inner, &host, &now, &read);
    Built { root, held, pages, domain_order, classes, generation: read.generation }
}

/// Runs one build when the catalogue changed since the last one (or none is built yet), and makes it the one requests read;
/// counted as `by`. A build that finds the generation unchanged builds nothing and counts nothing.
pub(crate) fn rebuild(inner: &Inner, by: &'static str) {
    let now = inner.now();
    let host = inner.config.host.clone();
    let Some(read) = read(inner, &host, &now) else { return };
    if built(inner).is_some_and(|b| b.generation == read.generation) {
        return;
    }
    let b = build_from(inner, read);
    *inner.catalog.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(b));
    inner.metrics.catalog_build(by);
}

fn built(inner: &Inner) -> Option<Arc<Built>> {
    inner.catalog.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// The `held` text of the summary, from the last build (`None` before it, or with no subjects).
pub(crate) fn held(inner: &Inner) -> Option<String> {
    built(inner)?.held.clone()
}

/// The root of `describe` (`describe/<host>/catalog`): the last build, or `503` before it.
pub(crate) fn describe_root(inner: &Inner) -> Reply {
    match built(inner) {
        Some(b) => Reply::json(200, b.root.clone(), Cache::Public(60)),
        None => envelope::not_yet_reply(inner.config.catalog_refresh.as_secs(), "The catalogue"),
    }
}

/// A class page (R9) for `GET /fn/<class name>`, from the last build: `503` before it, `404` for a class the catalogue does not
/// describe.
pub(crate) fn class_page(inner: &Inner, name: &str, json_form: bool) -> Reply {
    let Some(b) = built(inner) else {
        return envelope::not_yet_reply(inner.config.catalog_refresh.as_secs(), "The class page");
    };
    match b.classes.get(name) {
        Some(p) if json_form => Reply::json(200, p.json.clone(), Cache::Public(60)),
        Some(p) => Reply {
            status: 200,
            body: p.html.clone().into(),
            content_type: HTML,
            cache: Cache::Public(60),
            canonical: None,
            retry_after: None,
        },
        None => Reply {
            status: 404,
            body: html("not covered", "<h1>not_covered</h1><p>No catalogue page of that class.</p>").into(),
            content_type: HTML,
            cache: Cache::Public(60),
            canonical: None,
            retry_after: None,
        },
    }
}

fn unavailable(inner: &Inner) -> Out {
    Out {
        status: 503,
        body: html("not yet", "<h1>not yet</h1><p>The catalogue is being built. Try again shortly.</p>"),
        content_type: HTML,
        cache: "no-store".to_string(),
        etag: None,
        retry_after: Some(inner.config.catalog_refresh.as_secs().clamp(1, 300)),
    }
}

fn not_found() -> Out {
    Out {
        status: 404,
        body: html("not found", "<h1>not found</h1><p>No such catalogue page.</p>"),
        content_type: HTML,
        cache: "public, max-age=60".to_string(),
        etag: None,
        retry_after: None,
    }
}

fn page_out(body: Vec<u8>) -> Out {
    Out {
        status: 200,
        body,
        content_type: HTML,
        cache: "public, max-age=300".to_string(),
        etag: None,
        retry_after: None,
    }
}

/// The answer to `GET /catalog/<rest>`: `rest` is what follows `/catalog/` (empty for the root), `key` the decoded `key` of the
/// query, `after` the decoded `after` (the last subject of the previous page of a domain). A request reads the last build and builds
/// nothing; before the first build every page is `503`.
/// [`respond`] with the query's `after` (R11: the next page of a domain follows its last subject): the http route passes it.
pub(crate) fn respond_paged(inner: &Inner, rest: &str, key: Option<&str>, after: Option<&str>) -> Out {
    let Some(b) = built(inner) else { return unavailable(inner) };
    let host = inner.config.host.as_str();
    if rest.is_empty() {
        return b.pages.get("").cloned().map_or_else(not_found, page_out);
    }
    if rest == "rule" {
        return b.pages.get("rule").cloned().map_or_else(not_found, page_out);
    }
    match rest.split_once('/') {
        None => {
            let Some(order) = b.domain_order.get(rest) else { return not_found() };
            let start = match after {
                None => 0,
                Some(a) => match order.iter().position(|t| t == a) {
                    Some(p) => p + 1,
                    None => return not_found(),
                },
            };
            match b.pages.get(&domain_page_key(rest, start)) {
                Some(p) => page_out(p.clone()),
                None => not_found(),
            }
        }
        Some((domain, "subject")) => {
            let Some(text) = key else { return not_found() };
            let Some(k) = parse_subject_key(host, text) else {
                return Out {
                    status: 400,
                    body: html("bad key", "<h1>bad key</h1><p>Not a subject key.</p>"),
                    content_type: HTML,
                    cache: "public, max-age=60".to_string(),
                    etag: None,
                    retry_after: None,
                };
            };
            if k.0 != domain {
                return not_found();
            }
            match b.pages.get(&format!("s:{}", subject_key_text(host, &k))) {
                Some(p) => page_out(p.clone()),
                None => not_found(),
            }
        }
        Some(_) => not_found(),
    }
}
