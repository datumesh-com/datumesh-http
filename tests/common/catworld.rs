//! Fixtures of the catalogue phase 1 tests (CONTRACT-classes-round.md C1 to C10): worlds of names with facets on the 1.8b
//! SQLite store (fixed clocks), the catalogue job run over them, and the pages and answers read over HTTP.
//!
//! # The interfaces these tests pin (the implementer follows them)
//!
//! * The job, as a library function (the binary's `datumesh-mesh catalog-build` calls it with the system clock):
//!
//!   ```text
//!   datumesh_http::catalog_build::catalog_build_with<S: Store + ?Sized>(
//!       store: &S, key: &SigningKey, host: &str, licence: &str, opts: &PublishOptions)
//!       -> Result<CatalogBuilt, E>      // E: Display; CatalogBuilt: Debug, with the pub field `cells: u64`
//!   ```
//!
//!   `cells` is the number of cell versions the run wrote (classes, methods and bindings are not counted); a run over a store
//!   it has already built, with nothing changed, writes nothing at all (`cells == 0` and the store's file is unchanged).
//! * The binary: `datumesh-mesh catalog-build` takes no flag. It reads `DATABASE_URL`, `DATUMESH_HOST`, `DATUMESH_KEY_FILE` and
//!   `DATUMESH_LICENCE` (the licence of the cells it writes), and prints one JCS line `{"cells":N,...}`; exit 1 with the
//!   refusal on stderr.
//! * The job's names (all under the mesh host, all reserved by C1): the classes `catalog/subject` and `catalog/facet`, their
//!   methods, and the function `catalog/rule` (no argument, returns a text blob).
//! * Page paths: `/catalog`, `/catalog/<domain>`, `/catalog/<domain>/subject?key=<percent-encoded key encoding>`, and
//!   `/catalog/rule`. A source that is a class links to `/describe/<class>` (the class page), a function to `/fn/<name>`.
//! * Knobs and metrics of the pages (the series pattern): `DATUMESH_CATALOG_FIRST_BUILD_DELAY_SECS`,
//!   `DATUMESH_CATALOG_REFRESH_SECS`, and `datumesh_catalog_builds_total{by="background"|"request"}`.
#![allow(dead_code)]

use std::time::{Duration, Instant};

use datumesh_http::bundle::{self, BundleError, Imported};
use datumesh_http::Config;
use datumesh_store::Store;
use serde_json::{json, Map, Value};

use crate::common::lite::*;
use crate::common::pub031::{function_line, stored_descriptor, write, BEFORE, T1, T2};
use crate::common::readpath::*;
use crate::common::world::{call_path, class_json, qenc};
use crate::common::*;
use crate::crworld::*;

/// The licence the job is given (`DATUMESH_LICENCE`): not the one of the fixtures' own cells.
pub const LICENCE: &str = "CC-BY-4.0";
/// The instants of the job's runs: after every publication of the worlds, before the mesh clock.
pub const T3: &str = "2026-11-15T12:00:03.000000Z";
pub const T4: &str = "2026-11-15T12:00:04.000000Z";
pub const T5: &str = "2026-11-15T12:00:05.000000Z";
pub const T6: &str = "2026-11-15T12:00:06.000000Z";

pub fn subject_class() -> String {
    format!("{HOST}/catalog/subject")
}
pub fn facet_class() -> String {
    format!("{HOST}/catalog/facet")
}
pub fn rule_fn() -> String {
    format!("{HOST}/catalog/rule")
}

