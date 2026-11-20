//! The takeover page (Build 2, QUESTIONS-build2 D24): `GET /takeover/<domain>`, linked from the
//! `source.takeover` of a relayed receipt and of `describe`. It lists the functions this mesh
//! relays from `<domain>`, says how the domain can publish them under its own name, and says
//! whether this mesh's `POST /bundles` admits the domain (`DATUMESH_PUBLISH_DOMAINS`, D11).
//!
//! It promises nothing more: it names no contact address and does not say the relay is withdrawn.
//! The data is built once as a JSON value and the HTML is rendered from it, like the function page.
//! Everything shown is a validated DNS name, a function name or a hash, and is escaped anyway.

use datumesh_core::json::Value;
use datumesh_core::{lexical, FunctionDescriptor};
use datumesh_store::CatalogKind;

use crate::envelope::{self, Cache, Reply};
use crate::fnpage::{esc, page_url, relay_of};
use crate::mesh::Inner;
use datumesh_core::ErrorCode;

fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

fn obj(members: Vec<(&str, Value)>) -> Value {
    Value::Object(members.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The functions bound now whose relay source is `domain`, by name: `(name, fn_hash)`.
fn relayed_from(inner: &Inner, domain: &str) -> Vec<(String, String)> {
    let Ok(summary) = inner.store.catalog_summary() else { return Vec::new() };
    let now = inner.now();
    let mut out = Vec::new();
    for e in summary.entries.iter().filter(|e| e.kind == CatalogKind::Function) {
        let Ok(Some(b)) = inner.store.resolve_binding(&e.name, &now) else { continue };
        let Some(d) = inner.store.descriptor(&b.fn_hash).and_then(|d| FunctionDescriptor::from_json(&d).ok()) else {
            continue;
        };
        if relay_of(inner, &e.name, &b.fn_hash, &d, &now).is_some_and(|r| r.name == domain) {
            out.push((e.name.clone(), b.fn_hash));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The page's data, or `None` when this mesh relays nothing from `domain`.
pub(crate) fn data(inner: &Inner, domain: &str) -> Option<Value> {
    let found = relayed_from(inner, domain);
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
        s(&format!("Serve a key document at https://{domain}/.well-known/datumesh.json (spec 9).")),
        s(&format!("Publish the same functions under {domain}/..., on its own mesh or hosted on this mesh. {hosting}")),
    ];
    Some(obj(vec![
        ("domain", s(domain)),
        ("host", s(host)),
        ("functions", Value::Array(functions)),
        ("steps", Value::Array(steps)),
    ]))
}

/// `GET /takeover/<domain>`: a malformed domain is `E_CANON_NAME` (400); a domain with no relayed
/// function here is `not_covered` (404, as an unbound name).
pub(crate) fn respond(inner: &Inner, domain: &str, json_form: bool) -> Reply {
    if lexical::check_dns_name(domain, "$").is_err() {
        return envelope::bad_request_reply(ErrorCode::CanonName);
    }
    let max_age = inner.config.ttl.latest_max_age.as_secs();
    let Some(d) = data(inner, domain) else {
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

fn text<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// The HTML page of the data `d` (no script; every string escaped).
pub(crate) fn render(d: &Value) -> String {
    let (domain, host) = (text(d, "domain"), text(d, "host"));
    let mut h = String::new();
    h.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    h.push_str(&format!("<title>{} | takeover</title>\n", esc(domain)));
    h.push_str("<style>body{font:16px/1.5 system-ui,sans-serif;margin:0;padding:16px;color:#1d1d1f;background:#fff}main{max-width:52rem;margin:0 auto}code{font-family:ui-monospace,monospace;font-size:14px;overflow-wrap:anywhere}h1{font-size:1.5rem;margin:0.5rem 0}h2{font-size:1.1rem}</style>\n</head>\n<body>\n<main>\n");
    h.push_str(&format!("<h1>Taking over <code>{}</code></h1>\n", esc(domain)));
    h.push_str(&format!(
        "<p>This mesh ({}) relays these values from {} unchanged. {} remains their source.</p>\n",
        esc(host),
        esc(domain),
        esc(domain)
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
    h.push_str("</ul>\n</section>\n<section id=\"steps\">\n<h2>How the domain can publish them under its own name</h2>\n<ol>\n");
    if let Some(Value::Array(ss)) = d.get("steps") {
        for st in ss {
            h.push_str(&format!("<li>{}</li>\n", esc(st.as_str().unwrap_or(""))));
        }
    }
    h.push_str("</ol>\n</section>\n</main>\n</body>\n</html>\n");
    h
}
