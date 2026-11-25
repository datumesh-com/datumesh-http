//! The takeover page (QUESTIONS-build2 D24, http 0.4.4): `GET /takeover/<domain>`, for people (HTML,
//! no script) and for agents (JSON by `Accept: application/json` or `?format=json`). It lists the
//! functions this mesh relays from the domain, links each to its function page, says how the domain
//! can publish them under its own name, and says whether this mesh's `POST /bundles` admits the domain.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use calculus::*;
use common::lite::*;
use common::pub031::function_line;
use common::readpath::*;
use datumesh_http::config::PublishDomains;
use publish4::get;
use serde_json::{json, Value};

const A1: &str = "mesh-test.datumesh.com/scratch_b5/relay/a1";
const A2: &str = "mesh-test.datumesh.com/scratch_b5/relay/a2";
const B1: &str = "mesh-test.datumesh.com/scratch_b5/relay/b1";
const OWN: &str = "mesh-test.datumesh.com/scratch_b5/own/level";
const ECB: &str = "ecb.europa.eu";
const OTHER: &str = "stats.example.org";

fn relayed(f: &str, domain: &str) -> String {
    let mut v: Value = serde_json::from_str(&site_cell(f, "N", "1.5")).unwrap();
    v["source"] = json!({"name": domain, "url": format!("https://{domain}/data.csv"),
        "retrieved": "2026-11-15T10:00:00.000000Z",
        "takeover": format!("https://mesh-test.datumesh.com/takeover/{domain}")});
    v.to_string()
}

fn lines() -> Vec<String> {
    let mut v = Vec::new();
    for (f, d) in [(A1, ECB), (A2, ECB), (B1, OTHER)] {
        v.push(function_line(f, &reading_sig(), None));
        v.push(relayed(f, d));
        v.push(catalog_line(f, "function", "Rate", "A relayed rate.", json!({"site": "the site"}), json!({})));
    }
    v.push(function_line(OWN, &reading_sig(), None));
    v.push(site_cell(OWN, "N", "7"));
    v.push(catalog_line(OWN, "function", "Level", "A stored level.", json!({"site": "the site"}), json!({})));
    v
}

async fn served(label: &str, domains: PublishDomains) -> Served {
    serve_lite(label, &lines(), None, move |c| c.publish_domains = domains).await
}

