//! The function page (Build 2 Part 4, P2; CONTRACT-build2-part4-publish): `GET /fn/<function name>`,
//! for people (HTML, server-rendered, no script) and for agents (the same data as JSON, by
//! `Accept: application/json` or `?format=json`).
//!
//! The data is built once (as a JSON value, its members named as the contract names them) and the
//! HTML is rendered from it, so both forms carry the same facts. Publisher text is escaped; in the
//! description and the argument, example and case texts every space is a no-break space, so no
//! attribute-looking word ever follows a space in the markup.
//!
//! A relayed function (Build 2 Part 5, F5) is one whose stored receipts carry a `source`: the
//! function's first recorded cells are read (their receipts) and the first `source` found is its
//! source, cached for [`RELAY_TTL`].

use std::time::{Duration, Instant};

use datumesh_core::json::{self, Value};
use datumesh_core::{FunctionDescriptor, Receipt};
use datumesh_store::Store;

use crate::envelope::{self, Cache, Reply};
use crate::mesh::Inner;
use crate::testrun::{self, Expect, TestSpec};

/// How many of a function's recorded cells are read for a `source` (the first of them, lowest cell id first).
const RELAY_SCAN: usize = 8;
/// How long a function's source is remembered.
const RELAY_TTL: Duration = Duration::from_secs(300);

/// Where a relayed function's value is taken from (spec-0.6.0, 16.8): the source's name and its
/// takeover page, when the receipt gives one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Relay {
    pub(crate) name: String,
    pub(crate) takeover: Option<String>,
}

/// The page of `name` on this mesh (`https://<host>/fn/<name>`).
pub(crate) fn page_url(inner: &Inner, name: &str) -> String {
    format!("{}/fn/{name}", inner.config.origin())
}

/// The relay of a function (the first `source` among its first recorded cells), cached. A source found is kept
/// for the life of the process (a function's source does not change for a hash); a function with none is
/// looked at again after [`RELAY_TTL`].
pub(crate) fn relay_of(inner: &Inner, name: &str, fn_hash: &str) -> Option<Relay> {
    // Keyed by name and hash: two functions of one descriptor are two functions with their own cells.
    let key = format!("{name}\n{fn_hash}");
    if let Some((at, r)) = inner.relays.lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
        if r.is_some() || at.elapsed() < RELAY_TTL {
            return r.clone();
        }
    }
    let found = read_relay(inner, name, fn_hash);
    let mut cache = inner.relays.lock().unwrap_or_else(|p| p.into_inner());
    if cache.len() >= 10_000 {
        cache.clear();
    }
    cache.insert(key, (Instant::now(), found.clone()));
    found
}

/// A relay is found by reading the receipts of a function's first recorded cells. This is a heuristic
/// for now (all cells of a relay carry the source; a mixed function is not a relay the rule names). A
/// store index of the sources per function (filled by `publish-stored`) would answer without the
/// scan: that is the step after this one.
fn read_relay(inner: &Inner, name: &str, fn_hash: &str) -> Option<Relay> {
    let (ns, f) = name.rsplit_once('/')?;
    // The receipts of the first few stored cells, in one bounded read (CONTRACT-0.4.9 H2).
    let receipts = inner.store.stored_receipts_of(ns, f, fn_hash, RELAY_SCAN).ok()?;
    for bytes in receipts {
        let Ok(r) = Receipt::from_json(&bytes) else { continue };
        if let Some(s) = r.source() {
            return Some(Relay { name: s.name().to_string(), takeover: s.takeover().map(str::to_string) });
        }
    }
    None
}

/// The most tests a function page lists (its JSON `cases` and its HTML "Each test"); the total is given beside.
pub(crate) const CASES_SHOWN: usize = 100;

/// The cases a function page shows (the first [`CASES_SHOWN`]) and the total of them.
#[cfg(test)]
fn shown_cases(all: Vec<Value>) -> (Vec<Value>, usize) {
    let total = all.len();
    (all.into_iter().take(CASES_SHOWN).collect(), total)
}

fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

fn num(n: u64) -> Value {
    json::parse(n.to_string().as_bytes()).unwrap_or(Value::Null)
}

/// The reads of a JSON value this page makes (the core's value has no typed accessors for these).
trait Read4 {
    fn as_bool(&self) -> Option<bool>;
    fn as_u64(&self) -> Option<u64>;
    fn is_null(&self) -> bool;
}

impl Read4 for Value {
    fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    fn as_u64(&self) -> Option<u64> {
        self.as_number_text()?.parse().ok()
    }
    fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

fn obj(members: Vec<(&str, Value)>) -> Value {
    Value::Object(members.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The text of an expectation: the value, or the fault.
fn expect_text(e: &Expect) -> String {
    match e {
        Expect::Value(v) => v.lexical().to_string(),
        Expect::Fault(code) => format!("fault {code}"),
    }
}

/// The worst result of a function's stored tests: `error`, then `fail`, then `not run` when any test has not run
/// (or there are none), else `pass`.
fn worst_result(sum: &datumesh_store::TestSummary) -> &'static str {
    if sum.error > 0 {
        "error"
    } else if sum.fail > 0 {
        "fail"
    } else if sum.not_run > 0 || sum.count == 0 {
        "not run"
    } else {
        "pass"
    }
}

/// The page's data of `name`, or `None` when no function of that name is bound now.
pub(crate) fn data(inner: &Inner, name: &str) -> Option<Value> {
    let store: &dyn Store = &*inner.store;
    let now = inner.now();
    let binding = store.resolve_binding(name, &now).ok()??;
    let fn_hash = binding.fn_hash.clone();
    let d = FunctionDescriptor::from_json(&store.descriptor(&fn_hash)?).ok()?;
    let entry = crate::find::entry_by_name(store, name);
    let origin = inner.config.origin();
    let link = |n: &str| obj(vec![("name", s(n)), ("page", s(&page_url(inner, n)))]);

    // The catalogue: description, arguments, examples.
    let description = entry.as_ref().map(|e| e.description.clone()).filter(|t| !t.is_empty());
    let label = entry.as_ref().map(|e| e.label.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| name.to_string());
    let args: Vec<(String, Value)> =
        entry.as_ref().map(|e| e.args.iter().map(|(k, v)| (k.clone(), s(v))).collect()).unwrap_or_default();
    let examples_json: Vec<Value> = entry
        .as_ref()
        .and_then(|e| json::parse(e.examples.as_bytes()).ok())
        .and_then(|v| if let Value::Array(a) = v { Some(a) } else { None })
        .unwrap_or_default();
    let examples: Vec<Value> = examples_json
        .iter()
        .filter_map(|x| {
            let call = x.get("call").and_then(Value::as_str)?.to_string();
            let cell = format!("{origin}/cell/{}", crate::client::pct(&call));
            Some(obj(vec![
                ("call", s(&call)),
                ("expected", x.get("expected").cloned().unwrap_or(Value::Null)),
                ("cell", s(&cell)),
            ]))
        })
        .collect();

    // The signature: the descriptor's, with the order, the hash and the binding history.
    let order: Vec<Value> = match &d.order {
        Some(o) => o.iter().map(|x| s(x)).collect(),
        None => d.signature.args.keys().map(|x| s(x)).collect(),
    };
    let sig = d.signature.to_value();
    let mut sig_members: Vec<(String, Value)> = match sig {
        Value::Object(m) => m,
        _ => Vec::new(),
    };
    sig_members.retain(|(k, _)| k != "order" && k != "fn_hash" && k != "bindings");
    sig_members.push(("order".into(), Value::Array(order)));
    sig_members.push(("fn_hash".into(), s(&fn_hash)));
    sig_members
        .push(("bindings".into(), Value::Array(vec![obj(vec![("fn_hash", s(&fn_hash)), ("since", s(&binding.tx))])])));
    let signature = Value::Object(sig_members);

    // The body: a computed function's formula.
    let body = if d.lang == "stored" { Value::Null } else { s(&d.body) };

    // The tests: the stored results only (CONTRACT-0.4.9 H1); a request runs nothing.
    let unnamed = inner.is_unnamed(&fn_hash);
    let summary = testrun::summary_of(store, &fn_hash, name, unnamed, &now).unwrap_or_default();
    let open: Vec<Value> = testrun::open_counterexamples(store, &inner.config.host, name, &fn_hash, unnamed, &now)
        .unwrap_or_default()
        .into_iter()
        .map(|c| obj(vec![("signer", s(&c.signer)), ("case", s(&c.case))]))
        .collect();
    // Each case with its last result (`pass`, `fail`, `error`; null before its first run), the first 100.
    let held = testrun::rows_page(store, &fn_hash, name, unnamed, &now, CASES_SHOWN).unwrap_or_default();
    let cases: Vec<Value> = held
        .iter()
        .map(|(row, spec): &(_, TestSpec)| {
            let result = row.last_result.as_deref().map_or(Value::Null, s);
            obj(vec![("call", s(&spec.call)), ("expect", s(&expect_text(&spec.expect))), ("result", result)])
        })
        .collect();
    let cases_total = summary.count;
    let at = summary.last_run_at.as_deref().map_or(Value::Null, s);
    let tests = obj(vec![
        ("count", num(summary.count)),
        ("branches", num(summary.branches_total as u64)),
        ("covered", num(summary.branches_covered as u64)),
        ("counterexamples", num(open.len() as u64)),
        ("open_counterexamples", Value::Array(open)),
        ("cases", Value::Array(cases)),
        ("cases_total", num(cases_total)),
        (
            "last_run",
            obj(vec![
                ("result", s(worst_result(&summary))),
                ("at", at),
                ("pass", num(summary.pass)),
                ("fail", num(summary.fail)),
                ("error", num(summary.error)),
                ("not_run", num(summary.not_run)),
            ]),
        ),
    ]);

    // The lineage: inputs, dependants on this mesh, the relay's source.
    let inputs: Vec<Value> = d.inputs.iter().map(|n| link(n)).collect();
    let mut dependants: Vec<String> = Vec::new();
    for h in store.function_readers(name).unwrap_or_default() {
        for n in store.bound_names(&h).unwrap_or_default() {
            let bound_now = store.resolve_binding(&n, &now).ok().flatten().is_some_and(|b| b.fn_hash == h);
            if bound_now && !dependants.contains(&n) {
                dependants.push(n);
            }
        }
    }
    dependants.sort();
    let mut lineage = vec![
        ("inputs", Value::Array(inputs)),
        ("dependants", Value::Array(dependants.iter().map(|n| link(n)).collect())),
    ];
    let relay = relay_of(inner, name, &fn_hash);
    if let Some(r) = &relay {
        let mut src = vec![("name", s(&r.name)), ("relayed_by", s(&inner.config.host))];
        if let Some(t) = &r.takeover {
            src.push(("takeover", s(t)));
        }
        lineage.push(("source", obj(src)));
    }

    // The licence: the publisher's declarations of this function (SPEC 9.2, 8.5).
    let host = name.split('/').next().unwrap_or("");
    let declared: Vec<Value> = store
        .namespace(host)
        .ok()
        .flatten()
        .and_then(|n| json::parse(&n.keydoc).ok())
        .and_then(|doc| doc.get("declarations").and_then(|d| d.get(name)).cloned())
        .and_then(|v| if let Value::Array(a) = v { Some(a) } else { None })
        .unwrap_or_default();

    Some(obj(vec![
        ("name", s(name)),
        ("page", s(&page_url(inner, name))),
        ("fn_hash", s(&fn_hash)),
        ("label", s(&label)),
        ("description", description.as_deref().map_or(Value::Null, s)),
        ("args", Value::Object(args)),
        ("signature", signature),
        ("body", body),
        ("tests", tests),
        ("lineage", obj(lineage)),
        ("examples", Value::Array(examples)),
        ("licence", obj(vec![("declarations", Value::Array(declared))])),
    ]))
}

/// `GET /fn/<name>`: the JSON form when asked for it, else the HTML page; `not_covered` when no
/// function of that name is bound on this mesh.
pub(crate) fn respond(inner: &Inner, name: &str, json_form: bool) -> Reply {
    let Some(d) = data(inner, name) else {
        if json_form {
            return envelope::name_unbound_reply(inner.config.ttl.latest_max_age.as_secs());
        }
        let body = format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>not covered</title></head>\
             <body><h1>not_covered</h1><p>E_NAME_UNBOUND: no function named {} is bound on this mesh.</p></body></html>",
            esc(name)
        );
        return Reply {
            status: 404,
            body: body.into_bytes().into(),
            content_type: "text/html; charset=utf-8",
            cache: Cache::Public(60),
            canonical: None,
            retry_after: None,
        };
    };
    if json_form {
        return Reply::json(200, d.to_jcs(), Cache::Public(inner.config.ttl.latest_max_age.as_secs()));
    }
    let unnamed = d.get("fn_hash").and_then(Value::as_str).is_some_and(|h| inner.is_unnamed(h));
    let body = render(&d, unnamed);
    Reply {
        status: 200,
        body: body.into_bytes().into(),
        content_type: "text/html; charset=utf-8",
        cache: Cache::Public(inner.config.ttl.latest_max_age.as_secs()),
        canonical: None,
        retry_after: None,
    }
}

/// Escapes text for an HTML text node or a double-quoted attribute.
pub(crate) fn esc(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    for c in t.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Publisher text as markup: escaped, every space a no-break space (see the module documentation).
fn prose(t: &str) -> String {
    esc(t).replace(' ', "\u{a0}")
}

fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&Value::Null)
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(x) => x.clone(),
        Value::Null => String::new(),
        other => String::from_utf8_lossy(&other.to_jcs()).into_owned(),
    }
}

