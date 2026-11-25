//! Month and year pages of the relayed and computed series (Build 2 Part 6; CONTRACT-build2-part6-pages
//! G1 to G5; QUESTIONS-build2 D25 to D27).
//!
//! `GET /series/<function>/<YYYY-MM>` (a month of a daily series) and `/series/<function>/<YYYY>` (a year
//! of a monthly or event series), each with a `.csv` twin; `/series/<function>/` is the index of the
//! series, `/series/sitemap.xml` the sitemap. Which functions have pages, their granularity and their
//! columns come from the series file (`DATUMESH_SERIES_FILE`, a JSON array); without it there are no
//! pages, and a function it does not list has none.
//!
//! **No evaluation, no fetching.** Every page reads the store only: the binding of the function as of
//! now (`resolve_binding`), its descriptor (`inner.descriptor`), the members of a slice by a date range
//! (`slice_values`), the receipts of those members (`receipts_many`, for the licence and the relay
//! source), the catalogue (`catalog_summary`, `text_find`). A page of a function with no stored cell is
//! `404`, and nothing is computed, written or fetched from another mesh.
//!
//! The index and the sitemap need every period of a series. They read an overview of every listed series that
//! the mesh's background thread builds at start and then every `DATUMESH_SERIES_REFRESH_SECS` (`rebuild`); a
//! request reads the last one and never builds it. Before the first build they answer `503`. The previous and
//! next links of a period page come from the same overview, and are missing until the first build.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use datumesh_core::json::{self, Value};
use datumesh_core::{lexical, FunctionDescriptor, Receipt, Slice};
use datumesh_store::{SliceValue, Store};

use crate::config::ConfigError;
use crate::fnpage::{esc, relay_of, Relay};
use crate::mesh::Inner;

/// The most URLs in one sitemap file (the sitemap protocol's limit).
const SITEMAP_MAX: usize = 50_000;
/// Cache-Control of a period that is over: the edge keeps it a day.
const CACHE_PAST: &str = "public, max-age=300, s-maxage=86400";
/// Cache-Control of the current period, of an index, of the sitemap and of a 404.
const CACHE_NOW: &str = "public, max-age=300, s-maxage=600";
const HTML: &str = "text/html; charset=utf-8";
const CSV: &str = "text/csv; charset=utf-8";
const XML: &str = "application/xml; charset=utf-8";

/// The granularity of a series' pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Granularity {
    /// One page a month (a daily series).
    Month,
    /// One page a year (a monthly or an event series).
    Year,
}

/// One entry of the series file (G2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SeriesPage {
    pub(crate) function: String,
    pub(crate) granularity: Granularity,
    pub(crate) date_arg: String,
    pub(crate) column_arg: Option<String>,
    pub(crate) fixed: BTreeMap<String, String>,
    pub(crate) unit: String,
    /// `computed` (true) or `relayed` (false).
    pub(crate) computed: bool,
    /// The name of the source of a computed series, as its page says it (for example `CHIRPS v3`);
    /// read from the function's description when absent.
    pub(crate) source: Option<String>,
}

/// The series file: a JSON array of entries (see the `series` module documentation and QUESTIONS-build2 D27).
pub(crate) fn parse(text: &[u8]) -> Result<Vec<SeriesPage>, ConfigError> {
    const WHAT: &str = "DATUMESH_SERIES_FILE";
    let bad = |d: &str| ConfigError::new(WHAT, d.to_string());
    let v = json::parse(text).map_err(|e| bad(&e.to_string()))?;
    let Value::Array(items) = v else { return Err(bad("not a JSON array")) };
    let mut out = Vec::new();
    for it in &items {
        let s = |k: &str| it.get(k).and_then(Value::as_str).map(str::to_string);
        let function = s("function").ok_or_else(|| bad("an entry has no function"))?;
        lexical::check_function_name(&function, "$").map_err(|e| bad(&format!("{function}: {e}")))?;
        let granularity = match s("granularity").as_deref() {
            Some("month") => Granularity::Month,
            Some("year") => Granularity::Year,
            _ => return Err(bad(&format!("{function}: granularity is not month or year"))),
        };
        let date_arg =
            s("date_arg").filter(|x| !x.is_empty()).ok_or_else(|| bad(&format!("{function}: no date_arg")))?;
        let computed = match s("kind").as_deref() {
            Some("relayed") => false,
            Some("computed") => true,
            _ => return Err(bad(&format!("{function}: kind is not relayed or computed"))),
        };
        let mut fixed = BTreeMap::new();
        match it.get("fixed") {
            None => {}
            Some(Value::Object(m)) => {
                for (k, x) in m {
                    let x = x.as_str().ok_or_else(|| bad(&format!("{function}: fixed.{k} is not a string")))?;
                    fixed.insert(k.clone(), x.to_string());
                }
            }
            Some(_) => return Err(bad(&format!("{function}: fixed is not an object"))),
        }
        out.push(SeriesPage {
            function,
            granularity,
            date_arg,
            column_arg: s("column_arg").filter(|x| !x.is_empty()),
            fixed,
            unit: s("unit").unwrap_or_default(),
            computed,
            source: s("source").filter(|x| !x.is_empty()),
        });
    }
    Ok(out)
}

// ---- periods and dates ----------------------------------------------------------------------------

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_leap(y: u32) -> bool {
    y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400))
}

fn days_in(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

/// `YYYY-MM-DD` as year, month and day, when it is a date.
fn parse_date(s: &str) -> Option<(u32, u32, u32)> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' || !digits(&s[0..4]) || !digits(&s[5..7]) || !digits(&s[8..10]) {
        return None;
    }
    let (y, m, d): (u32, u32, u32) = (s[0..4].parse().ok()?, s[5..7].parse().ok()?, s[8..10].parse().ok()?);
    (1..=12).contains(&m).then_some(())?;
    (1..=days_in(y, m)).contains(&d).then_some((y, m, d))
}

/// The period a date falls in, of this granularity (`YYYY-MM` or `YYYY`).
fn period_of(g: Granularity, date: &str) -> Option<String> {
    match g {
        Granularity::Month => date.get(0..7).filter(|p| digits(&p[0..4]) && digits(&p[5..7])).map(str::to_string),
        Granularity::Year => date.get(0..4).filter(|p| digits(p)).map(str::to_string),
    }
}

/// A period as `(period, first day, last day)`, when the text is a period of this granularity.
fn parse_period(g: Granularity, s: &str) -> Option<(String, String, String)> {
    match g {
        Granularity::Month => {
            if s.len() != 7 || &s[4..5] != "-" || !digits(&s[0..4]) || !digits(&s[5..7]) {
                return None;
            }
            let (y, m): (u32, u32) = (s[0..4].parse().ok()?, s[5..7].parse().ok()?);
            if !(1..=12).contains(&m) {
                return None;
            }
            Some((s.to_string(), format!("{y:04}-{m:02}-01"), format!("{y:04}-{m:02}-{:02}", days_in(y, m))))
        }
        Granularity::Year => {
            if s.len() != 4 || !digits(s) {
                return None;
            }
            Some((s.to_string(), format!("{s}-01-01"), format!("{s}-12-31")))
        }
    }
}

/// The period of `now` (the current month or year).
fn current_of(g: Granularity, now: &str) -> String {
    period_of(g, now).unwrap_or_default()
}

fn cache_for(current: bool) -> String {
    if current {
        CACHE_NOW.to_string()
    } else {
        CACHE_PAST.to_string()
    }
}

