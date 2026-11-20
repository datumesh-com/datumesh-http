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
    let steps = j["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2, "{j}");
    assert!(steps[0].as_str().unwrap().contains(&format!("https://{ECB}/.well-known/datumesh.json")), "{j}");
    assert!(steps[1].as_str().unwrap().contains(&format!("{ECB}/...")), "{j}");
    let mut keys: Vec<&String> = j.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["contact", "domain", "functions", "host", "policy", "steps"]);
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

// ---- the takeover policy (QUESTIONS-build2 D40, http 0.4.8) -------------------------------------------

/// The policy's sentences, as phrases the page must contain (the wording around them is the implementer's;
/// the JSON `policy` is an array of the sentences, the HTML shows the same text).
const POLICY: [&str; 5] = [
    "no permission",
    "key document on its own domain",
    "stops taking new stored cells",
    "rebound to read the source's cells",
    "history stays as the mesh's signed statements",
];

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
async fn the_page_states_the_takeover_policy() {
    let m = served("tk-policy", PublishDomains::Own).await;
    let t = plain(&get(&m, &format!("/takeover/{ECB}"), &[]).await.text()).to_lowercase();
    for phrase in POLICY {
        assert!(t.contains(phrase), "{phrase} missing: {t}");
    }
    // the JSON form carries the same sentences
    let j = get(&m, &format!("/takeover/{ECB}?format=json"), &[]).await.json();
    let sentences: Vec<&str> = j["policy"]
        .as_array()
        .unwrap_or_else(|| panic!("no policy array: {j}"))
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(sentences.len() >= 4, "{j}");
    assert!(sentences.iter().all(|s| s.trim().ends_with('.') && s.len() > 20), "{sentences:?}");
    let all = sentences.join(" ").to_lowercase();
    for phrase in POLICY {
        assert!(all.contains(phrase), "{phrase} missing from the JSON policy: {j}");
    }
    // the policy does not say the relay is withdrawn: it is rebound, its history kept
    for no in ["withdraw", "retire", "deleted", "removed"] {
        assert!(!all.contains(no) && !t.contains(no), "{no}: {t}");
    }
    // the other statements are as before
    assert!(t.contains(&format!("{ECB} remains their source")), "{t}");
    assert_eq!(j["steps"].as_array().unwrap().len(), 2, "{j}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_contact_line_appears_only_when_a_contact_is_configured() {
    // not set: no contact member value, no address, no contact line
    for (label, contact) in [("tk-nocontact", None), ("tk-emptycontact", Some("  "))] {
        let m = served_with(label, contact).await;
        let j = get(&m, &format!("/takeover/{ECB}?format=json"), &[]).await.json();
        assert!(j["contact"].is_null() && j.get("contact").is_some(), "{label}: {j}");
        let t = plain(&get(&m, &format!("/takeover/{ECB}"), &[]).await.text()).to_lowercase();
        assert!(!t.contains('@') && !t.contains("contact"), "{label}: {t}");
        for phrase in POLICY {
            assert!(t.contains(phrase), "{label}: {phrase}: {t}");
        }
    }
    // set: the address is shown, in the page and in the JSON
    let m = served_with("tk-contact", Some("takeover@datumesh.example")).await;
    let j = get(&m, &format!("/takeover/{ECB}?format=json"), &[]).await.json();
    assert_eq!(j["contact"], "takeover@datumesh.example", "{j}");
    let t = plain(&get(&m, &format!("/takeover/{ECB}"), &[]).await.text());
    assert!(t.contains("takeover@datumesh.example"), "{t}");
}