/// The HTML page of the data `d` (no script; every publisher string escaped). `unnamed`: the function's hash
/// still has tests no one has named, so its counts may include tests of other functions of the same signature
/// (CONTRACT-0.4.9 H1).
fn render(d: &Value, unnamed: bool) -> String {
    let name = get(d, "name").as_str().unwrap_or("");
    let label = get(d, "label").as_str().unwrap_or(name);
    let mut h = String::new();
    h.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    h.push_str(&format!("<title>{} | {}</title>\n", esc(label), esc(name)));
    h.push_str("<style>body{font:16px/1.5 system-ui,sans-serif;margin:0;padding:16px;color:#1d1d1f;background:#fff}main{max-width:52rem;margin:0 auto}code,pre{font-family:ui-monospace,monospace;font-size:14px;overflow-x:auto}section{border-top:1px solid #d2d2d7;padding:12px 0}h1{font-size:1.5rem;margin:0.5rem 0}h2{font-size:1.1rem;margin:0 0 .5rem}</style>\n</head>\n<body>\n<main>\n");
    h.push_str(&format!("<header><h1>{}</h1><p><code>{}</code></p></header>\n", esc(label), esc(name)));

    // description
    h.push_str("<section id=\"description\">\n<h2>Description</h2>\n");
    match get(d, "description").as_str() {
        Some(t) => h.push_str(&format!("<p>{}</p>\n", prose(t))),
        None => h.push_str("<p>No description is published.</p>\n"),
    }
    if let Value::Object(args) = get(d, "args") {
        if !args.is_empty() {
            h.push_str("<dl>\n");
            for (k, v) in args {
                h.push_str(&format!("<dt><code>{}</code></dt><dd>{}</dd>\n", esc(k), prose(&text_of(v))));
            }
            h.push_str("</dl>\n");
        }
    }
    h.push_str("</section>\n");

    // signature
    let sig = get(d, "signature");
    h.push_str("<section id=\"signature\">\n<h2>Signature</h2>\n<ul>\n");
    if let Value::Object(args) = get(sig, "args") {
        let order: Vec<String> = match get(sig, "order") {
            Value::Array(a) => a.iter().map(text_of).collect(),
            _ => args.iter().map(|(k, _)| k.clone()).collect(),
        };
        for k in order {
            if let Some((_, decl)) = args.iter().find(|(n, _)| *n == k) {
                let ty = get(decl, "type").as_str().unwrap_or("");
                let unit = get(decl, "unit").as_str().map(|u| format!(", unit {u}")).unwrap_or_default();
                let valid = if get(decl, "valid").as_bool() == Some(true) { ", valid time" } else { "" };
                h.push_str(&format!("<li><code>{}</code>: {}{}{}</li>\n", esc(&k), esc(ty), esc(&unit), valid));
            }
        }
    }
    let ret = get(sig, "returns");
    let ret_text = match ret {
        Value::String(t) => t.clone(),
        Value::Object(_) => {
            let ty = get(ret, "type").as_str().unwrap_or("");
            match get(ret, "unit").as_str() {
                Some(u) => format!("{ty}, unit {u}"),
                None => ty.to_string(),
            }
        }
        _ => String::new(),
    };
    h.push_str(&format!("<li>returns: {}</li>\n", esc(&ret_text)));
    h.push_str(&format!("<li>fn_hash: <code>{}</code></li>\n", esc(get(sig, "fn_hash").as_str().unwrap_or(""))));
    h.push_str("</ul>\n<h3>Bindings</h3>\n<ul>\n");
    if let Value::Array(bs) = get(sig, "bindings") {
        for b in bs {
            h.push_str(&format!(
                "<li><code>{}</code> since {}</li>\n",
                esc(get(b, "fn_hash").as_str().unwrap_or("")),
                esc(get(b, "since").as_str().unwrap_or(""))
            ));
        }
    }
    h.push_str("</ul>\n</section>\n");

    // body
    h.push_str("<section id=\"body\">\n<h2>Body</h2>\n");
    match get(d, "body").as_str() {
        Some(b) => h.push_str(&format!("<pre><code>{}</code></pre>\n", esc(b))),
        None => h.push_str("<p>A stored function: its values are published, it has no formula.</p>\n"),
    }
    h.push_str("</section>\n");

    // tests
    let t = get(d, "tests");
    let n = |k: &str| get(t, k).as_u64().unwrap_or(0);
    h.push_str("<section id=\"tests\">\n<h2>Tests</h2>\n");
    h.push_str(&format!(
        "<p>{} tests, {} of {} branches covered, {} counterexamples.</p>\n",
        n("count"),
        n("covered"),
        n("branches"),
        n("counterexamples")
    ));
    let last = get(t, "last_run");
    h.push_str(&format!(
        "<p>Worst result: {}; last run {}.</p>\n",
        esc(get(last, "result").as_str().unwrap_or("")),
        esc(get(last, "at").as_str().unwrap_or("never"))
    ));
    if unnamed {
        h.push_str(
            "<p>The counts include tests of other functions of the same signature (their tests are being named).</p>\n",
        );
    }
    h.push_str("<h3>Counterexamples</h3>\n<ul>\n");
    if let Value::Array(open) = get(t, "open_counterexamples") {
        for c in open {
            h.push_str(&format!(
                "<li>open counterexample from {}: case <code>{}</code></li>\n",
                esc(get(c, "signer").as_str().unwrap_or("")),
                esc(get(c, "case").as_str().unwrap_or(""))
            ));
        }
    }
    h.push_str("</ul>\n<h3>Each test</h3>\n<ul>\n");
    if let Value::Array(cs) = get(t, "cases") {
        for c in cs {
            h.push_str(&format!(
                "<li><code>{}</code> expects {}: {}</li>\n",
                prose(get(c, "call").as_str().unwrap_or("")),
                prose(get(c, "expect").as_str().unwrap_or("")),
                esc(get(c, "result").as_str().unwrap_or("not run"))
            ));
        }
    }
    let shown = if let Value::Array(cs) = get(t, "cases") { cs.len() } else { 0 };
    let total = n("cases_total") as usize;
    if total > shown {
        h.push_str(&format!("<p>Showing the first {shown} of {total} tests.</p>\n"));
    }
    h.push_str("</ul>\n</section>\n");

    // lineage
    let lin = get(d, "lineage");
    h.push_str("<section id=\"lineage\">\n<h2>Lineage</h2>\n");
    for (title, key) in [("Inputs", "inputs"), ("Dependants on this mesh", "dependants")] {
        h.push_str(&format!("<h3>{title}</h3>\n<ul>\n"));
        if let Value::Array(items) = get(lin, key) {
            for it in items {
                let n = get(it, "name").as_str().unwrap_or("");
                h.push_str(&format!(
                    "<li><a href=\"{}\"><code>{}</code></a></li>\n",
                    esc(get(it, "page").as_str().unwrap_or("")),
                    esc(n)
                ));
            }
        }
        h.push_str("</ul>\n");
    }
    let src = get(lin, "source");
    if !src.is_null() {
        let relayed_by = get(src, "relayed_by").as_str().unwrap_or("");
        h.push_str(&format!(
            "<p>Source: {} (relayed by {}).</p>\n",
            esc(get(src, "name").as_str().unwrap_or("")),
            esc(relayed_by)
        ));
        if let Some(take) = get(src, "takeover").as_str() {
            h.push_str(&format!(
                "<p>The source can take over publishing these values: <a href=\"{}\">{}</a></p>\n",
                esc(take),
                esc(take)
            ));
        }
    }
    h.push_str("</section>\n");

    // examples
    h.push_str("<section id=\"examples\">\n<h2>Examples</h2>\n");
    match get(d, "examples") {
        Value::Array(ex) if !ex.is_empty() => {
            h.push_str("<ul>\n");
            for e in ex {
                let call = get(e, "call").as_str().unwrap_or("");
                let expected = match get(e, "expected") {
                    Value::Object(_) => get(get(e, "expected"), "value")
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| text_of(get(e, "expected"))),
                    other => text_of(other),
                };
                h.push_str(&format!(
                    "<li><a href=\"{}\">{}</a> gives {}</li>\n",
                    esc(get(e, "cell").as_str().unwrap_or("")),
                    prose(call),
                    prose(&expected)
                ));
            }
            h.push_str("</ul>\n");
        }
        _ => h.push_str("<p>No examples are published.</p>\n"),
    }
    h.push_str("</section>\n");

    // licence
    h.push_str("<section id=\"licence\">\n<h2>Licence and declarations</h2>\n");
    match get(get(d, "licence"), "declarations") {
        Value::Array(ds) if !ds.is_empty() => {
            h.push_str("<ul>\n");
            for dec in ds {
                h.push_str(&format!(
                    "<li>{} from {}{}</li>\n",
                    esc(get(dec, "licence").as_str().unwrap_or("")),
                    esc(get(dec, "valid_from").as_str().unwrap_or("")),
                    get(dec, "valid_to").as_str().map(|t| format!(" until {}", esc(t))).unwrap_or_default()
                ));
            }
            h.push_str("</ul>\n");
        }
        _ => h.push_str("<p>No declarations.</p>\n"),
    }
    h.push_str("</section>\n");
    h.push_str("</main>\n</body>\n</html>\n");
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_function_page_lists_at_most_the_first_100_cases_and_the_total() {
        let all: Vec<Value> = (0..8_120).map(|i| s(&format!("case {i}"))).collect();
        let (shown, total) = shown_cases(all);
        assert_eq!((shown.len(), total), (CASES_SHOWN, 8_120));
        assert_eq!(shown[0].as_str(), Some("case 0"));
        assert_eq!(shown[99].as_str(), Some("case 99"));
        let few: Vec<Value> = (0..3).map(|i| s(&format!("case {i}"))).collect();
        assert_eq!(shown_cases(few).0.len(), 3);
    }
}