/// The slice address of a series call for the page's interval (the redirect of `/slice`, G1): `None` when
/// the request is not a browser's, or the function is not listed, or the date is not a date. The query form
/// (`?date=...`) and the call form (`<fn>(date: ...)`, percent-encoded or not; D32) are both read.
pub(crate) fn slice_redirect(inner: &Inner, rest: &str, headers: &axum::http::HeaderMap) -> Option<String> {
    let config = &inner.config;
    if config.series.is_empty() || !led_by_html(headers) {
        return None;
    }
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    if query.split('&').any(|p| p.starts_with("format=")) {
        return None;
    }
    let (function, date) = if crate::call::is_call(rest) {
        // The crate's call parser: it reads the call as the slice endpoint does (aliases, unquoted values).
        let now = inner.now();
        let target = crate::call::resolve(rest, &now, &config.aliases, inner, true)
            .or_else(|_| crate::call::resolve(rest, &now, &config.aliases, inner, false))
            .ok()?;
        let function = target.fn_name();
        let page = config.series.iter().find(|p| p.function == function)?;
        let date = match target.cell.args.get(&page.date_arg)? {
            datumesh_core::Arg::Slice => "*".to_string(),
            a => a.scalar()?.lexical().to_string(),
        };
        (function, date)
    } else {
        let page = config.series.iter().find(|p| p.function == path)?;
        let key = format!("{}=", page.date_arg);
        let date = query.split('&').find_map(|p| p.strip_prefix(key.as_str()))?;
        (path.to_string(), date.to_string())
    };
    let page = config.series.iter().find(|p| p.function == function)?;
    let origin = config.origin();
    if date == "*" {
        return Some(format!("{origin}/series/{function}/"));
    }
    let (y, m, _) = parse_date(&date)?;
    let period = match page.granularity {
        Granularity::Month => format!("{y:04}-{m:02}"),
        Granularity::Year => format!("{y:04}"),
    };
    Some(format!("{origin}/series/{function}/{period}"))
}

/// Whether deciding the redirect of this request reads the store: a browser's call form on a mesh with a
/// series file (the call is resolved, which looks the name up). Such a decision runs on the pool, never on an
/// async thread (http 0.4.7: the PostgreSQL client panics there).
pub(crate) fn redirect_reads_store(inner: &Inner, rest: &str, headers: &axum::http::HeaderMap) -> bool {
    let has_format = rest.split_once('?').is_some_and(|(_, q)| q.split('&').any(|p| p.starts_with("format=")));
    !inner.config.series.is_empty() && !has_format && led_by_html(headers) && crate::call::is_call(rest)
}

/// Whether a request is answered by a page or a format of its own, so that `Accept` does not refuse it: a
/// browser on a listed function's slice, or a `format=` of the slice. A browser's call form is not decided
/// here (that reads the store): the slice handler decides it on the pool, and refuses it as 406 when it is
/// no redirect; see [`redirect_reads_store`].
pub(crate) fn negotiation_free(inner: &Inner, rest: &str, headers: &axum::http::HeaderMap) -> bool {
    let has_format = rest.split_once('?').is_some_and(|(_, q)| q.split('&').any(|p| p.starts_with("format=")));
    has_format || redirect_reads_store(inner, rest, headers) || slice_redirect(inner, rest, headers).is_some()
}

/// Whether the first media type of `Accept` is `text/html`.
fn led_by_html(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .is_some_and(|first| first.split(';').next().unwrap_or("").trim() == "text/html")
}

// ---- the store ----------------------------------------------------------------------------------

/// A series' function as bound now.
struct Bound {
    name: String,
    fn_hash: String,
    ns: String,
    fname: String,
    /// Argument name to type, as the signature declares them.
    types: BTreeMap<String, String>,
    d: Arc<FunctionDescriptor>,
}

fn bound(inner: &Inner, page: &SeriesPage, now: &str) -> Option<Bound> {
    let store: &dyn Store = &*inner.store;
    let b = store.resolve_binding(&page.function, now).ok()??;
    let d = inner.descriptor(&b.fn_hash).ok()?;
    let types: BTreeMap<String, String> =
        d.signature.args.iter().map(|(k, a)| (k.clone(), a.ty.as_str().to_string())).collect();
    if !types.contains_key(&page.date_arg) || page.fixed.keys().any(|k| !types.contains_key(k)) {
        return None;
    }
    if page.column_arg.as_ref().is_some_and(|c| !types.contains_key(c)) {
        return None;
    }
    let (ns, fname) = page.function.rsplit_once('/')?;
    Some(Bound {
        name: page.function.clone(),
        fn_hash: b.fn_hash,
        ns: ns.to_string(),
        fname: fname.to_string(),
        types,
        d,
    })
}

fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

