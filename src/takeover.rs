//! The takeover page (Build 2, QUESTIONS-build2 D24; rewritten in site round 3, CONTRACT-site-round3 B3): `GET /takeover/<domain>`,
//! linked from the `source.takeover` of a relayed receipt and of `describe`. It lists the functions this mesh relays from
//! `<domain>` as a stand-in, says how the source's own name comes first in this mesh's catalogue, how the relay can stop, and
//! how to serve a mesh of one's own; it says whether this mesh's `POST /bundles` admits the domain (`DATUMESH_PUBLISH_DOMAINS`, D11).
//!
//! It names a contact address only when one is configured (`DATUMESH_CONTACT`), and then under "Stopping the relay".
//! The data is built once as a JSON value and the HTML is rendered from it, like the function page.
//! Everything shown is a validated DNS name, a function name or a hash, and is escaped anyway.

//! The relayed functions of every source domain are listed by a thread of the mesh (CONTRACT-0.4.9 H2): a request
//! reads the last build, and before its first build answers `503` with `Retry-After` and `no-store`.

use std::collections::BTreeMap;
use std::sync::Arc;

use datumesh_core::json::Value;
use datumesh_core::lexical;
use datumesh_store::CatalogKind;

use crate::envelope::{self, Cache, Reply};
use crate::fnpage::{esc, page_url, relay_of};
use crate::mesh::Inner;
use datumesh_core::ErrorCode;

/// `Retry-After` of the takeover page before its first build, in seconds.
const RETRY_SECS: u64 = 5;

/// The relayed functions of each source domain, by name: `(name, fn_hash)`, sorted.
pub(crate) type Lists = BTreeMap<String, Vec<(String, String)>>;

fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

