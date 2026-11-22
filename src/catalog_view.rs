//! The catalogue for people and for the summaries (CONTRACT-classes-round C7 to C10): the root of `describe`
//! (`describe/<host>/catalog`), the `held` text of `find?summary=1`, and the pages `/catalog`, `/catalog/<domain>`,
//! `/catalog/<domain>/subject?key=…` and `/catalog/rule`.
//!
//! The view is read from the cells the catalogue job wrote (`catalog_build`): the subjects that exist, their labels, their
//! sources in rank order. A thread of the mesh reads it at start (unless `DATUMESH_CATALOG_FIRST_BUILD_DELAY_SECS` is set,
//! then after it) and every `DATUMESH_CATALOG_REFRESH_SECS` (default 300 s), and the answers are built from it then: the pages
//! as HTML, the root as JSON. A request reads the last build and never reads the store for it: before the first build a page or
//! the root is `503` with `Retry-After` and `no-store` (the overview rule of the series pages).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use datumesh_core::json::Value;
use datumesh_store::{CatalogKind, Store};

use crate::catalog_build::{
    cells_of, parse_subject_key, subject_class, subject_key_text, subject_label, subject_of_args, SubjectKey, RULE,
};
use crate::envelope::{self, Cache, Reply};
use crate::mesh::Inner;
use crate::series::Out;

/// The most subjects a domain's `top` lists (C7).
const TOP: usize = 5;

/// The most words of `held` (C8).
const HELD_WORDS: usize = 60;

/// The most measures of a domain `held` shows (C8).
const HELD_MEASURES: usize = 3;

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

/// The catalogue as the last build read it.
pub(crate) struct Built {
    /// The root of `describe`, as JCS.
    root: Vec<u8>,
    /// The `held` text of the summary, when there are subjects.
    held: Option<String>,
    /// The pages, by path: `""` for `/catalog`, `rule`, `d:<domain>`, `s:<key text>`.
    pages: BTreeMap<String, Vec<u8>>,
}

/// What a build reads: the live subjects, the domains, and the names of the catalogue that are classes.
struct Read {
    subjects: Vec<Subject>,
    domains: Vec<Domain>,
    classes: BTreeSet<String>,
}