fn obj(members: Vec<(&str, Value)>) -> Value {
    Value::Object(members.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// Which members a read takes: every member, or those whose date is in `[lo, hi]`.
enum Span<'a> {
    All,
    Days(&'a str, &'a str),
}

/// The slice of a series for a span: the date argument open or in the range, the fixed arguments bound,
/// every other argument open.
fn slice_of(b: &Bound, page: &SeriesPage, span: &Span) -> Option<Slice> {
    let star = || Value::String("*".into());
    let mut args = Vec::new();
    for (k, ty) in &b.types {
        let v = if *k == page.date_arg {
            match span {
                Span::All => star(),
                Span::Days(lo, hi) => obj(vec![("hi", s(hi)), ("lo", s(lo)), ("type", s("date"))]),
            }
        } else if let Some(x) = page.fixed.get(k) {
            obj(vec![("type", s(ty)), ("value", s(x))])
        } else {
            star()
        };
        args.push((k.clone(), v));
    }
    let cell =
        obj(vec![("args", Value::Object(args)), ("fn", s(&b.fname)), ("fn_hash", s(&b.fn_hash)), ("ns", s(&b.ns))]);
    Slice::from_value(&cell).ok()
}

/// At most `limit` members of a series in a span, as of now (stored versions only): the store stops there.
fn members(inner: &Inner, b: &Bound, page: &SeriesPage, span: Span, limit: usize) -> Option<Vec<SliceValue>> {
    let slice = slice_of(b, page, &span)?;
    let store: &dyn Store = &*inner.store;
    // The first `limit` members by key, one page (CONTRACT-0.5.1 H1).
    store.slice_page(&slice, &inner.now(), None, limit).ok()
}

fn key_of(m: &SliceValue, arg: &str) -> Option<String> {
    m.cell.args.get(arg).and_then(|a| a.scalar()).map(|x| x.lexical().to_string())
}

/// What an index or a sitemap needs of a series: its periods, and the latest instant of its cells.
#[derive(Default)]
pub(crate) struct Overview {
    latest: String,
    periods: BTreeMap<String, Period>,
    values: BTreeSet<String>,
}

pub(crate) struct Period {
    latest_tx: String,
    first_date: String,
    hash: String,
}

fn build_overview(inner: &Inner, b: &Bound, page: &SeriesPage) -> Option<Overview> {
    // The members are folded in pages of SERIES_PAGE by cell id, never held whole (CONTRACT-round-0.7-P HP7).
    let slice = slice_of(b, page, &Span::All)?;
    let store: &dyn Store = &*inner.store;
    let now = inner.now();
    let mut ov = Overview::default();
    let mut after: Option<String> = None;
    loop {
        let chunk = store.slice_values_page(&slice, &now, after.as_deref(), SERIES_PAGE).ok()?;
        let n = chunk.len();
        after = chunk.last().map(|m| m.version.cell_id.clone());
        fold_members(&mut ov, &chunk, page);
        if n < SERIES_PAGE {
            return Some(ov);
        }
    }
}

/// The members of a page of an overview's fold (see [`build_overview`]).
const SERIES_PAGE: usize = 10_000;

/// Folds members into an overview: their periods, first and latest instants, and the column values.
fn fold_members(ov: &mut Overview, members: &[SliceValue], page: &SeriesPage) {
    for m in members {
        let Some(date) = key_of(m, &page.date_arg) else { continue };
        let Some(period) = period_of(page.granularity, &date) else { continue };
        let tx = m.version.tx.clone();
        if tx > ov.latest {
            ov.latest = tx.clone();
        }
        if let Some(v) = page.column_arg.as_deref().and_then(|c| key_of(m, c)) {
            ov.values.insert(v);
        }
        match ov.periods.get_mut(&period) {
            Some(p) => {
                if tx > p.latest_tx {
                    p.latest_tx = tx;
                }
                if date < p.first_date {
                    p.first_date = date;
                }
            }
            None => {
                ov.periods
                    .insert(period, Period { latest_tx: tx, first_date: date, hash: m.version.receipt_hash.clone() });
            }
        }
    }
}

/// The values of a dictionary in their order, each with its value id.
type Dictionary = Arc<Vec<(String, i64)>>;

/// The overviews of every listed series and the sitemap made from them (G4, G5; http 0.4.8), and the
/// dictionary orders and entry texts the pages show (CONTRACT-0.4.9 H5): built by the background thread
/// (`rebuild`) and read by the requests, which never build one.
#[derive(Default)]
pub(crate) struct Built {
    overviews: HashMap<String, Arc<Overview>>,
    /// `sitemap.xml`: the urlset, or the sitemap index of the parts when there are more than one.
    root: String,
    /// `sitemap-<n>.xml` (from index 0 for part 1): the urlsets of the parts.
    parts: Vec<String>,
    /// The dictionary order of each keyed series' column argument, by [`order_key`].
    orders: HashMap<String, Dictionary>,
    /// The label and description of each listed series, by its function name.
    entries: HashMap<String, (String, String)>,
}

/// Remembered reads: the last build (the overviews, the orders, the entry texts).
#[derive(Default)]
pub(crate) struct Memo {
    built: Option<Arc<Built>>,
}

fn order_key(owner: &str, key: &str) -> String {
    format!("{owner}\n{key}")
}

/// The last built overviews, or `None` before the first build has completed.
fn built(inner: &Inner) -> Option<Arc<Built>> {
    inner.series.lock().unwrap_or_else(|p| p.into_inner()).built.clone()
}

/// The overview of a series as the last build made it (none before the first build, or for a series that had
/// no bound function or no cell then). A request never builds one.
fn overview(inner: &Inner, page: &SeriesPage) -> Option<Arc<Overview>> {
    built(inner)?.overviews.get(&page.function).cloned()
}

/// Builds the overviews of every listed series and the sitemap from them: the store's reads, on the calling
/// thread (the background thread of the mesh, never a request).
pub(crate) fn build(inner: &Inner) -> Built {
    let now = inner.now();
    let origin = inner.config.origin();
    let mut overviews = HashMap::new();
    let mut orders = HashMap::new();
    let mut entries = HashMap::new();
    let mut urls: Vec<(String, String)> = Vec::new();
    for page in &inner.config.series {
        let Some(b) = bound(inner, page, &now) else { continue };
        entries.insert(b.name.clone(), entry_text_now(&*inner.store, &b.name));
        if let Some(col) = &page.column_arg {
            orders.insert(order_key(&b.name, col), dictionary_order_now(&*inner.store, &b.name, col));
        }
        let Some(ov) = build_overview(inner, &b, page) else { continue };
        if !ov.periods.is_empty() {
            urls.push((format!("/series/{}/", b.name), ov.latest.clone()));
            for (p, info) in &ov.periods {
                urls.push((format!("/series/{}/{p}", b.name), info.latest_tx.clone()));
            }
        }
        overviews.insert(page.function.clone(), Arc::new(ov));
    }
    let (root, parts) = sitemap_bodies(&origin, &urls);
    Built { overviews, root, parts, orders, entries }
}

/// Builds the overview now and makes it the one requests read; counted as `by` (`background`).
pub(crate) fn rebuild(inner: &Inner, by: &'static str) {
    let built = Arc::new(build(inner));
    inner.series.lock().unwrap_or_else(|p| p.into_inner()).built = Some(built);
    inner.metrics.series_build(by);
}

/// The values of a dictionary in the order the publisher gave them (the store's value ids), each with its id, as
/// the last build has them (empty before it, or for a key the series does not name). A request never reads the
/// store for it (CONTRACT-0.4.9 H5).
fn dictionary_order(inner: &Inner, owner: &str, key: &str) -> Dictionary {
    built(inner).and_then(|b| b.orders.get(&order_key(owner, key)).cloned()).unwrap_or_default()
}

/// The dictionary order of `owner`'s `key` now: the store's catalogue and value ids (the overview thread's read).
fn dictionary_order_now(store: &dyn Store, owner: &str, key: &str) -> Dictionary {
    // The values in insertion order, one read of the dictionary (CONTRACT-round-0.7-P HP7): the position is the order.
    let values = store.dictionary_values_ordered(owner, key, DICTIONARY_VALUES).unwrap_or_default();
    Arc::new(values.into_iter().enumerate().map(|(i, v)| (v, i as i64)).collect())
}

/// The most values of a dictionary the overview reads.
const DICTIONARY_VALUES: usize = 100_000;

/// The columns of a keyed series: the dictionary's values in dictionary order, then any value the cells
/// have that the dictionary does not list (sorted).
fn columns_of(inner: &Inner, b: &Bound, col: &str, seen: &BTreeSet<String>) -> Vec<String> {
    let mut out: Vec<String> = dictionary_order(inner, &b.name, col).iter().map(|(v, _)| v.clone()).collect();
    for v in seen {
        if !out.contains(v) {
            out.push(v.clone());
        }
    }
    out
}

fn licences_of(inner: &Inner, hashes: &[String]) -> Vec<String> {
    let refs: Vec<&str> = hashes.iter().map(String::as_str).collect();
    let store: &dyn Store = &*inner.store;
    let mut set = BTreeSet::new();
    if let Ok(list) = store.receipts_many(&refs) {
        for bytes in list.into_iter().flatten() {
            if let Ok(r) = Receipt::from_json(&bytes) {
                let l = r.licence();
                if !l.is_empty() {
                    set.insert(l.to_string());
                }
            }
        }
    }
    set.into_iter().collect()
}

// ---- the views ----------------------------------------------------------------------------------

/// What a page shows; the same facts feed the HTML, the CSV and the JSON-LD.
struct View {
    origin: String,
    host: String,
    name: String,
    label: String,
    description: String,
    /// The period of a page; `None` for the index.
    period: Option<String>,
    path: String,
    unit: String,
    computed: bool,
    relay: Option<Relay>,
    /// The source a computed series says it is computed from (its name as the page shows it).
    computed_from: Option<String>,
    licences: Vec<String>,
    asof: String,
    prev: Option<String>,
    next: Option<String>,
    /// The table of a page: its header (`date`, then one per column) and its rows.
    headers: Vec<String>,
    rows: Vec<(String, Vec<String>)>,
    /// The index: each group (a year of months; for year pages one group with no label) and its periods.
    groups: Vec<(String, Vec<String>)>,
    /// The values of the keyed column (empty when the series has none).
    columns: Vec<String>,
    /// The fixed arguments, the column argument and the date argument, for the addresses of the slice.
    fixed: Vec<(String, String)>,
    column_arg: Option<String>,
    date_arg: String,
    /// A call for the agents: the key of the first column and a date the series has.
    example_key: Option<String>,
    example_date: String,
    /// The first and last day of the page's interval (a period, or the index's whole coverage).
    interval: (String, String),
    arg_types: BTreeMap<String, String>,
    coverage: String,
}

/// The slice address of the series: the fixed arguments, the column (a value or `*`) and the date (a value
/// or `*`), with `extra` appended (`&format=csv` and the like).
fn slice_url(v: &View, column: Option<&str>, date: &str, extra: &str) -> String {
    let mut q: Vec<String> = v.fixed.iter().map(|(k, x)| format!("{k}={x}")).collect();
    if let Some(c) = &v.column_arg {
        q.push(format!("{c}={}", column.unwrap_or("*")));
    }
    q.push(format!("{}={date}", v.date_arg));
    format!("{}/slice/{}?{}{extra}", v.origin, v.name, q.join("&"))
}

fn csv_of(headers: &[String], rows: &[(String, Vec<String>)]) -> String {
    use crate::slice::csv_field;
    let mut out = String::new();
    let line = |cells: Vec<String>| cells.iter().map(|c| csv_field(c)).collect::<Vec<_>>().join(",") + "\r\n";
    out.push_str(&line(headers.to_vec()));
    for (date, vals) in rows {
        let mut cells = vec![date.clone()];
        cells.extend(vals.iter().cloned());
        out.push_str(&line(cells));
    }
    out
}

/// The JSON of a value with `<`, `>` and `&` written as escapes, for a `<script>` element.
fn script_json(v: &Value) -> String {
    String::from_utf8_lossy(&v.to_jcs()).replace('<', "\\u003c").replace('>', "\\u003e").replace('&', "\\u0026")
}

fn dataset_ld(v: &View) -> Value {
    let (creator, source) = match (&v.relay, v.computed) {
        (Some(r), _) => (r.name.clone(), r.name.clone()),
        _ => (v.host.clone(), v.computed_from.clone().unwrap_or_else(|| v.host.clone())),
    };
    let name = match &v.period {
        Some(p) => format!("{} {p}", v.label),
        None => v.label.clone(),
    };
    let mut members: Vec<(&str, Value)> = vec![
        ("@context", s("https://schema.org")),
        ("@type", s("Dataset")),
        ("name", s(&name)),
        ("description", s(if v.description.is_empty() { &v.label } else { &v.description })),
        ("url", s(&format!("{}{}", v.origin, v.path))),
        ("creator", obj(vec![("@type", s("Organization")), ("name", s(&creator))])),
        ("sourceOrganization", obj(vec![("@type", s("Organization")), ("name", s(&source))])),
    ];
    match v.licences.len() {
        0 => {}
        1 => members.push(("license", s(&v.licences[0]))),
        _ => members.push(("license", Value::Array(v.licences.iter().map(|l| s(l)).collect()))),
    }
    members.push(("temporalCoverage", s(&v.coverage)));
    members.push(("dateModified", s(&v.asof)));
    let measured: Vec<Value> = if v.columns.is_empty() {
        vec![obj(vec![("@type", s("PropertyValue")), ("name", s("value")), ("unitText", s(&v.unit))])]
    } else {
        v.columns
            .iter()
            .map(|c| obj(vec![("@type", s("PropertyValue")), ("name", s(c)), ("unitText", s(&v.unit))]))
            .collect()
    };
    members.push(("variableMeasured", Value::Array(measured)));
    let mut dist = Vec::new();
    if v.period.is_some() {
        dist.push(obj(vec![
            ("@type", s("DataDownload")),
            ("encodingFormat", s("text/csv")),
            ("contentUrl", s(&format!("{}{}.csv", v.origin, v.path))),
        ]));
    }
    dist.push(obj(vec![
        ("@type", s("DataDownload")),
        ("encodingFormat", s("application/json")),
        ("contentUrl", s(&slice_url(v, None, "*", ""))),
    ]));
    members.push(("distribution", Value::Array(dist)));
    members.push(("isAccessibleForFree", Value::Bool(true)));
    obj(members)
}

const STYLE: &str = "body{font:16px/1.5 system-ui,sans-serif;margin:0;padding:16px;color:#1d1d1f;background:#fff;overflow-wrap:anywhere}main{max-width:60rem;margin:0 auto}table{border-collapse:collapse;display:block;overflow-x:auto}th,td{border:1px solid #d2d2d7;padding:2px 8px;text-align:right;white-space:nowrap}th:first-child,td:first-child{text-align:left}code,pre{font-family:ui-monospace,monospace;font-size:14px}pre{background:#f5f5f7;padding:12px;box-sizing:border-box;max-width:100%;overflow-x:auto;white-space:pre-wrap;overflow-wrap:anywhere}nav a{margin-right:1rem}section{margin:1.25rem 0}h1{font-size:1.5rem;margin:0.5rem 0}h2{font-size:1.1rem}";

/// The HTML of a view (no script but the JSON-LD; every text escaped).
fn render(v: &View) -> String {
    let title = match &v.period {
        Some(p) => format!("{} {p} | {}", v.label, v.name),
        None => format!("{} | {}", v.label, v.name),
    };
    let mut h = String::new();
    h.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    h.push_str(&format!("<title>{}</title>\n", esc(&title)));
    h.push_str(&format!("<style>{STYLE}</style>\n"));
    h.push_str(&format!("<script type=\"application/ld+json\">{}</script>\n", script_json(&dataset_ld(v))));
    h.push_str("</head>\n<body>\n<main>\n");

    // navigation
    h.push_str("<nav>\n");
    if let Some(p) = &v.prev {
        h.push_str(&format!(
            "<a rel=\"prev\" href=\"{}\">&larr; {}</a>\n",
            esc(&format!("/series/{}/{p}", v.name)),
            esc(p)
        ));
    }
    if let Some(n) = &v.next {
        h.push_str(&format!(
            "<a rel=\"next\" href=\"{}\">{} &rarr;</a>\n",
            esc(&format!("/series/{}/{n}", v.name)),
            esc(n)
        ));
    }
    h.push_str(&format!("<a href=\"/series/{}/\">Index</a>\n", esc(&v.name)));
    h.push_str(&format!("<a href=\"/fn/{}\">Function page</a>\n", esc(&v.name)));
    h.push_str("</nav>\n");

    // heading and text
    match &v.period {
        Some(p) => h.push_str(&format!("<h1>{} <small>{}</small></h1>\n", esc(&v.label), esc(p))),
        None => h.push_str(&format!("<h1>{}</h1>\n", esc(&v.label))),
    }
    if !v.description.is_empty() {
        h.push_str(&format!("<p>{}</p>\n", esc(&v.description)));
    }
    match (&v.relay, v.computed) {
        (Some(r), _) => {
            let take = format!("/takeover/{}", r.name);
            h.push_str(&format!(
                "<p>{} relays these values unchanged from {}. {} can publish them under its own name: <a href=\"{}\">{}</a>.</p>\n",
                esc(&v.host),
                esc(&r.name),
                esc(&r.name),
                esc(&take),
                esc(&take)
            ));
        }
        (None, true) => {
            let text = match &v.computed_from {
                Some(src) => {
                    let owner = src.split_whitespace().next().unwrap_or(src);
                    format!("Computed by {} from {src}; not {owner}'s own values.", v.host)
                }
                None => {
                    format!("Computed by {} from the inputs named in its description; not their own values.", v.host)
                }
            };
            h.push_str(&format!("<p>{}</p>\n", esc(&text)));
        }
        (None, false) => h.push_str("<p>These are relayed values; their receipts do not name the source.</p>\n"),
    }
    if v.licences.is_empty() {
        h.push_str("<p>Licence: none stated on these cells.</p>\n");
    } else {
        h.push_str(&format!("<p>Licence: {}.</p>\n", esc(&v.licences.join(", "))));
    }
    h.push_str(&format!("<p>As of {}: the latest signed value on this page.</p>\n", esc(&v.asof)));

    // the CSV twin
    if let Some(p) = &v.period {
        let csv = format!("{}{}.csv", v.origin, v.path);
        h.push_str(&format!(
            "<p>CSV: <a href=\"{}.csv\">{}.csv</a>. Excel (Data &gt; From Web) and Google Sheets (<code>=IMPORTDATA(&quot;{}&quot;)</code>) can import this CSV from its address.</p>\n",
            esc(&format!("/series/{}/{p}", v.name)),
            esc(p),
            esc(&csv)
        ));
        // the table
        h.push_str("<table>\n<thead><tr>");
        for c in &v.headers {
            h.push_str(&format!("<th>{}</th>", esc(c)));
        }
        h.push_str("</tr></thead>\n<tbody>\n");
        for (date, vals) in &v.rows {
            h.push_str(&format!("<tr><td>{}</td>", esc(date)));
            for x in vals {
                h.push_str(&format!("<td>{}</td>", esc(x)));
            }
            h.push_str("</tr>\n");
        }
        h.push_str("</tbody>\n</table>\n");
    } else {
        for (group, periods) in &v.groups {
            if !group.is_empty() {
                h.push_str(&format!("<section>\n<h2>{}</h2>\n", esc(group)));
            } else {
                h.push_str("<section>\n");
            }
            h.push_str("<ul>\n");
            for p in periods {
                h.push_str(&format!(
                    "<li><a href=\"{}\">{}</a></li>\n",
                    esc(&format!("/series/{}/{p}", v.name)),
                    esc(p)
                ));
            }
            h.push_str("</ul>\n</section>\n");
        }
    }

    // for agents
    let key = v.example_key.clone();
    let date = v.example_date.clone();
    let mut call_args: Vec<String> = v
        .fixed
        .iter()
        .map(|(k, x)| format!("{k}: {}", call_value(v.arg_types.get(k).map(String::as_str), x)))
        .collect();
    if let (Some(c), Some(k)) = (&v.column_arg, &key) {
        call_args.push(format!("{c}: {}", call_value(v.arg_types.get(c).map(String::as_str), k)));
    }
    call_args.push(format!("{}: {}", v.date_arg, call_value(Some("date"), &date)));
    let call = format!("{}({})", v.name, call_args.join(", "));
    let mcp = mcp_request(&call);
    let key_part = match (&v.column_arg, &key) {
        (Some(c), Some(k)) => format!("{c}={k}&"),
        _ => String::new(),
    };
    let fixed_part: String = v.fixed.iter().map(|(k, x)| format!("{k}={x}&")).collect();
    let base = format!("{}/slice/{}?{fixed_part}{key_part}{}", v.origin, v.name, v.date_arg);
    // The interval call (D32): the page's interval, for any interval change `from` and `to`.
    let (from, to) = v.interval.clone();
    let ibase = format!("{}/x/interval/{}?{key_part}from={from}&to={to}", v.origin, v.name);
    let mut pre = String::new();
    pre.push_str(&format!("JSON   {base}=*           every date (one day: {}={date})\n", v.date_arg));
    pre.push_str(&format!("CSV    {base}=*&format=csv\n"));
    pre.push_str(&format!("Arrow  {base}=*&format=arrow\n"));
    pre.push_str(&format!("MCP    get {mcp}\n"));
    pre.push_str("Any interval: /x/interval takes any from and to (a keyed series: key=* for every key).\n");
    pre.push_str(&format!("JSON   {ibase}\n"));
    pre.push_str(&format!("CSV    {ibase}&format=csv\n"));
    pre.push_str(&format!("Arrow  {ibase}&format=arrow\n"));
    h.push_str(&format!(
        "<section id=\"agents\">\n<h2>For agents</h2>\n<pre>{}</pre>\n</section>\n",
        esc(pre.trim_end())
    ));

    // verify
    let verify = slice_url(v, None, "*", "");
    h.push_str(&format!(
        "<section id=\"verify\">\n<p>Every value is a signed receipt. The <a href=\"{}\">slice of this series</a> lists them; check them with <code>datumesh verify</code> (the command line) or <code>@datumesh/verify</code> (JavaScript).</p>\n</section>\n",
        esc(&verify.replacen(&v.origin, "", 1))
    ));
    h.push_str("</main>\n</body>\n</html>\n");
    h
}

/// The MCP form of a call: `{"request":"<call>"}`, the request of the `get` tool.
fn mcp_request(call: &str) -> String {
    let mut out = String::from("{\"request\":\"");
    for c in call.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push_str("\"}");
    out
}

/// A value as a call writes it: a date bare, a string quoted.
fn call_value(ty: Option<&str>, x: &str) -> String {
    match ty {
        Some("date") | Some("number") | Some("boolean") => x.to_string(),
        _ => format!("\"{x}\""),
    }
}

// ---- responses ----------------------------------------------------------------------------------

/// What a route answers: the status, the body, its media type and cache, and an `ETag` when it has one.
pub(crate) struct Out {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
    pub(crate) content_type: &'static str,
    pub(crate) cache: String,
    pub(crate) etag: Option<String>,
    /// `Retry-After` in seconds, on a `503` (the overview is not built yet).
    pub(crate) retry_after: Option<u64>,
}

fn not_found() -> Out {
    Out {
        status: 404,
        body: b"<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>not found</title></head><body><h1>not found</h1><p>No stored value is published at this address.</p></body></html>\n".to_vec(),
        content_type: HTML,
        cache: CACHE_NOW.to_string(),
        etag: None,
        retry_after: None,
    }
}

fn etag_of(parts: &[&str]) -> String {
    let h = datumesh_core::sha256_hex(parts.join("\n").as_bytes());
    format!("\"{}\"", &h[..32])
}

fn not_modified(inm: Option<&str>, etag: &str) -> bool {
    inm.is_some_and(|v| {
        v.split(',').map(str::trim).any(|t| t == "*" || t == etag || t.strip_prefix("W/") == Some(etag))
    })
}

fn view_base(inner: &Inner, b: &Bound, page: &SeriesPage, label: String, description: String) -> View {
    let relay = if page.computed { None } else { relay_of(inner, &b.name, &b.fn_hash) };
    let computed_from = if page.computed {
        page.source.clone().or_else(|| {
            let i = description.find(" from ")?;
            let rest = &description[i + 6..];
            let end = rest.find(['.', ',', ';']).unwrap_or(rest.len());
            let src = rest[..end].trim();
            (!src.is_empty()).then(|| src.to_string())
        })
    } else {
        None
    };
    let types = b.types.clone();
    View {
        origin: inner.config.origin(),
        host: inner.config.host.clone(),
        name: b.name.clone(),
        label,
        description,
        period: None,
        path: format!("/series/{}/", b.name),
        unit: page.unit.clone(),
        computed: page.computed,
        relay,
        computed_from,
        licences: Vec::new(),
        asof: String::new(),
        prev: None,
        next: None,
        headers: Vec::new(),
        rows: Vec::new(),
        groups: Vec::new(),
        columns: Vec::new(),
        fixed: page.fixed.iter().map(|(k, x)| (k.clone(), x.clone())).collect(),
        column_arg: page.column_arg.clone(),
        date_arg: page.date_arg.clone(),
        example_key: None,
        example_date: String::new(),
        interval: (String::new(), String::new()),
        arg_types: types,
        coverage: String::new(),
    }
}

/// The label and description of a series' function, as the last build has them (CONTRACT-0.4.9 H5).
fn entry_text(inner: &Inner, name: &str) -> (String, String) {
    built(inner).and_then(|b| b.entries.get(name).cloned()).unwrap_or_else(|| (name.to_string(), String::new()))
}

/// The label and description of a function now: the overview thread's read.
fn entry_text_now(store: &dyn Store, name: &str) -> (String, String) {
    match crate::find::entry_by_name(store, name) {
        Some(e) => {
            let label = if e.label.is_empty() { name.to_string() } else { e.label };
            (label, e.description)
        }
        None => (name.to_string(), String::new()),
    }
}

/// The page of one period (HTML, or its CSV twin).
fn period_page(
    inner: &Inner,
    page: &SeriesPage,
    b: &Bound,
    period: &str,
    days: (&str, &str),
    csv: bool,
    inm: Option<&str>,
) -> Out {
    let (lo, _) = days;
    // At most `slice_max` members, read as `/slice` reads them: `slice_max + 1` to know it is cut
    // (CONTRACT-round-0.7-P HP8).
    let cap = inner.config.slice_max;
    let Some(mut members) = members(inner, b, page, Span::Days(days.0, days.1), cap + 1) else { return not_found() };
    members.truncate(cap);
    if members.is_empty() {
        return not_found();
    }
    let (label, description) = entry_text(inner, &b.name);
    let mut v = view_base(inner, b, page, label, description);
    v.path = format!("/series/{}/{period}", b.name);
    v.period = Some(period.to_string());
    v.coverage = period.to_string();
    v.interval = (days.0.to_string(), days.1.to_string());

    // the columns: the dictionary's (keyed) or one value column
    let seen: BTreeSet<String> =
        page.column_arg.as_deref().map(|c| members.iter().filter_map(|m| key_of(m, c)).collect()).unwrap_or_default();
    let columns: Vec<String> = match page.column_arg.as_deref() {
        Some(c) => columns_of(inner, b, c, &seen),
        None => vec!["value".to_string()],
    };
    let unit_of = |x: &str| if page.unit.is_empty() { x.to_string() } else { format!("{x} ({})", page.unit) };
    v.headers = std::iter::once("date".to_string()).chain(columns.iter().map(|c| unit_of(c))).collect();
    v.columns = if page.column_arg.is_some() { columns.clone() } else { Vec::new() };
    v.example_key = columns.first().cloned().filter(|_| page.column_arg.is_some());

    // the table: one row a date with a cell, the latest version of each column
    let index: HashMap<&str, usize> = columns.iter().enumerate().map(|(i, c)| (c.as_str(), i)).collect();
    let mut rows: BTreeMap<String, Vec<Option<(String, String)>>> = BTreeMap::new();
    let mut latest = String::new();
    for m in &members {
        if m.version.tx > latest {
            latest = m.version.tx.clone();
        }
        let Some(date) = key_of(m, &page.date_arg) else { continue };
        let col = match page.column_arg.as_deref() {
            Some(c) => match key_of(m, c).and_then(|k| index.get(k.as_str()).copied()) {
                Some(i) => i,
                None => continue,
            },
            None => 0,
        };
        let cells = rows.entry(date).or_insert_with(|| vec![None; columns.len()]);
        let newer = match &cells[col] {
            Some((tx, _)) => m.version.tx.as_str() > tx.as_str(),
            None => true,
        };
        if newer {
            cells[col] = Some((m.version.tx.clone(), m.value.lexical().to_string()));
        }
    }
    v.rows = rows
        .into_iter()
        .map(|(d, cells)| (d, cells.into_iter().map(|c| c.map(|(_, x)| x).unwrap_or_default()).collect()))
        .collect();
    v.example_date = v.rows.first().map(|r| r.0.clone()).unwrap_or_else(|| lo.to_string());
    v.asof = latest.clone();

    let hashes: Vec<String> = members.iter().map(|m| m.version.receipt_hash.clone()).collect();
    let (prev, next) = match overview(inner, page) {
        Some(ov) => (
            ov.periods.keys().rfind(|k| k.as_str() < period).cloned(),
            ov.periods.keys().find(|k| k.as_str() > period).cloned(),
        ),
        None => (None, None),
    };
    v.prev = prev;
    v.next = next;
    let now = inner.now();
    let current = period == current_of(page.granularity, &now);

    let mut parts: Vec<&str> = vec![if csv { "csv" } else { "html" }, period];
    let (pv, nx) = (v.prev.clone().unwrap_or_default(), v.next.clone().unwrap_or_default());
    parts.push(&pv);
    parts.push(&nx);
    let refs: Vec<&str> = hashes.iter().map(String::as_str).collect();
    parts.extend(refs);
    let etag = etag_of(&parts);
    if not_modified(inm, &etag) {
        return Out {
            status: 304,
            body: Vec::new(),
            content_type: if csv { CSV } else { HTML },
            cache: cache_for(current),
            etag: Some(etag),
            retry_after: None,
        };
    }
    // The licences are read from the receipts only when an HTML body is rendered (not for a 304 or a CSV).
    if !csv {
        v.licences = licences_of(inner, &hashes);
    }
    let (body, content_type) =
        if csv { (csv_of(&v.headers, &v.rows).into_bytes(), CSV) } else { (render(&v).into_bytes(), HTML) };
    Out { status: 200, body, content_type, cache: cache_for(current), etag: Some(etag), retry_after: None }
}

/// The index of a series: its periods, grouped by year for a monthly granularity.
fn index_page(inner: &Inner, page: &SeriesPage, b: &Bound, inm: Option<&str>) -> Out {
    let Some(ov) = overview(inner, page) else { return not_found() };
    if ov.periods.is_empty() {
        return not_found();
    }
    let (label, description) = entry_text(inner, &b.name);
    let mut v = view_base(inner, b, page, label, description);
    v.asof = ov.latest.clone();
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for p in ov.periods.keys() {
        let group = match page.granularity {
            Granularity::Month => p[0..4].to_string(),
            Granularity::Year => String::new(),
        };
        match groups.last_mut() {
            Some((g, ps)) if *g == group => ps.push(p.clone()),
            _ => groups.push((group, vec![p.clone()])),
        }
    }
    v.groups = groups;
    let first = ov.periods.keys().next().cloned().unwrap_or_default();
    let last = ov.periods.keys().next_back().cloned().unwrap_or_default();
    if let (Some((_, lo, _)), Some((_, _, hi))) =
        (parse_period(page.granularity, &first), parse_period(page.granularity, &last))
    {
        v.interval = (lo, hi);
    }
    v.coverage = if first == last { first } else { format!("{first}/{last}") };
    let seen = ov.values.clone();
    v.columns = match page.column_arg.as_deref() {
        Some(c) => columns_of(inner, b, c, &seen),
        None => Vec::new(),
    };
    v.example_key = v.columns.first().cloned().filter(|_| page.column_arg.is_some());
    v.example_date = ov.periods.values().next().map(|p| p.first_date.clone()).unwrap_or_default();
    let reps: Vec<String> = ov.periods.values().map(|p| p.hash.clone()).collect();
    v.licences = licences_of(inner, &reps);
    let keys: Vec<String> = ov.periods.iter().flat_map(|(k, p)| [k.clone(), p.latest_tx.clone()]).collect();
    let mut parts: Vec<&str> = vec!["index", &ov.latest];
    parts.extend(keys.iter().map(String::as_str));
    parts.extend(reps.iter().map(String::as_str));
    let etag = etag_of(&parts);
    if not_modified(inm, &etag) {
        return Out {
            status: 304,
            body: Vec::new(),
            content_type: HTML,
            cache: CACHE_NOW.to_string(),
            etag: Some(etag),
            retry_after: None,
        };
    }
    let body = render(&v).into_bytes();
    Out { status: 200, body, content_type: HTML, cache: CACHE_NOW.to_string(), etag: Some(etag), retry_after: None }
}

/// The sitemap part `n` (`sitemap-<n>.xml`), or `None` for `sitemap.xml` itself.
fn sitemap_part(rest: &str) -> Option<Option<usize>> {
    if rest == "sitemap.xml" {
        return Some(None);
    }
    let n = rest.strip_prefix("sitemap-")?.strip_suffix(".xml")?;
    n.parse::<usize>().ok().filter(|k| *k >= 1).map(Some)
}

/// The sitemap of `urls` (`(path, lastmod)`): the root (a urlset, or a sitemap index of the parts when there are
/// more than [`SITEMAP_MAX`]) and the parts, each a urlset of at most [`SITEMAP_MAX`] URLs.
fn sitemap_bodies(origin: &str, urls: &[(String, String)]) -> (String, Vec<String>) {
    let urlset = |chunk: &[(String, String)]| {
        let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
        for (path, last) in chunk {
            x.push_str(&format!(
                "<url><loc>{}</loc><lastmod>{}</lastmod></url>\n",
                esc(&format!("{origin}{path}")),
                esc(last)
            ));
        }
        x.push_str("</urlset>\n");
        x
    };
    let parts: Vec<String> =
        if urls.is_empty() { vec![urlset(&[])] } else { urls.chunks(SITEMAP_MAX).map(urlset).collect() };
    if parts.len() == 1 {
        return (parts[0].clone(), parts);
    }
    let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    for (i, chunk) in urls.chunks(SITEMAP_MAX).enumerate() {
        let last = chunk.iter().map(|c| c.1.as_str()).max().unwrap_or("");
        x.push_str(&format!(
            "<sitemap><loc>{}</loc><lastmod>{}</lastmod></sitemap>\n",
            esc(&format!("{origin}/series/sitemap-{}.xml", i + 1)),
            esc(last)
        ));
    }
    x.push_str("</sitemapindex>\n");
    (x, parts)
}

/// `503` for the sitemap and the index before the first build: `Retry-After` the refresh interval (1 to 300 s),
/// never cached.
fn unavailable(inner: &Inner) -> Out {
    Out {
        status: 503,
        body: b"<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>not yet</title></head><body><h1>not yet</h1><p>The series overview is being built. Try again shortly.</p></body></html>\n".to_vec(),
        content_type: HTML,
        cache: "no-store".to_string(),
        etag: None,
        retry_after: Some(inner.config.series_refresh.as_secs().clamp(1, 300)),
    }
}

/// `/series/sitemap.xml`: every index and page of every listed series, `lastmod` the as-of, read from the last
/// build (`503` before it). Over 50,000 URLs it is a sitemap index of `sitemap-<n>.xml` files of 50,000.
fn sitemap(inner: &Inner, part: Option<usize>) -> Out {
    if inner.config.series.is_empty() {
        return not_found();
    }
    let Some(b) = built(inner) else { return unavailable(inner) };
    let body = match part {
        None => b.root.clone(),
        Some(k) => match b.parts.get(k - 1) {
            Some(p) => p.clone(),
            None => return not_found(),
        },
    };
    Out {
        status: 200,
        body: body.into_bytes(),
        content_type: XML,
        cache: CACHE_NOW.to_string(),
        etag: None,
        retry_after: None,
    }
}

/// The answer to `GET /series/<rest>`: a period's page or its CSV, the index, or the sitemap. Every answer is
/// read from the store; an unknown function, period or empty period is `404`.
pub(crate) fn respond(inner: &Inner, rest: &str, inm: Option<&str>) -> Out {
    if let Some(part) = sitemap_part(rest) {
        return sitemap(inner, part);
    }
    let Some((fname, last)) = rest.rsplit_once('/') else { return not_found() };
    let Some(page) = inner.config.series.iter().find(|p| p.function == fname) else { return not_found() };
    let now = inner.now();
    let Some(b) = bound(inner, page, &now) else { return not_found() };
    if last.is_empty() {
        if built(inner).is_none() {
            return unavailable(inner);
        }
        return index_page(inner, page, &b, inm);
    }
    let (stem, csv) = match last.strip_suffix(".csv") {
        Some(x) => (x, true),
        None => (last, false),
    };
    let Some((period, lo, hi)) = parse_period(page.granularity, stem) else { return not_found() };
    period_page(inner, page, &b, &period, (&lo, &hi), csv, inm)
}

// ---- the interval call (D32) ---------------------------------------------------------------------------

/// The longest interval of a series with month pages, in years.
const INTERVAL_YEARS: u32 = 10;
/// The longest interval of a series with year pages, in years (CONTRACT-0.4.9 H5).
const INTERVAL_YEARS_YEAR: u32 = 50;
const JSON_TYPE: &str = "application/json";
const ARROW_TYPE: &str = crate::slice::ARROW;

fn interval_error(status: u16, outcome: &str, code: Option<&str>, cache: &str) -> Out {
    let mut m = vec![("outcome".to_string(), s(outcome))];
    if let Some(c) = code {
        m.push(("error".to_string(), s(c)));
    }
    Out {
        status,
        body: Value::Object(m).to_jcs(),
        content_type: JSON_TYPE,
        cache: cache.to_string(),
        etag: None,
        retry_after: None,
    }
}

fn interval_bad(code: &str) -> Out {
    interval_error(400, "bad_request", Some(code), "no-store")
}

/// The most days a date may be after `from` for a series with month pages: `from` plus ten years.
fn plus_years(date: &str, years: u32) -> String {
    let y: u32 = date[0..4].parse().unwrap_or(0);
    format!("{:04}{}", y + years, &date[4..])
}

/// `GET /x/interval/<function>?...`: the stored cells of a listed series with a date in `[from, to]`, keys bound
/// (`key=value`) or open (`key=*`, or left out), each with its receipt; JSON, CSV or Arrow. Read from the store
/// only, as the pages are.
pub(crate) fn interval(inner: &Inner, function: &str, query: &str, inm: Option<&str>) -> Out {
    let missing = || interval_error(404, "not_covered", None, CACHE_NOW);
    let Some(page) = inner.config.series.iter().find(|p| p.function == function) else { return missing() };
    let now = inner.now();
    let Some(b) = bound(inner, page, &now) else { return missing() };

    let mut params: BTreeMap<String, String> = BTreeMap::new();
    for part in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        let (Some(k), Some(v)) = (crate::call::url_decode(k), crate::call::url_decode(v)) else {
            return interval_bad("E_PARSE_URI");
        };
        if params.insert(k, v).is_some() {
            return interval_bad("E_PARSE_URI");
        }
    }
    let (Some(from), Some(to)) = (params.remove("from"), params.remove("to")) else {
        return interval_bad("E_PARSE_URI");
    };
    if parse_date(&from).is_none() || parse_date(&to).is_none() || from > to {
        return interval_bad("E_PARSE_URI");
    }
    let longest = match page.granularity {
        Granularity::Month => INTERVAL_YEARS,
        Granularity::Year => INTERVAL_YEARS_YEAR,
    };
    if to > plus_years(&from, longest) {
        return interval_bad("E_RANGE_TOO_LARGE");
    }
    let form = match params.remove("format").as_deref() {
        None | Some("json") => "json",
        Some("csv") => "csv",
        Some("arrow") => "arrow",
        Some(_) => return interval_bad("E_PARSE_URI"),
    };
    // The keys: bound ones join the series' fixed arguments for this read; `*` leaves a key open.
    let mut p2 = page.clone();
    for (k, v) in &params {
        if *k == page.date_arg || !b.types.contains_key(k) {
            return interval_bad("E_PARSE_URI");
        }
        if let Some(fixed) = page.fixed.get(k) {
            if v != fixed {
                return interval_bad("E_PARSE_URI");
            }
        } else if v != "*" {
            p2.fixed.insert(k.clone(), v.clone());
        }
    }
    let Some(slice) = slice_of(&b, &p2, &Span::Days(&from, &to)) else { return interval_bad("E_PARSE_URI") };
    let store: &dyn Store = &*inner.store;
    // At most `slice_max + 1` members are read: one more than the answer holds, to know it is cut (H5).
    let Ok(mut members) = store.slice_page(&slice, &now, None, inner.config.slice_max + 1) else {
        return interval_error(503, "error", Some("E_EVAL_UNAVAILABLE"), "no-store");
    };
    // The open arguments, the date first, in the order of the signature after it.
    let open: Vec<String> = std::iter::once(page.date_arg.clone())
        .chain(b.d.signature.args.keys().filter(|k| **k != page.date_arg && !p2.fixed.contains_key(*k)).cloned())
        .collect();
    let lex = |m: &SliceValue, k: &str| key_of(m, k).unwrap_or_default();
    members.sort_by(|a, c| {
        let ka: Vec<String> = open.iter().map(|k| lex(a, k)).collect();
        let kc: Vec<String> = open.iter().map(|k| lex(c, k)).collect();
        ka.cmp(&kc).then_with(|| a.version.tx.cmp(&c.version.tx))
    });
    let truncated = members.len() > inner.config.slice_max;
    members.truncate(inner.config.slice_max);

    let current = match (period_of(page.granularity, &to), period_of(page.granularity, &now)) {
        (Some(t), Some(n)) => t >= n,
        _ => true,
    };
    let cache = cache_for(current);
    let mut parts: Vec<String> = vec![form.into(), function.into(), from.clone(), to.clone(), truncated.to_string()];
    parts.extend(params.iter().map(|(k, v)| format!("{k}={v}")));
    parts.extend(members.iter().map(|m| m.version.receipt_hash.clone()));
    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    let etag = etag_of(&refs);
    let (content_type, ok): (&'static str, bool) = match form {
        "csv" => (CSV, true),
        "arrow" => (ARROW_TYPE, true),
        _ => (JSON_TYPE, true),
    };
    debug_assert!(ok);
    if not_modified(inm, &etag) {
        return Out { status: 304, body: Vec::new(), content_type, cache, etag: Some(etag), retry_after: None };
    }
    let body = match form {
        "arrow" => match crate::slice::arrow_stream(&members, &open, &slice, &now, function) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(target: "datumesh_http", error = %e, "arrow");
                return interval_error(503, "error", Some("E_EVAL_UNAVAILABLE"), "no-store");
            }
        },
        "csv" => interval_csv(inner, &b, page, &open, &members).into_bytes(),
        _ => interval_json(inner, function, (&from, &to), &now, &members, truncated),
    };
    Out { status: 200, body, content_type, cache, etag: Some(etag), retry_after: None }
}