fn obj(members: Vec<(&str, Value)>) -> Value {
    Value::Object(members.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The most catalogue entries a takeover build reads.
const TAKEOVER_ENTRIES: usize = 100_000;

/// The relayed functions of every source domain, from the names bound now (the thread's build, never a request's):
/// each function that has a relay source is listed under its domain.
pub(crate) fn build(inner: &Inner) -> Lists {
    let mut out: Lists = BTreeMap::new();
    // The names come from one read of the entries, with no counts (CONTRACT-round-0.7-P HP7).
    let Ok(entries) = inner.store.catalog_entries_all(TAKEOVER_ENTRIES) else { return out };
    let now = inner.now();
    for (_, name, _, _) in entries.iter().filter(|e| e.0 == CatalogKind::Function) {
        let Ok(Some(b)) = inner.store.resolve_binding(name, &now) else { continue };
        if let Some(r) = relay_of(inner, name, &b.fn_hash) {
            out.entry(r.name).or_default().push((name.clone(), b.fn_hash));
        }
    }
    for list in out.values_mut() {
        list.sort();
        list.dedup();
    }
    out
}

/// Runs one build of the lists and makes it the one requests read (the takeover thread).
pub(crate) fn refresh(inner: &Inner) {
    let lists = Arc::new(build(inner));
    *inner.takeover.lock().unwrap_or_else(|p| p.into_inner()) = Some(lists);
}

/// The relayed functions of `domain` as the last build has them; `None` before the first build.
fn relayed_from(inner: &Inner, domain: &str) -> Option<Vec<(String, String)>> {
    let lists = inner.takeover.lock().unwrap_or_else(|p| p.into_inner()).clone()?;
    Some(lists.get(domain).cloned().unwrap_or_default())
}

/// The section "How the source's own name comes first" (CONTRACT-site-round3 B3), the same sentences on the page and in the
/// JSON `policy`. The relay is a stand-in: the source's own publication ranks above it under the catalogue rule.
fn policy(domain: &str) -> Vec<String> {
    vec![
        format!(
            "When {domain} publishes these functions under its own domain, its own name ranks first among the sources of the \
             same subject in this mesh's catalogue."
        ),
        format!("No permission is needed: {domain} proves control of its domain with a key document on its own domain."),
        "Datumesh lists a source's mesh once it exists: a mesh is listed when it publishes its own catalogue with subject keys \
         and Datumesh adds its host to the list."
            .to_string(),
        "The relay goes on as a mirror, labelled relayed. Its history stays as this mesh's signed statements.".to_string(),
    ]
}

/// The section "Stopping the relay", the same sentences on the page and in the JSON `stopping`. No declaration is read yet.
fn stopping(domain: &str) -> Vec<String> {
    vec![
        format!(
            "{domain} can ask for the relay to stop. A declaration it can put in its own key document for this is planned; \
             this mesh does not read one yet."
        ),
        format!("Until then, when {domain} asks, the feed is ended."),
    ]
}

/// The page's data, or `None` when this mesh relays nothing from `domain`.
pub(crate) fn data(inner: &Inner, domain: &str, found: &[(String, String)]) -> Option<Value> {
    if found.is_empty() {
        return None;
    }
    let host = &inner.config.host;
    let hosting = if inner.config.publish_domains.allows(domain) {
        format!("POST /bundles is open to {domain} on this mesh ({host}): a bundle signed by a key in its key document is accepted.")
    } else {
        format!(
            "Hosting on this mesh ({host}) needs the operator of this mesh to allow {domain}: POST /bundles does not admit it yet."
        )
    };
    let functions: Vec<Value> = found
        .iter()
        .map(|(n, h)| obj(vec![("name", s(n)), ("page", s(&page_url(inner, n))), ("fn_hash", s(h))]))
        .collect();
    let steps = vec![
        s(&format!("Make a key, then serve its key document at https://{domain}/.well-known/datumesh.json (spec 9).")),
        s(&format!("Publish the same functions under {domain}/..., on its own mesh or hosted on this mesh.")),
        s(&hosting),
        s(&format!("The full route, with each command, is at {PUBLISH_DOCS}.")),
    ];
    let contact = inner.config.contact.as_deref().map_or(Value::Null, s);
    Some(obj(vec![
        ("contact", contact),
        ("domain", s(domain)),
        ("host", s(host)),
        ("functions", Value::Array(functions)),
        ("policy", Value::Array(policy(domain).iter().map(|p| s(p)).collect())),
        ("steps", Value::Array(steps)),
        ("stopping", Value::Array(stopping(domain).iter().map(|p| s(p)).collect())),
    ]))
}

/// `GET /takeover/<domain>`: a malformed domain is `E_CANON_NAME` (400); a domain with no relayed
/// function here is `not_covered` (404, as an unbound name).
pub(crate) fn respond(inner: &Inner, domain: &str, json_form: bool) -> Reply {
    if lexical::check_dns_name(domain, "$").is_err() {
        return envelope::bad_request_reply(ErrorCode::CanonName);
    }
    let max_age = inner.config.ttl.latest_max_age.as_secs();
    let Some(found) = relayed_from(inner, domain) else {
        return envelope::not_yet_reply(RETRY_SECS, "The takeover list");
    };
    let Some(d) = data(inner, domain, &found) else {
        if json_form {
            return envelope::name_unbound_reply(max_age);
        }
        let body = format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>not covered</title></head>\
             <body><h1>not_covered</h1><p>This mesh relays no function from {}.</p></body></html>",
            esc(domain)
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
        return Reply::json(200, d.to_jcs(), Cache::Public(max_age));
    }
    Reply {
        status: 200,
        body: render(&d).into_bytes().into(),
        content_type: "text/html; charset=utf-8",
        cache: Cache::Public(max_age),
        canonical: None,
        retry_after: None,
    }
}

/// The Publish page, linked from the last step of "Serving your own mesh".
const PUBLISH_DOCS: &str = "https://datumesh.com/docs/publish";

/// The catalogue rule's page, linked from the first sentence of "How the source's own name comes first".
const RULE_PAGE: &str = "/catalog/rule";

fn text<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// The strings of an array member (none when the member is absent).
fn strings<'a>(v: &'a Value, k: &str) -> Vec<&'a str> {
    match v.get(k) {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

/// Text content, escaped: `&`, `<` and `>`. An apostrophe is legal in text and is kept as written, so a sentence reads as
/// the contract words it carries; attribute values go through [`esc`].
fn text_esc(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// `t` as text, with its first `phrase` wrapped in a link to `href`.
fn linked(t: &str, phrase: &str, href: &str) -> String {
    match t.split_once(phrase) {
        Some((before, after)) => {
            format!("{}<a href=\"{}\">{}</a>{}", text_esc(before), esc(href), text_esc(phrase), text_esc(after))
        }
        None => text_esc(t),
    }
}

/// The HTML page of the data `d` (no script; every string escaped).
pub(crate) fn render(d: &Value) -> String {
    let (domain, host) = (text(d, "domain"), text(d, "host"));
    let title = format!("{domain}'s values under its own name");
    let mut h = String::new();
    h.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    h.push_str(&format!("<title>{}</title>\n", esc(&title)));
    h.push_str("<style>body{font:16px/1.5 system-ui,sans-serif;margin:0;padding:16px;color:#1d1d1f;background:#fff;overflow-wrap:anywhere}main{max-width:52rem;margin:0 auto}code{font-family:ui-monospace,monospace;font-size:14px;overflow-wrap:anywhere}h1{font-size:1.5rem;margin:0.5rem 0}h2{font-size:1.1rem}pre{overflow-x:auto;max-width:100%;box-sizing:border-box}</style>\n</head>\n<body>\n<main>\n");
    h.push_str(&format!("<h1>{}</h1>\n", esc(&title)));
    h.push_str(&format!(
        "<p>This mesh ({}) relays these values from {} unchanged, as a stand-in. {} remains their source.</p>\n",
        text_esc(host),
        text_esc(domain),
        text_esc(domain)
    ));
    h.push_str("<section id=\"functions\">\n<h2>Functions relayed from this domain</h2>\n<ul>\n");
    if let Some(Value::Array(fs)) = d.get("functions") {
        for f in fs {
            h.push_str(&format!(
                "<li><a href=\"{}\"><code>{}</code></a> (<code>{}</code>)</li>\n",
                esc(text(f, "page")),
                esc(text(f, "name")),
                esc(text(f, "fn_hash"))
            ));
        }
    }
    h.push_str("</ul>\n</section>\n");
    h.push_str("<section id=\"first\">\n<h2>How the source's own name comes first</h2>\n<ul>\n");
    for p in strings(d, "policy") {
        h.push_str(&format!("<li>{}</li>\n", linked(p, "ranks first", RULE_PAGE)));
    }
    h.push_str("</ul>\n</section>\n");
    h.push_str("<section id=\"stopping\">\n<h2>Stopping the relay</h2>\n<ul>\n");
    for p in strings(d, "stopping") {
        h.push_str(&format!("<li>{}</li>\n", text_esc(p)));
    }
    h.push_str("</ul>\n");
    if let Some(c) = d.get("contact").and_then(Value::as_str) {
        h.push_str(&format!("<p>Contact: <code>{}</code>.</p>\n", esc(c)));
    }
    h.push_str("</section>\n");
    h.push_str("<section id=\"serving\">\n<h2>Serving your own mesh</h2>\n<ol>\n");
    for st in strings(d, "steps") {
        h.push_str(&format!("<li>{}</li>\n", linked(st, PUBLISH_DOCS, PUBLISH_DOCS)));
    }
    h.push_str("</ol>\n</section>\n</main>\n</body>\n</html>\n");
    h
}