fn read(inner: &Inner, host: &str, now: &str) -> Option<Read> {
    let store: &dyn Store = &*inner.store;
    let class = subject_class(host);
    let hash = store.resolve_class(&class, now).ok()?;
    let read_cells = |method: &str| cells_of(store, &class, hash.as_deref(), method).ok();
    let mut exists: BTreeMap<SubjectKey, bool> = BTreeMap::new();
    for (args, value) in read_cells("exists")? {
        if let Some(k) = subject_of_args(&args) {
            exists.insert(k, value == "true");
        }
    }
    let mut labels: BTreeMap<SubjectKey, String> = BTreeMap::new();
    for (args, value) in read_cells("label")? {
        if let Some(k) = subject_of_args(&args) {
            labels.insert(k, value);
        }
    }
    let mut counts: BTreeMap<SubjectKey, usize> = BTreeMap::new();
    for (args, value) in read_cells("sources")? {
        if let (Some(k), Ok(n)) = (subject_of_args(&args), value.parse::<usize>()) {
            counts.insert(k, n);
        }
    }
    let mut sources: BTreeMap<(SubjectKey, usize), String> = BTreeMap::new();
    for (args, value) in read_cells("source")? {
        if let (Some(k), Some(r)) = (subject_of_args(&args), args.get("rank").and_then(|r| r.parse::<usize>().ok())) {
            sources.insert((k, r), value);
        }
    }
    let mut subjects: Vec<Subject> = Vec::new();
    for (k, live) in &exists {
        if !live {
            continue;
        }
        let n = counts.get(k).copied().unwrap_or(0);
        let text = subject_key_text(host, k);
        subjects.push(Subject {
            key: k.clone(),
            label: labels.get(k).cloned().unwrap_or_else(|| subject_label(k)),
            sources: (1..=n).map(|r| sources.get(&(k.clone(), r)).cloned().unwrap_or_default()).collect(),
            text,
        });
    }
    subjects.sort_by(|a, b| a.text.cmp(&b.text));
    // The domains: their subjects, and the number of catalogue names that have the domain among their facets (C7).
    let mut names: BTreeMap<String, usize> = BTreeMap::new();
    let mut classes: BTreeSet<String> = BTreeSet::new();
    for e in store.catalog_entries_with_facets().ok()? {
        if e.kind == CatalogKind::Class {
            classes.insert(e.name.clone());
        }
        for d in e.facets.get("domain").into_iter().flatten() {
            *names.entry(d.clone()).or_default() += 1;
        }
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
    Some(Read { subjects, domains, classes })
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
        "body{font:16px/1.5 system-ui,sans-serif;margin:0;padding:16px;color:#1d1d1f;background:#fff}main{max-width:60rem;margin:0 auto}li{margin:0.25rem 0}h1{font-size:1.5rem}"
    )
    .into_bytes()
}

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
        let mut body = format!(
            "<h1>{}</h1><p>{} {}, each with its rank one source.</p><ul>",
            crate::fnpage::esc(&label_of_domain(&d.name)),
            d.subjects.len(),
            if d.subjects.len() == 1 { "subject" } else { "subjects" }
        );
        for &i in &d.subjects {
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
        body.push_str("</ul><p><a href=\"/catalog\">All domains</a></p>");
        pages.insert(format!("d:{}", d.name), html(&label_of_domain(&d.name), &body));
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

/// The `held` text (C8): domains by size, each with up to three measures and the rank-one source of one of their subjects; in at
/// most [`HELD_WORDS`] words; the domains that do not fit are left out, the smaller first.
fn held_text(subjects: &[Subject], domains: &[Domain]) -> Option<String> {
    if subjects.is_empty() {
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    let mut words = 0usize;
    for d in domains {
        let mut by_measure: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for &i in &d.subjects {
            by_measure.entry(subjects[i].key.2.as_str()).or_default().push(i);
        }
        let mut measures: Vec<(&str, Vec<usize>)> = by_measure.into_iter().collect();
        measures.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(b.0)));
        let mut frag = format!("{}:", label_of_domain(&d.name));
        for (m, mut idx) in measures.into_iter().take(HELD_MEASURES) {
            idx.sort_by(|&a, &b| subjects[a].text.cmp(&subjects[b].text));
            let Some(&first) = idx.first() else { continue };
            let src = subjects[first].sources.first().cloned().unwrap_or_default();
            if src.is_empty() {
                continue;
            }
            frag.push_str(&format!(" {} {};", label_of_domain(m), src));
        }
        let n = frag.split_whitespace().count();
        if words + n > HELD_WORDS {
            break;
        }
        words += n;
        out.push(frag);
    }
    Some(out.join(" "))
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

/// Builds the view from the store as of `now` (the catalogue thread's work, never a request's).
pub(crate) fn build(inner: &Inner) -> Option<Built> {
    let now = inner.now();
    let host = inner.config.host.clone();
    let origin = inner.config.origin();
    let Read { subjects, domains, classes } = read(inner, &host, &now)?;
    let root = root_json(&host, &origin, &domains, &subjects);
    let held = held_text(&subjects, &domains);
    let pages = build_pages(&subjects, &domains, &classes);
    Some(Built { root, held, pages })
}

/// Runs one build and makes it the one requests read; counted as `by`.
pub(crate) fn rebuild(inner: &Inner, by: &'static str) {
    if let Some(b) = build(inner) {
        *inner.catalog.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(b));
    }
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
/// query. A request reads the last build and builds nothing; before the first build every page is `503`.
pub(crate) fn respond(inner: &Inner, rest: &str, key: Option<&str>) -> Out {
    let Some(b) = built(inner) else { return unavailable(inner) };
    let host = inner.config.host.as_str();
    if rest.is_empty() {
        return b.pages.get("").cloned().map_or_else(not_found, page_out);
    }
    if rest == "rule" {
        return b.pages.get("rule").cloned().map_or_else(not_found, page_out);
    }
    match rest.split_once('/') {
        None => match b.pages.get(&format!("d:{rest}")) {
            Some(p) => page_out(p.clone()),
            None => not_found(),
        },
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