/// The decoded text of a piece of HTML: tags removed, entities decoded, white space folded.
fn plain(h: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in h.chars() {
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
    for (e, d) in [("&quot;", "\""), ("&#39;", "'"), ("&lt;", "<"), ("&gt;", ">"), ("&amp;", "&")] {
        out = out.replace(e, d);
    }
    out.replace('\u{a0}', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn names(j: &Value) -> Vec<String> {
    j["functions"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap().to_string()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_page_lists_exactly_the_functions_relayed_from_that_domain() {
    let m = served("tk-list", PublishDomains::Own).await;
    let ecb = get(&m, &format!("/takeover/{ECB}?format=json"), &[]).await;
    assert_eq!(ecb.status, 200, "{}", ecb.text());
    assert_eq!(names(&ecb.json()), vec![A1.to_string(), A2.to_string()], "{}", ecb.text());
    let other = get(&m, &format!("/takeover/{OTHER}?format=json"), &[]).await.json();
    assert_eq!(names(&other), vec![B1.to_string()], "{other}");
    // the HTML names the same functions, each linked to its function page, and not the others
    let html = get(&m, &format!("/takeover/{ECB}"), &[]).await;
    assert_eq!(html.status, 200);
    assert!(html.header("content-type").unwrap().starts_with("text/html"));
    let h = html.text();
    for f in [A1, A2] {
        assert!(h.contains(&format!("href=\"https://{HOST}/fn/{f}\"")) || h.contains(&format!("/fn/{f}\"")), "{h}");
    }
    for f in [B1, OWN] {
        assert!(!h.contains(f), "{f} is not relayed from {ECB}: {h}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_json_shape() {
    let m = served("tk-json", PublishDomains::Own).await;
    let by_accept = get(&m, &format!("/takeover/{ECB}"), &[("accept", "application/json")]).await;
    let by_query = get(&m, &format!("/takeover/{ECB}?format=json"), &[]).await;
    assert_eq!(by_accept.status, 200);
    assert_eq!(by_accept.json(), by_query.json());
    let j = by_query.json();
    assert_eq!(j["domain"], ECB);
    assert_eq!(j["host"], HOST);
    let fns = j["functions"].as_array().unwrap();
    assert_eq!(fns.len(), 2);
    for f in fns {
        let name = f["name"].as_str().unwrap();
        assert_eq!(f["page"], format!("https://{HOST}/fn/{name}"), "{f}");
        let page = get(&m, &format!("/fn/{name}?format=json"), &[]).await.json();
        assert_eq!(f["fn_hash"], page["fn_hash"], "{f}");
    }
    // the steps are the Publish page's, short (CONTRACT-site-round3 B3): no fixed count any more
    let steps: Vec<&str> = j["steps"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(!steps.is_empty(), "{j}");
    assert!(steps.iter().any(|s| s.contains(&format!("https://{ECB}/.well-known/datumesh.json"))), "{j}");
    assert!(steps.iter().any(|s| s.contains(&format!("{ECB}/..."))), "{j}");
    let mut keys: Vec<&String> = j.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["contact", "domain", "functions", "host", "policy", "steps", "stopping"]);
    // cached like the function page
    assert_eq!(
        by_query.header("cache-control"),
        get(&m, &format!("/fn/{A1}?format=json"), &[]).await.header("cache-control")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_text_says_what_is_and_is_not_promised() {
    let m = served("tk-text", PublishDomains::Own).await;
    let t = plain(&get(&m, &format!("/takeover/{ECB}"), &[]).await.text());
    assert!(t.contains(&format!("This mesh ({HOST}) relays these values from {ECB} unchanged")), "{t}");
    assert!(t.contains(&format!("{ECB} remains their source")), "{t}");
    assert!(t.contains(&format!("https://{ECB}/.well-known/datumesh.json")), "{t}");
    assert!(t.contains(&format!("under {ECB}/...")), "{t}");
    assert!(t.contains("on its own mesh or hosted on this mesh"), "{t}");
    for no in ["@", "contact", "withdrawn", "withdraw", "retire"] {
        assert!(!t.to_lowercase().contains(no), "{no} in: {t}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hosting_here_depends_on_publish_domains() {
    let closed = "needs the operator of this mesh to allow";
    let open = "POST /bundles is open to";
    for (label, domains, admits) in [
        ("tk-own", PublishDomains::Own, false),
        ("tk-other", PublishDomains::Hosts(vec![OTHER.into()]), false),
        ("tk-named", PublishDomains::Hosts(vec![ECB.into()]), true),
        ("tk-any", PublishDomains::Any, true),
    ] {
        let m = served(label, domains).await;
        let t = plain(&get(&m, &format!("/takeover/{ECB}"), &[]).await.text());
        let j = get(&m, &format!("/takeover/{ECB}?format=json"), &[]).await.text();
        for (what, text) in [("html", t.as_str()), ("json", j.as_str())] {
            assert_eq!(text.contains(open), admits, "{label} {what}: {text}");
            assert_eq!(text.contains(closed), !admits, "{label} {what}: {text}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_script_and_everything_escaped() {
    let m = served("tk-xss", PublishDomains::Own).await;
    let h = get(&m, &format!("/takeover/{ECB}"), &[]).await.text().to_lowercase();
    for bad in ["<script", "javascript:", " onload=", " onclick=", " onerror=", "<iframe"] {
        assert!(!h.contains(bad), "{bad}: {h}");
    }
    // a domain that is not a DNS name is refused and never echoed as markup
    for d in ["%3Cscript%3Ealert(1)%3C%2Fscript%3E", "a%22%3E%3Csvg%20onload=x%3E.com"] {
        let r = get(&m, &format!("/takeover/{d}"), &[("accept", "text/html")]).await;
        assert_eq!(r.status, 400, "{}", r.text());
        assert!(!r.text().contains("<script") && !r.text().contains("<svg"), "{}", r.text());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_domain_with_no_relayed_function_is_not_covered() {
    let m = served("tk-404", PublishDomains::Own).await;
    // a domain with a stored, non-relayed function of its own is not covered either
    for d in ["nobody.example", HOST] {
        let r = get(&m, &format!("/takeover/{d}?format=json"), &[]).await;
        assert_eq!(r.status, 404, "{d}: {}", r.text());
        let j = r.json();
        assert_eq!(j["outcome"], "not_covered", "{j}");
        assert_eq!(j["error"], "E_NAME_UNBOUND", "{j}");
    }
    let r = get(&m, "/takeover/nobody.example", &[]).await;
    assert_eq!(r.status, 404);
    assert!(r.text().contains("not_covered"), "{}", r.text());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bad_domain_is_a_bad_request() {
    let m = served("tk-400", PublishDomains::Own).await;
    for d in ["not_a_dns_name", "-bad.example", "a..b", "UPPER.example", "has%20space.example", "a/b.example", "x"] {
        let r = get(&m, &format!("/takeover/{d}?format=json"), &[]).await;
        assert_eq!(r.status, 400, "{d}: {}", r.text());
        assert_eq!(r.json()["error"], "E_CANON_NAME", "{d}: {}", r.text());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mesh_and_openapi_documents() {
    let m = served("tk-docs", PublishDomains::Own).await;
    let doc = get(&m, "/.well-known/datumesh-mesh.json", &[]).await.json();
    // the function page has no entry in the mesh document, so the takeover page has none either
    assert!(doc["endpoints"].get("fn").is_none(), "{doc}");
    assert!(doc["endpoints"].get("takeover").is_none(), "{doc}");
    let api = get(&m, "/openapi.json", &[]).await.json();
    assert!(api["paths"].get("/takeover/{domain}").is_some(), "{}", api["paths"]);
}

// ---- the takeover page, rewritten (CONTRACT-site-round3 B3; replaces the D40 phrases of http 0.4.8) -------------------

/// The title: "{domain}'s values under its own name".
fn title(domain: &str) -> String {
    format!("{domain}'s values under its own name")
}

/// The opening.
fn opening(domain: &str, host: &str) -> String {
    format!(
        "This mesh ({host}) relays these values from {domain} unchanged, as a stand-in. {domain} remains their source."
    )
}

const H_FIRST: &str = "How the source's own name comes first";
const H_STOP: &str = "Stopping the relay";
const H_SERVE: &str = "Serving your own mesh";
const PUBLISH_DOCS: &str = "https://datumesh.com/docs/publish";

/// The sentences of "How the source's own name comes first", in order (the JSON `policy`).
fn first(domain: &str) -> [String; 4] {
    [
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

/// The sentences of "Stopping the relay", in order (the JSON `stopping`).
fn stopping(domain: &str) -> [String; 2] {
    [
        format!(
            "{domain} can ask for the relay to stop. A declaration it can put in its own key document for this is planned; \
             this mesh does not read one yet."
        ),
        format!("Until then, when {domain} asks, the feed is ended."),
    ]
}

/// Words the page must not say any more (the rebind sentence and the end of stored cells are removed).
const GONE: [&str; 4] = ["rebound", "rebind", "stops taking new stored cells", "new stored cells"];

/// Entities decoded, tags kept.
fn decoded(h: &str) -> String {
    let mut out = h.to_string();
    for (e, d) in [
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&#x27;", "'"),
        ("&apos;", "'"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&nbsp;", " "),
        ("&amp;", "&"),
    ] {
        out = out.replace(e, d);
    }
    out
}

/// The decoded text, with inline tags (a link around words of a sentence, code) removed without a space and the other
/// tags read as a space, white space folded: a sentence reads as one run whatever inline markup it carries.
fn tight(h: &str) -> String {
    const INLINE: [&str; 8] = ["a", "code", "strong", "em", "b", "i", "span", "q"];
    let mut out = String::new();
    let mut tag = String::new();
    let mut in_tag = false;
    for ch in h.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name: String = tag
                    .trim_start_matches('/')
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_lowercase();
                if !INLINE.contains(&name.as_str()) {
                    out.push(' ');
                }
            }
            _ if in_tag => tag.push(ch),
            _ => out.push(ch),
        }
    }
    decoded(&out).replace('\u{a0}', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The markup (entities decoded) from the heading whose text is `heading` to the next `<h2` or the end of `<main>`.
fn part<'a>(d: &'a str, heading: &str) -> &'a str {
    let at = d.find(heading).unwrap_or_else(|| panic!("no heading {heading:?} in: {d}"));
    let rest = &d[at..];
    let end =
        [rest.find("<h2"), rest.find("<H2"), rest.find("</main")].into_iter().flatten().min().unwrap_or(rest.len());
    &rest[..end]
}

/// The strings of a JSON array member.
fn strings(j: &Value, k: &str) -> Vec<String> {
    j[k].as_array()
        .unwrap_or_else(|| panic!("no {k} array: {j}"))
        .iter()
        .map(|v| v.as_str().unwrap_or_else(|| panic!("{k}: not a string: {v}")).to_string())
        .collect()
}

async fn served_with(label: &str, contact: Option<&str>) -> Served {
    let contact = contact.map(str::to_string);
    let cfg = datumesh_http::Config::from_vars(move |k| match k {
        "DATUMESH_HOST" => Some(HOST.to_string()),
        "DATUMESH_CONTACT" => contact.clone(),
        _ => None,
    })
    .unwrap();
    serve_lite(label, &lines(), Some(cfg), |_| {}).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_title_and_the_opening() {
    let m = served("tk-title", PublishDomains::Own).await;
    let h = get(&m, &format!("/takeover/{ECB}"), &[]).await.text();
    let d = decoded(&h);
    let t_at = d.find("<title>").unwrap_or_else(|| panic!("no title: {h}"));
    let t = tight(&d[t_at..t_at + d[t_at..].find("</title>").unwrap()]);
    assert!(t.contains(&title(ECB)), "the <title>: {t}");
    let h1_at = d.find("<h1").unwrap_or_else(|| panic!("no h1: {h}"));
    let h1 = tight(&d[h1_at..h1_at + d[h1_at..].find("</h1>").unwrap()]);
    assert!(h1.contains(&title(ECB)), "the <h1>: {h1}");
    assert!(!h1.to_lowercase().contains("taking over"), "the old title: {h1}");
    // the apostrophe is escaped in the markup (the title and every sentence go through the page's escaping)
    assert!(!h.contains(&format!("{ECB}'s")), "an unescaped apostrophe: {h}");
    assert!(tight(&h).contains(&opening(ECB, HOST)), "{}", tight(&h));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn how_the_source_s_own_name_comes_first() {
    let m = served("tk-first", PublishDomains::Own).await;
    let h = get(&m, &format!("/takeover/{ECB}"), &[]).await.text();
    let d = decoded(&h);
    let sec = part(&d, H_FIRST);
    let t = tight(sec);
    for s in first(ECB) {
        assert!(t.contains(&s), "missing from {H_FIRST:?}:\n want: {s}\n have: {t}");
    }
    // the first sentence is linked to the rule
    assert!(
        sec.contains("href=\"/catalog/rule\"") || sec.contains(&format!("href=\"https://{HOST}/catalog/rule\"")),
        "no link to /catalog/rule in {H_FIRST:?}: {sec}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_the_relay() {
    let m = served("tk-stop", PublishDomains::Own).await;
    let h = get(&m, &format!("/takeover/{ECB}"), &[]).await.text();
    let d = decoded(&h);
    let t = tight(part(&d, H_STOP));
    for s in stopping(ECB) {
        assert!(t.contains(&s), "missing from {H_STOP:?}:\n want: {s}\n have: {t}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serving_your_own_mesh() {
    for (label, domains, hosting) in [
        ("tk-serve-own", PublishDomains::Own, "POST /bundles does not admit it yet"),
        ("tk-serve-any", PublishDomains::Any, "POST /bundles is open to"),
    ] {
        let m = served(label, domains).await;
        let h = get(&m, &format!("/takeover/{ECB}"), &[]).await.text();
        let d = decoded(&h);
        let sec = part(&d, H_SERVE);
        assert!(sec.contains(&format!("href=\"{PUBLISH_DOCS}\"")), "{label}: no link to the Publish page: {sec}");
        let t = tight(sec);
        assert!(t.contains(hosting), "{label}: the hosting sentence: {t}");
        assert!(t.contains(&format!("https://{ECB}/.well-known/datumesh.json")), "{label}: the key document step: {t}");
        // the JSON steps carry the same: the key document, the Publish page, the hosting sentence
        let j = get(&m, &format!("/takeover/{ECB}?format=json"), &[]).await.json();
        let steps = strings(&j, "steps").join(" ");
        for want in [format!("https://{ECB}/.well-known/datumesh.json"), PUBLISH_DOCS.to_string(), hosting.to_string()]
        {
            assert!(steps.contains(&want), "{label}: {want} not in the JSON steps: {j}");
        }
        for s in strings(&j, "steps") {
            assert!(t.contains(&s), "{label}: the JSON step is not on the page:\n step: {s}\n page: {t}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_removed_sentences_are_gone() {
    let m = served("tk-gone", PublishDomains::Own).await;
    let h = tight(&get(&m, &format!("/takeover/{ECB}"), &[]).await.text()).to_lowercase();
    let j = get(&m, &format!("/takeover/{ECB}?format=json"), &[]).await.text().to_lowercase();
    for no in GONE {
        assert!(!h.contains(no), "{no} on the page: {h}");
        assert!(!j.contains(no), "{no} in the JSON: {j}");
    }
    // nor a withdrawal: the relay goes on as a mirror
    for no in ["withdraw", "retire", "deleted", "removed"] {
        assert!(!h.contains(no) && !j.contains(no), "{no}: {h}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_json_form_carries_the_same_sentences() {
    let m = served("tk-json-sentences", PublishDomains::Own).await;
    let j = get(&m, &format!("/takeover/{ECB}?format=json"), &[]).await.json();
    let page = tight(&get(&m, &format!("/takeover/{ECB}"), &[]).await.text());
    let policy = strings(&j, "policy");
    let stop = strings(&j, "stopping");
    // each member's sentences, in order (a bullet of two sentences may be one string or two)
    let joined = policy.join(" ");
    let mut from = 0;
    for s in first(ECB) {
        let at =
            joined[from..].find(&s).unwrap_or_else(|| panic!("policy lacks, in order:\n {s}\n policy: {policy:?}"));
        from += at + s.len();
    }
    let joined = stop.join(" ");
    let mut from = 0;
    for s in stopping(ECB) {
        let at =
            joined[from..].find(&s).unwrap_or_else(|| panic!("stopping lacks, in order:\n {s}\n stopping: {stop:?}"));
        from += at + s.len();
    }
    // and nothing else: every string is a sentence of the page
    for s in policy.iter().chain(stop.iter()) {
        assert!(s.trim().ends_with('.'), "{s}");
        assert!(page.contains(s.as_str()), "the JSON sentence is not on the page:\n {s}\n page: {page}");
    }
    assert!(j["stopping"].as_array().unwrap().iter().all(|v| !v.as_str().unwrap().contains('@')), "{j}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_contact_line_appears_only_when_a_contact_is_configured() {
    // not set: no contact member value, no address, no contact line
    for (label, contact) in [("tk-nocontact", None), ("tk-emptycontact", Some("  "))] {
        let m = served_with(label, contact).await;
        let j = get(&m, &format!("/takeover/{ECB}?format=json"), &[]).await.json();
        assert!(j["contact"].is_null() && j.get("contact").is_some(), "{label}: {j}");
        let t = tight(&get(&m, &format!("/takeover/{ECB}"), &[]).await.text());
        let lower = t.to_lowercase();
        assert!(!lower.contains('@') && !lower.contains("contact") && !lower.contains("mailto"), "{label}: {t}");
        assert!(!j.to_string().contains('@'), "{label}: {j}");
        for s in first(ECB).iter().chain(stopping(ECB).iter()) {
            assert!(t.contains(s.as_str()), "{label}: {s}: {t}");
        }
    }
    // set: the address is shown, in the page (in "Stopping the relay") and in the JSON
    let m = served_with("tk-contact", Some("takeover@datumesh.example")).await;
    let j = get(&m, &format!("/takeover/{ECB}?format=json"), &[]).await.json();
    assert_eq!(j["contact"], "takeover@datumesh.example", "{j}");
    let h = get(&m, &format!("/takeover/{ECB}"), &[]).await.text();
    let d = decoded(&h);
    let t = tight(part(&d, H_STOP));
    assert!(t.contains("takeover@datumesh.example"), "the contact under {H_STOP:?}: {t}");
}
