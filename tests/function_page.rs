//! Build 2 Part 4, P2: the function page, `GET /fn/<function name>` (CONTRACT-build2-part4-publish).
//!
//! What these tests specify (the implementer follows):
//! * HTML by default, `text/html; charset=utf-8`, server-rendered, no `<script>` and no script URL,
//!   publisher text escaped. `<html lang="en">`, an `<h1>` and `<title>` naming the function, and one
//!   `<section id="...">` per part: `description`, `signature`, `body`, `tests`, `lineage`,
//!   `examples`, `licence`. The tests below read the text of a section (tags removed, entities decoded);
//! * `description`: the catalogue description and a line per argument; `signature`: the arguments with
//!   types and units, `returns`, the `fn_hash`, the binding history (hash and since); `body`: the
//!   formula of a computed function; `tests`: `6 tests`, `2 of 2 branches`, the derived cases (each
//!   covered or not), `counterexamples`, `last run` and its result, each test's call and expected value
//!   or fault; `lineage`: inputs and dependants as links to their pages (`/fn/<name>`), for a relay the
//!   source, "relayed by <mesh host>" and the takeover page (see `relayed_source.rs`); `examples`: the
//!   publisher's example calls, linked to `/cell/...`;
//! * JSON, by `Accept: application/json` or `?format=json` (the same body): the members `name`,
//!   `fn_hash`, `description` (string), `args` (argument -> catalogue line), `signature` (`args`,
//!   `order`, `returns`, `fn_hash`, `bindings`: [{`fn_hash`, `since`}]), `body` (string, absent or null for
//!   a stored function), `tests` (`count`, `branches`, `covered`, `counterexamples`, `derived`: [{`case`,
//!   `covered`}], `cases`: [{`call`, `expect`}], `last_run`: {`result`, ...}), `lineage` (`inputs` and
//!   `dependants`: [{`name`, `page`}], `source` for a relay), `examples`: [{`call`, `expected`, `cell`}],
//!   `licence`;
//! * an unknown name is `404` with the not-covered shape (`outcome: not_covered`, `E_NAME_UNBOUND`);
//! * `describe` of a function answers `page`: `https://<mesh host>/fn/<name>`.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use calculus::*;
use common::lite::*;
use common::pub031::function_line;
use common::readpath::*;
use common::world::call_path;
use publish4::get;
use serde_json::{json, Value};

const PREFIX: &str = "mesh-test.datumesh.com/scratch_b4/fn";

const DESC: &str = r#"Divides <script>alert("xss")</script> the reading & <img src=x onerror=alert(1)> by units"#;
const ARG_SITE: &str = r#""><svg onload=alert(2)> the measuring site"#;
const ARG_UNITS: &str = "units to divide by";
const LABEL: &str = "<b>Per unit</b>";

fn c() -> Calc {
    Calc::new(PREFIX)
}

fn lines(c: &Calc) -> Vec<String> {
    let mut v = data_lines(c);
    v.push(per_unit_line(c, None));
    v.extend(per_unit_tests(c));
    let ex = json!([{"call": format!("{}(site: \"N\", units: 2)", c.per_unit), "expected": {"type": "number", "value": "5"}}]);
    v.push(
        json!({"catalog": c.per_unit, "kind": "function", "label": LABEL, "description": DESC,
            "args": {"site": ARG_SITE, "units": ARG_UNITS}, "examples": ex, "dictionary": {}})
        .to_string(),
    );
    v.push(catalog_line(
        &c.reading,
        "function",
        "Reading",
        "The raw reading at a site.",
        json!({"site": "the site"}),
        json!({}),
    ));
    v
}

async fn served(label: &str) -> Served {
    serve_lite(label, &lines(&c()), None, |_| {}).await
}

fn path(name: &str) -> String {
    format!("/fn/{name}")
}

fn html_of(r: &publish4::Reply4) -> String {
    assert_eq!(r.status, 200, "{}", r.text());
    r.text()
}

/// The decoded text of a piece of HTML: tags removed, entities decoded.
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
    for (e, d) in [
        ("&quot;", "\""),
        ("&#34;", "\""),
        ("&#x22;", "\""),
        ("&#39;", "'"),
        ("&#x27;", "'"),
        ("&apos;", "'"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&amp;", "&"),
    ] {
        out = out.replace(e, d);
    }
    out
}