/// The key encoding of a subject (16.5: names ascending, no space).
pub fn sk(domain: &str, thing: &str, measure: &str, region: &str) -> String {
    format!(r#"{}(domain:"{domain}",measure:"{measure}",region:"{region}",thing:"{thing}")"#, subject_class())
}

/// The key encoding of a facet object.
pub fn fk(facet: &str, value: &str) -> String {
    format!(r#"{}(facet:"{facet}",value:"{value}")"#, facet_class())
}

/// A call on a subject, keys in class order (domain, thing, measure, region), as a request writes it.
pub fn sc(domain: &str, thing: &str, measure: &str, region: &str, method: &str) -> String {
    format!("{}({domain}, {thing}, {measure}, {region}).{method}", subject_class())
}

/// A call on a facet object.
pub fn fc(facet: &str, value: &str, method: &str) -> String {
    format!("{}({facet}, {value}).{method}", facet_class())
}

// ---------------------------------------------------------------------------------------------------------------------
// Entries with facets

/// The facets of an entry; a key whose list is empty is left out.
pub fn fac(domain: &[&str], thing: &[&str], measure: &[&str], region: &[&str], frequency: &[&str]) -> Value {
    let mut o = Map::new();
    for (k, v) in
        [("domain", domain), ("thing", thing), ("measure", measure), ("region", region), ("frequency", frequency)]
    {
        if !v.is_empty() {
            o.insert(k.to_string(), json!(v));
        }
    }
    Value::Object(o)
}

/// A function of one date argument and a number.
pub fn date_sig() -> String {
    format!(r#"{{"args":{{"date":{DATE_V}}},"order":["date"],"returns":{NUM1}}}"#)
}

pub fn fn_line(name: &str) -> String {
    function_line(name, &date_sig(), None)
}

/// A cell of a relayed function: it has a `source`, so its trust is `relayed`.
pub fn relay_cell(name: &str) -> String {
    let rest = name.split("/relay/").nth(1).unwrap_or("x.org/y");
    let domain = rest.split('/').next().unwrap();
    json!({"cell": {"fn": name, "args": {"date": {"type": "date", "value": DAY}}}, "licence": "CC0-1.0",
        "summary": "Synthetic test data.", "value": {"type": "number", "value": "1.5"},
        "source": {"name": domain, "url": format!("https://{domain}/data.csv"), "retrieved": "2026-11-15T10:00:00.000000Z",
            "takeover": format!("https://{HOST}/takeover/{domain}")}})
    .to_string()
}

pub fn cat_line(name: &str, facets: Value) -> String {
    catalog_line_of(name, "function", json!({"facets": facets}))
}

/// The lines of a file of functions with facets: the functions (the relayed ones with a relayed cell), then the catalogue.
pub fn entries_lines(entries: &[(String, Value)]) -> Vec<String> {
    let mut v: Vec<String> = entries.iter().map(|(n, _)| fn_line(n)).collect();
    v.extend(entries.iter().filter(|(n, _)| n.contains("/relay/")).map(|(n, _)| relay_cell(n)));
    v.extend(entries.iter().map(|(n, f)| cat_line(n, f.clone())));
    v
}

pub fn host_name(rest: &str) -> String {
    format!("{HOST}/{rest}")
}

// ---------------------------------------------------------------------------------------------------------------------
// Foreign publishers (a bundle of their own namespace, signed with their own test key)

fn foreign_entry(name: &str, facets: Option<Value>) -> Vec<u8> {
    let mut v = json!({"name": name, "kind": "function", "label": "A foreign name", "description": "A foreign description.",
        "args": {}, "examples": [], "dictionary": {}});
    if let Some(f) = facets {
        v["facets"] = f;
    }
    v.to_string().into_bytes()
}

/// The bundle of the stored function `name` (under `ns`) with a catalogue entry carrying `facets`, signed by `host`'s key.
pub fn foreign_bundle(host: &str, ns: &str, name: &str, facets: Option<Value>) -> Vec<u8> {
    let d = datumesh_core::FunctionDescriptor::from_json(stored_descriptor(&date_sig()).as_bytes()).unwrap();
    bundle::build_with_catalog(
        ns,
        vec![d.canonical()],
        vec![(name.to_string(), d.hash())],
        vec![],
        vec![foreign_entry(name, facets)],
        &seed_for(host),
    )
    .unwrap()
}

/// Imports `bytes` (a bundle of `host`'s namespace) at `clock`, after putting `host`'s key document in the store.
pub fn import_foreign(lite: &Lite, host: &str, bytes: &[u8], clock: &'static str) -> Result<Imported, BundleError> {
    lite.store.put_namespace(host, &key_doc(host, &key_for(host)), K_FROM).unwrap();
    let owned = vec![host.to_string()];
    bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(clock), bytes)
}

// ---------------------------------------------------------------------------------------------------------------------
// World R: the ranking of one subject, S1 = finance, rate, interest_rate, ea
//
// Names, by the instant their name was first bound:
//   10:00:00  agg.example/rates/ecb_policy             a third party's non-relayed republication      (level 3)
//   11:00:00  <host>/relay/ecb.europa.eu/policy_rate   a relay, trust relayed                          (level 2)
//   11:59:59  <host>/finance/policy_double, policy_quad  views: double reads the relay, quad reads double (level 2, tie)
//   12:00:00  <host>/finance/policy_book               a class whose method `rate` reads the relay      (level 2)
//   12:00:01  data.ecb.europa.eu/api/policy_rate       a sub-domain of the source domain               (level 1)
//   12:00:02  ecb.europa.eu/data/policy_rate           the source's own domain                         (level 1)
// and the subject S2 = finance, rate, interest_rate, gb, with the one relay <host>/relay/bankofengland.co.uk/bank_rate.

pub const T_AGG: &str = "2026-11-15T10:00:00.000000Z";
pub const T_RELAY: &str = "2026-11-15T11:00:00.000000Z";

pub fn agg() -> String {
    "agg.example/rates/ecb_policy".to_string()
}
pub fn relay_policy() -> String {
    host_name("relay/ecb.europa.eu/policy_rate")
}
pub fn view_double() -> String {
    host_name("finance/policy_double")
}
pub fn view_quad() -> String {
    host_name("finance/policy_quad")
}
pub fn view_book() -> String {
    host_name("finance/policy_book")
}
pub fn sub_domain() -> String {
    "data.ecb.europa.eu/api/policy_rate".to_string()
}
pub fn own_domain() -> String {
    "ecb.europa.eu/data/policy_rate".to_string()
}
pub fn relay_bank() -> String {
    host_name("relay/bankofengland.co.uk/bank_rate")
}

pub fn s1() -> Value {
    fac(&["finance"], &["rate"], &["interest_rate"], &["ea"], &["on_change"])
}

/// The expected order of the sources of S1.
pub fn ranked_names() -> Vec<String> {
    vec![sub_domain(), own_domain(), relay_policy(), view_double(), view_quad(), view_book(), agg()]
}

fn book_sig() -> String {
    format!(
        r#"{{"args":{{"date":{DATE_V},"n":{{"type":"number","unit":"1","valid":false}}}},"order":["n","date"],"returns":{NUM1}}}"#
    )
}

/// World R, before the job.
pub fn ranked(label: &str) -> Lite {
    let lite = Lite::new(label);
    let s1 = s1();
    import_foreign(
        &lite,
        "agg.example",
        &foreign_bundle("agg.example", "agg.example/rates", &agg(), Some(s1.clone())),
        T_AGG,
    )
    .unwrap_or_else(|e| panic!("agg: {e}"));
    let rp = relay_policy();
    let a = entries_lines(&[
        (rp.clone(), s1.clone()),
        (relay_bank(), fac(&["finance"], &["rate"], &["interest_rate"], &["gb"], &["daily"])),
    ]);
    crate::crworld::publish(&lite, "relay.jsonl", &a, T_RELAY).unwrap_or_else(|e| panic!("relay: {e}"));
    let book_rate = format!("{}/rate", view_book());
    let views = vec![
        formula_line(&view_double(), &format!("{rp}(date: date) * 2"), &date_sig()),
        formula_line(&view_quad(), &format!("{}(date: date) * 2", view_double()), &date_sig()),
        formula_line(&book_rate, &format!("{rp}(date: date) * n"), &book_sig()),
        cat_line(&view_double(), s1.clone()),
        cat_line(&view_quad(), s1.clone()),
    ];
    crate::crworld::publish(&lite, "views.jsonl", &views, BEFORE).unwrap_or_else(|e| panic!("views: {e}"));
    let h = fn_hash(&lite, &book_rate);
    let class = class_json(r#"{"n":{"type":"number","unit":"1"}}"#, &["n"], &[("rate", &h)], HOST, "");
    let c = vec![
        crate::common::pub031::class_line(&view_book(), &class, None),
        catalog_line_of(&view_book(), "class", json!({"facets": s1.clone()})),
    ];
    crate::crworld::publish(&lite, "book.jsonl", &c, NOW).unwrap_or_else(|e| panic!("book: {e}"));
    import_foreign(
        &lite,
        "data.ecb.europa.eu",
        &foreign_bundle("data.ecb.europa.eu", "data.ecb.europa.eu/api", &sub_domain(), Some(s1.clone())),
        T1,
    )
    .unwrap_or_else(|e| panic!("sub-domain: {e}"));
    import_foreign(
        &lite,
        "ecb.europa.eu",
        &foreign_bundle("ecb.europa.eu", "ecb.europa.eu/data", &own_domain(), Some(s1)),
        T2,
    )
    .unwrap_or_else(|e| panic!("own domain: {e}"));
    lite
}

// ---------------------------------------------------------------------------------------------------------------------
// World V: the vocabulary and the labels

pub fn v_entries() -> Vec<(String, Value)> {
    let f = |d: &str, t: &str, m: &str, r: &str, q: &str| fac(&[d], &[t], &[m], &[r], &[q]);
    vec![
        (host_name("relay/ecb.europa.eu/fx_reference"), f("finance", "currency", "exchange_rate", "ea", "daily")),
        (host_name("relay/ecb.europa.eu/estr"), f("finance", "rate", "interest_rate", "ea", "daily")),
        (host_name("relay/ecb.europa.eu/policy_rate"), f("finance", "rate", "interest_rate", "ea", "on_change")),
        (host_name("relay/ecb.europa.eu/yield_curve"), f("finance", "yield", "interest_rate", "ea", "daily")),
        (host_name("relay/bankofengland.co.uk/bank_rate"), f("finance", "rate", "interest_rate", "gb", "daily")),
        (host_name("relay/treasury.gov/par_yield"), f("finance", "yield", "yield", "us", "daily")),
        (host_name("earth/monthly_rainfall"), f("earth", "region", "rainfall", "world", "monthly")),
        (host_name("ratio/pe"), f("tools", "none", "ratio", "none", "static")),
    ]
}

/// World V, before the job.
pub fn vocab(label: &str) -> Lite {
    let lite = Lite::new(label);
    crate::crworld::publish(&lite, "v.jsonl", &entries_lines(&v_entries()), NOW)
        .unwrap_or_else(|e| panic!("world V: {e}"));
    lite
}

/// The subjects of world V: `(domain, thing, measure, region, label)`.
pub fn v_subjects() -> Vec<(&'static str, &'static str, &'static str, &'static str, &'static str)> {
    vec![
        ("finance", "currency", "exchange_rate", "ea", "exchange rate · currency · euro area · finance"),
        ("finance", "rate", "interest_rate", "ea", "interest rate · rate · euro area · finance"),
        ("finance", "yield", "interest_rate", "ea", "interest rate · yield · euro area · finance"),
        ("finance", "rate", "interest_rate", "gb", "interest rate · rate · United Kingdom · finance"),
        ("finance", "yield", "yield", "us", "yield · yield · United States · finance"),
        ("earth", "region", "rainfall", "world", "rainfall · region · world · earth"),
        ("tools", "none", "ratio", "none", "ratio · tools"),
    ]
}

// ---------------------------------------------------------------------------------------------------------------------
// World S: the scratch mesh of C6 (names under the mesh host, with the user's facets)

pub struct Subject {
    pub domain: &'static str,
    pub thing: &'static str,
    pub measure: &'static str,
    pub region: &'static str,
    /// Its sources in rank order.
    pub sources: Vec<String>,
}

pub fn s_names() -> Vec<(String, Value)> {
    let f = |d: &str, t: &str, m: &str, r: &str, q: &str| fac(&[d], &[t], &[m], &[r], &[q]);
    vec![
        (host_name("scripture/verse"), f("scripture", "verse", "text", "world", "static")),
        (host_name("scripture/edition"), f("scripture", "edition", "text", "world", "static")),
        (host_name("register/dataset"), f("registry", "dataset", "licence", "world", "on_change")),
        (host_name("relay/ecb.europa.eu/fx_reference"), f("finance", "currency", "exchange_rate", "ea", "daily")),
        (host_name("relay/ecb.europa.eu/estr"), f("finance", "rate", "interest_rate", "ea", "daily")),
        (host_name("relay/ecb.europa.eu/policy_rate"), f("finance", "rate", "interest_rate", "ea", "on_change")),
        (host_name("relay/bankofengland.co.uk/bank_rate"), f("finance", "rate", "interest_rate", "gb", "daily")),
        (host_name("relay/treasury.gov/par_yield"), f("finance", "yield", "yield", "us", "daily")),
        (host_name("earth/monthly_rainfall"), f("earth", "region", "rainfall", "world", "monthly")),
        (host_name("relay/sec.gov/filing"), f("finance", "filing", "fundamentals", "us", "annual")),
        (host_name("ratio/pe"), f("tools", "none", "ratio", "none", "static")),
    ]
}

pub fn sec_company_facets() -> Value {
    fac(&["finance"], &["company"], &["fundamentals"], &["us"], &["annual"])
}

/// The subjects of world S with their sources in rank order. `ratio/pe` was bound before `calendar/day`, so it ranks first.
pub fn s_subjects() -> Vec<Subject> {
    let n = |r: &str| host_name(r);
    let s = |domain, thing, measure, region, sources: Vec<String>| Subject { domain, thing, measure, region, sources };
    vec![
        s("scripture", "verse", "text", "world", vec![n("scripture/verse")]),
        s("scripture", "edition", "text", "world", vec![n("scripture/edition")]),
        s("registry", "dataset", "licence", "world", vec![n("register/dataset")]),
        s("finance", "currency", "exchange_rate", "ea", vec![n("relay/ecb.europa.eu/fx_reference")]),
        s(
            "finance",
            "rate",
            "interest_rate",
            "ea",
            vec![n("relay/ecb.europa.eu/estr"), n("relay/ecb.europa.eu/policy_rate")],
        ),
        s("finance", "rate", "interest_rate", "gb", vec![n("relay/bankofengland.co.uk/bank_rate")]),
        s("finance", "yield", "yield", "us", vec![n("relay/treasury.gov/par_yield")]),
        s("earth", "region", "rainfall", "world", vec![n("earth/monthly_rainfall")]),
        s("finance", "company", "fundamentals", "us", vec![n("sec/company")]),
        s("finance", "filing", "fundamentals", "us", vec![n("relay/sec.gov/filing")]),
        s("tools", "none", "ratio", "none", vec![n("ratio/pe"), n("calendar/day")]),
    ]
}

/// World S, before the job: the functions at `BEFORE`; at `NOW` the class `sec/company` (a class source), `calendar/day`
/// and the catalogue.
pub fn scratch(label: &str) -> Lite {
    let lite = Lite::new(label);
    let names = s_names();
    let mut a: Vec<String> = names.iter().map(|(n, _)| fn_line(n)).collect();
    a.extend(names.iter().filter(|(n, _)| n.contains("/relay/")).map(|(n, _)| relay_cell(n)));
    a.extend(filing_functions());
    crate::crworld::publish(&lite, "a.jsonl", &a, BEFORE).unwrap_or_else(|e| panic!("world S, functions: {e}"));
    let mut b = filing_classes(&lite);
    b.push(fn_line(&host_name("calendar/day")));
    b.extend(names.iter().map(|(n, f)| cat_line(n, f.clone())));
    b.push(catalog_line_of(&company(), "class", json!({"facets": sec_company_facets()})));
    b.push(cat_line(&host_name("calendar/day"), fac(&["tools"], &["none"], &["ratio"], &["none"], &["static"])));
    crate::crworld::publish(&lite, "b.jsonl", &b, NOW)
        .unwrap_or_else(|e| panic!("world S, classes and catalogue: {e}"));
    lite
}

/// The names of all the sources of world S.
pub fn s_all_sources() -> Vec<String> {
    let mut v: Vec<String> = s_subjects().into_iter().flat_map(|s| s.sources).collect();
    v.sort();
    v
}

// ---------------------------------------------------------------------------------------------------------------------
// Serving, asking

pub fn cfg(vars: &[(&str, &str)]) -> Config {
    let mut env: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    env.push(("DATUMESH_HOST".into(), HOST.into()));
    for k in ["DATUMESH_RATE_PER_IP", "DATUMESH_RATE_IP_BURST", "DATUMESH_RATE_SERIES_PER_IP"] {
        env.push((k.into(), "1000000".into()));
    }
    Config::from_vars(move |k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).unwrap()
}

pub async fn serve(lite: Lite, vars: &[(&str, &str)]) -> Served {
    build_served(lite, Some(cfg(vars)), |_| {}).await
}

/// A plain GET: status, headers, body.
pub struct Page {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: Vec<u8>,
}

impl Page {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn header(&self, k: &str) -> Option<String> {
        self.headers.get(k).and_then(|v| v.to_str().ok()).map(str::to_string)
    }
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.text()))
    }
}