fn interval_json(
    inner: &Inner,
    function: &str,
    range: (&str, &str),
    now: &str,
    members: &[SliceValue],
    truncated: bool,
) -> Vec<u8> {
    let hashes: Vec<&str> = members.iter().map(|m| m.version.receipt_hash.as_str()).collect();
    let receipts = inner.store.receipts_many(&hashes).unwrap_or_default();
    let mut out = Vec::with_capacity(members.len());
    for (i, m) in members.iter().enumerate() {
        let keys: Vec<(String, Value)> =
            m.cell.args.iter().filter_map(|(k, a)| Some((k.clone(), s(a.scalar()?.lexical())))).collect();
        let receipt =
            receipts.get(i).and_then(|r| r.as_deref()).and_then(|b| json::parse(b).ok()).unwrap_or(Value::Null);
        out.push(obj(vec![
            ("asof", s(&m.version.asof)),
            ("keys", Value::Object(keys)),
            ("receipt", receipt),
            ("receipt_hash", s(&m.version.receipt_hash)),
            ("tx", s(&m.version.tx)),
            ("type", s(m.value.ty().as_str())),
            ("value", s(m.value.lexical())),
        ]));
    }
    let count = json::parse(members.len().to_string().as_bytes()).unwrap_or(Value::Null);
    let mut top = vec![
        ("asof", s(now)),
        ("count", count),
        ("from", s(range.0)),
        ("function", s(function)),
        ("members", Value::Array(out)),
        ("to", s(range.1)),
    ];
    if truncated {
        top.push(("truncated", Value::Bool(true)));
    }
    obj(top).to_jcs()
}