/// The part of the page from `id="<id>"` to the next section (or the end).
fn section(h: &str, id: &str) -> String {
    let at = h.find(&format!("id=\"{id}\"")).unwrap_or_else(|| panic!("no section {id} in the page"));
    let rest = &h[at..];
    let end = rest[1..].find("<section").map(|i| i + 1).unwrap_or(rest.len());
    rest[..end].to_string()
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_page_for_people() {
    let c = c();
    let m = served("fp-html").await;
    let r = get(&m, &path(&c.per_unit), &[]).await;
    let html = html_of(&r);
    assert!(r.header("content-type").unwrap().starts_with("text/html"), "{:?}", r.header("content-type"));
    assert!(r.header("content-type").unwrap().to_lowercase().contains("utf-8"));
    let lower = html.to_lowercase();
    assert!(lower.contains("<html lang="), "{html}");
    assert!(lower.contains("<h1") && lower.contains("<title"), "{html}");
    assert!(plain(&html).contains(&c.per_unit));
    let hash = fn_hash(&m, &c.per_unit);

    // description: the catalogue's, and a line per argument
    let d = norm(&plain(&section(&html, "description")));
    assert!(d.contains(&norm(DESC)), "{d}");
    assert!(d.contains(&norm(ARG_SITE)) && d.contains(ARG_UNITS), "{d}");

    // signature: types and units, returns, the hash, the binding history
    let sg = norm(&plain(&section(&html, "signature")));
    for want in ["site", "units", "number", "string", &hash, "since", &lite_bound(&m, &c.per_unit)] {
        assert!(sg.contains(want), "{want}: {sg}");
    }

    // body: the canonical formula of a computed function
    let b = norm(&plain(&section(&html, "body")));
    assert!(b.contains(&c.reading) && b.contains("units"), "{b}");

    // tests: count, branches, derived cases, each test, the last run
    let t = norm(&plain(&section(&html, "tests")));
    for want in ["6 tests", "2 of 2 branches", "zero(units)", "negative(units)", "E_FORMULA_ABSENT", "last run"] {
        assert!(t.to_lowercase().contains(&want.to_lowercase()), "{want}: {t}");
    }
    for call in [pu_call(&c, "N", "2"), pu_call(&c, "N", "0"), pu_call(&c, "N", "-3"), pu_call(&c, "ZZ", "2")] {
        assert!(t.contains(&call), "{call}: {t}");
    }

    // lineage: the input, linked to its own page
    let l = section(&html, "lineage");
    assert!(l.contains("href=\"") && l.contains(&format!("/fn/{}", c.reading)), "{l}");

    // examples: linked to /cell/
    let e = section(&html, "examples");
    assert!(
        e.contains("/cell/") && norm(&plain(&e)).contains(&format!("{}(site: \"N\", units: 2)", c.per_unit)),
        "{e}"
    );

    // the licence and the declarations have their section
    let _ = section(&html, "licence");
}

fn lite_bound(m: &Served, name: &str) -> String {
    m.lite.bound_tx(name).unwrap()
}

/// A page of a stored function: no body, tests of none; and the dependants are listed, linked.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stored_function_and_its_dependants() {
    let c = c();
    let m = served("fp-stored").await;
    let html = html_of(&get(&m, &path(&c.reading), &[]).await);
    assert!(norm(&plain(&section(&html, "description"))).contains("The raw reading at a site."));
    let l = section(&html, "lineage");
    assert!(l.contains(&format!("/fn/{}", c.per_unit)), "dependant not linked: {l}");
    let j = get(&m, &path(&c.reading), &[("accept", "application/json")]).await.json();
    assert!(j.get("body").is_none_or(Value::is_null), "{j}");
    assert_eq!(
        j["lineage"]["dependants"].as_array().unwrap().iter().map(|d| d["name"].clone()).collect::<Vec<_>>(),
        vec![json!(c.per_unit)],
        "{j}"
    );
}