pub async fn page(m: &Served, path: &str) -> Page {
    let r = m.http.get(format!("{}{path}", m.base)).send().await.unwrap();
    let status = r.status().as_u16();
    let headers = r.headers().clone();
    let body = r.bytes().await.unwrap().to_vec();
    Page { status, headers, body }
}

/// Waits until `path` answers something but 503 (the first build is done).
pub async fn until_built(m: &Served, path: &str) -> Page {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let r = page(m, path).await;
        if r.status != 503 || Instant::now() > end {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[track_caller]
pub fn assert_cold(r: &Page, what: &str) {
    assert_eq!(r.status, 503, "{what}: {}", r.text());
    let ra: u64 = r.header("retry-after").unwrap_or_else(|| panic!("{what}: no Retry-After")).parse().unwrap();
    assert!((1..=300).contains(&ra), "{what}: Retry-After {ra}");
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"), "{what}");
}

/// The cell a call denotes, read.
pub async fn cell(m: &Served, call: &str) -> Ans {
    ask(m, &path(call, None)).await
}

/// The value of an answer as a boolean (a receipt writes a boolean as JSON `true` or as its lexical text).
pub fn truth(a: &Ans) -> Option<bool> {
    match &a.v["receipt"]["value"] {
        Value::Bool(b) => Some(*b),
        Value::String(s) if s == "true" => Some(true),
        Value::String(s) if s == "false" => Some(false),
        _ => None,
    }
}

/// `/describe/<id>` (id percent-encoded as a path).
pub async fn describe(m: &Served, id: &str) -> Ans {
    ask(m, &format!("/describe/{}", call_path(id))).await
}

/// The value of `source(rank)` of a subject, `None` when it has no such cell.
pub async fn source_of(m: &Served, d: &str, t: &str, me: &str, r: &str, rank: u32) -> Option<String> {
    let a = cell(m, &sc(d, t, me, r, &format!("source({rank})"))).await;
    if a.outcome() == "ok" {
        a.value().map(str::to_string)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Reading pages

/// A page's text without its tags and with the common entities decoded, white space collapsed.
pub fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    let out = out
        .replace("&middot;", "\u{b7}")
        .replace("&#183;", "\u{b7}")
        .replace("&#xb7;", "\u{b7}")
        .replace("&#xB7;", "\u{b7}")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The `href` values of a page, in order, entities decoded.
pub fn hrefs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("href=") {
        rest = &rest[i + 5..];
        let q = rest.chars().next();
        let (open, close) = match q {
            Some('"') => ('"', '"'),
            Some('\'') => ('\'', '\''),
            _ => continue,
        };
        let body = &rest[open.len_utf8()..];
        if let Some(j) = body.find(close) {
            out.push(body[..j].replace("&amp;", "&"));
            rest = &body[j..];
        }
    }
    out
}

/// A link made root-relative: the mesh's own origin is dropped, anything else is left as it is.
pub fn local(href: &str) -> String {
    for p in [format!("https://{HOST}"), format!("http://{HOST}")] {
        if let Some(r) = href.strip_prefix(&p) {
            return if r.is_empty() { "/".to_string() } else { r.to_string() };
        }
    }
    href.to_string()
}

/// A path and query of a root-relative link, percent-decoded: `(segments, query pairs)`.
pub fn split_link(link: &str) -> (Vec<String>, Vec<(String, String)>) {
    let (p, q) = link.split_once('?').unwrap_or((link, ""));
    let segs = p.trim_start_matches('/').split('/').filter(|s| !s.is_empty()).map(unpct).collect();
    let pairs = q
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (unpct(k), unpct(v))
        })
        .collect();
    (segs, pairs)
}

pub fn unpct(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(x) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
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

/// The path of a subject's page.
pub fn subject_page(domain: &str, thing: &str, measure: &str, region: &str) -> String {
    format!("/catalog/{domain}/subject?key={}", qenc(&sk(domain, thing, measure, region)))
}

/// The words of a text.
pub fn words(t: &str) -> Vec<String> {
    t.split_whitespace().map(str::to_string).collect()
}

/// A snapshot-free count: the files of a world are written to `dir` of the lite store.
pub fn file_of(lite: &Lite, name: &str, lines: &[String]) -> std::path::PathBuf {
    write(lite, name, lines)
}