/// The CSV of an interval, long form (D33): one row a cell, the open key columns (`open` without the date; the
/// series file's fixed arguments are omitted), then `date`, `value` and `receipt` (the receipt hash). Rows are
/// by date, then by key, the column argument in dictionary order.
fn interval_csv(inner: &Inner, b: &Bound, page: &SeriesPage, open: &[String], members: &[SliceValue]) -> String {
    let keys: Vec<&String> = open.iter().filter(|k| **k != page.date_arg).collect();
    let rank: HashMap<String, usize> = match page.column_arg.as_deref() {
        Some(c) => {
            let seen: BTreeSet<String> = members.iter().filter_map(|m| key_of(m, c)).collect();
            columns_of(inner, b, c, &seen).into_iter().enumerate().map(|(i, v)| (v, i)).collect()
        }
        None => HashMap::new(),
    };
    let order = |m: &SliceValue| -> (String, Vec<(usize, String)>) {
        let ks = keys
            .iter()
            .map(|k| {
                let v = key_of(m, k).unwrap_or_default();
                let r = if page.column_arg.as_deref() == Some(k.as_str()) {
                    rank.get(&v).copied().unwrap_or(usize::MAX)
                } else {
                    0
                };
                (r, v)
            })
            .collect();
        (key_of(m, &page.date_arg).unwrap_or_default(), ks)
    };
    let mut sorted: Vec<&SliceValue> = members.iter().collect();
    sorted.sort_by(|a, c| order(a).cmp(&order(c)).then_with(|| a.version.tx.cmp(&c.version.tx)));
    let mut headers: Vec<String> = keys.iter().map(|k| (*k).clone()).collect();
    headers.extend(["date", "value", "receipt"].map(String::from));
    let rows: Vec<Vec<String>> = sorted
        .iter()
        .map(|m| {
            let mut r: Vec<String> = keys.iter().map(|k| key_of(m, k).unwrap_or_default()).collect();
            r.push(key_of(m, &page.date_arg).unwrap_or_default());
            r.push(m.value.lexical().to_string());
            r.push(m.version.receipt_hash.clone());
            r
        })
        .collect();
    use crate::slice::csv_field;
    let line = |cells: &[String]| cells.iter().map(|c| csv_field(c)).collect::<Vec<_>>().join(",") + "\r\n";
    let mut out = line(&headers);
    for r in &rows {
        out.push_str(&line(r));
    }
    out
}