/// No script, no script URL, no handler: and the publisher's text is escaped, not interpreted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_script_and_publisher_text_is_escaped() {
    let c = c();
    let m = served("fp-xss").await;
    for name in [&c.per_unit, &c.reading] {
        let html = html_of(&get(&m, &path(name), &[]).await);
        let lower = html.to_lowercase();
        for bad in ["<script", "javascript:", "<img", "<svg", "<b>per unit", "<iframe", "<object", "<embed"] {
            assert!(!lower.contains(bad), "{bad} in the page of {name}: {html}");
        }
        assert!(!lower.contains(" onload=") && !lower.contains(" onclick=") && !lower.contains(" onerror="), "{html}");
    }
    let html = html_of(&get(&m, &path(&c.per_unit), &[]).await);
    assert!(html.contains("&lt;script"), "the publisher's markup is shown as text: {html}");
    assert!(plain(&html).contains("<script>alert(\"xss\")</script>"));
    assert!(plain(&html).contains("<b>Per unit</b>"), "the label is shown as text");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_json_form_is_the_same_data() {
    let c = c();
    let m = served("fp-json").await;
    let by_accept = get(&m, &path(&c.per_unit), &[("accept", "application/json")]).await;
    let by_query = get(&m, &format!("{}?format=json", path(&c.per_unit)), &[]).await;
    assert_eq!(by_accept.status, 200, "{}", by_accept.text());
    assert!(by_accept.header("content-type").unwrap().starts_with("application/json"));
    assert_eq!(by_accept.body, by_query.body);
    let j = by_accept.json();
    let hash = fn_hash(&m, &c.per_unit);
    for k in ["name", "fn_hash", "description", "args", "signature", "body", "tests", "lineage", "examples", "licence"]
    {
        assert!(j.get(k).is_some(), "no {k}: {j}");
    }
    assert_eq!((j["name"].as_str(), j["fn_hash"].as_str()), (Some(c.per_unit.as_str()), Some(hash.as_str())));
    assert_eq!(j["description"], DESC);
    assert_eq!(j["args"], json!({"site": ARG_SITE, "units": ARG_UNITS}));
    assert_eq!(j["signature"]["fn_hash"], hash.as_str());
    assert_eq!(j["signature"]["args"]["units"]["type"], "number");
    assert_eq!(j["signature"]["returns"]["unit"], "1");
    assert_eq!(j["signature"]["order"], json!(["site", "units"]));
    assert_eq!(j["signature"]["bindings"], json!([{"fn_hash": hash, "since": lite_bound(&m, &c.per_unit)}]), "{j}");
    assert!(j["body"].as_str().unwrap().contains(&c.reading), "{j}");
    let t = &j["tests"];
    assert_eq!(
        (t["count"].as_u64(), t["branches"].as_u64(), t["covered"].as_u64(), t["counterexamples"].as_u64()),
        (Some(6), Some(2), Some(2), Some(0)),
        "{t}"
    );
    let derived = t["derived"].as_array().unwrap();
    assert_eq!(derived.len(), 3, "{t}");
    for case in ["zero(units)", "negative(units)", &format!("absent({})", c.reading)] {
        let d = derived.iter().find(|d| d["case"] == case).unwrap_or_else(|| panic!("{case}: {t}"));
        assert_eq!(d["covered"], true, "{d}");
    }
    let cases = t["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 6);
    assert!(cases.iter().all(|x| x["call"].is_string() && x.get("expect").is_some()), "{t}");
    assert!(t["last_run"]["result"].is_string(), "{t}");
    assert_eq!(
        j["lineage"]["inputs"],
        json!([{"name": c.reading, "page": format!("https://{HOST}/fn/{}", c.reading)}]),
        "{j}"
    );
    assert_eq!(j["lineage"]["dependants"], json!([]));
    let ex = j["examples"].as_array().unwrap();
    assert_eq!(ex.len(), 1);
    assert_eq!(ex[0]["call"], format!("{}(site: \"N\", units: 2)", c.per_unit));
    assert!(ex[0]["cell"].as_str().unwrap().contains("/cell/"), "{j}");
    // the page of people shows the same data
    let html = html_of(&get(&m, &path(&c.per_unit), &[]).await);
    let text = norm(&plain(&html));
    assert!(text.contains(&hash) && text.contains(&norm(DESC)));
    for x in cases {
        assert!(text.contains(x["call"].as_str().unwrap()), "{x}");
    }
}

/// An unknown name: 404 with the not-covered shape (as `/cell` answers an unbound name).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unknown_function_is_not_covered() {
    let m = served("fp-404").await;
    for name in [format!("{PREFIX}/nothing"), "other.example/b4/nothing".to_string()] {
        for (p, h) in
            [(path(&name), vec![("accept", "application/json")]), (format!("{}?format=json", path(&name)), vec![])]
        {
            let r = get(&m, &p, &h).await;
            assert_eq!(r.status, 404, "{p}: {}", r.text());
            let j = r.json();
            assert_eq!(j["outcome"], "not_covered", "{j}");
            assert_eq!(j["error"], "E_NAME_UNBOUND", "{j}");
        }
        let r = get(&m, &path(&name), &[]).await;
        assert_eq!(r.status, 404);
        assert!(r.text().contains("not_covered"), "{}", r.text());
    }
}

/// `describe` of a function links to its page.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describe_links_to_the_page() {
    let c = c();
    let m = served("fp-describe").await;
    for name in [&c.per_unit, &c.reading] {
        // The page is the `Link` header of the answer (rel="describedby"), not a member of it: the
        // answer is the spec's (16.8), which core checks strictly.
        let r = get(&m, &format!("/describe/{}", call_path(name)), &[]).await;
        assert_eq!(r.status, 200, "{}", r.text());
        assert_eq!(r.json()["kind"], "function", "{}", r.text());
        assert!(r.json().get("page").is_none(), "page is not a member: {}", r.text());
        assert_eq!(r.header("link"), Some(format!("<https://{HOST}/fn/{name}>; rel=\"describedby\"")), "{}", r.text());
    }
}

#[allow(dead_code)]
fn unused() -> String {
    function_line("", "", None)
}
