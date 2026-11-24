//! http 0.5.4, CONTRACT-next-patch item 15: **the function page tells an empty-slice test apart from its twin.**
//! Written first; FAIL until the page shows a case's `inputs`.
//!
//! The Part 2 shape: `bank_rate_change` has two tests with the same call, one of them with its slice input given as
//! empty (`{"<slice call>": {"members": {}}}`). On the page they read as the same line twice. Here the same shape on the
//! calculus fixture: `avg_log(scale: 1, site: "S")` without inputs (S's one stored log day, 7) and with the slice
//! `log(site: "S", day: *)` given as empty (`E_FORMULA_EMPTY`); and `avg_log(scale: 1, site: "N")` without inputs (3)
//! and with two members given (7).
//!
//! What these tests specify (the implementer follows):
//! * HTML, `<section id="tests">`, "Each test": a case with `inputs` shows them beside the call. A slice given as
//!   empty reads "with `<slice call>` given as empty"; a slice given with members reads "with N members given". A case
//!   without inputs says neither. So two cases with the same call text are two distinct lines;
//! * JSON (`?format=json`), `tests.cases[]`: each case carries its `inputs` as the test gave them (absent, `null` or
//!   `{}` for a case without).

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use calculus::*;
use common::readpath::*;
use publish4::get;
use serde_json::{json, Value};

fn slice(c: &Calc, site: &str) -> String {
    format!("{}(site: \"{site}\", day: *)", c.log)
}

fn cell(c: &Calc, site: &str, day: &str) -> String {
    format!("{}(site: \"{site}\", day: {day})", c.log)
}

fn empty_input(c: &Calc) -> Value {
    json!({ slice(c, "S"): {"members": {}} })
}

fn two_members_input(c: &Calc) -> Value {
    json!({ slice(c, "N"): {"members": {cell(c, "N", "2026-03-03"): num("9"), cell(c, "N", "2026-03-04"): num("5")}} })
}

/// The calculus data with a site `S` of one log day, `avg_log`, and its tests: the public rule's three (positive,
/// negative scale, the empty slice by `{"members": {}}`), the twin of the empty one without inputs, and a call on N
/// with and without two members given.
fn lines(c: &Calc) -> Vec<String> {
    let f = &c.avg_log;
    let mut v = data_lines(c);
    v.push(log_cell(&c.log, "S", "2026-03-01", "7"));
    v.push(avg_log_line(c));
    v.push(test_line(f, &avg_call(c, "N", "2"), None, None, num("6")));
    v.push(test_line(f, &avg_call(c, "N", "-1"), None, None, num("-3")));
    v.push(test_line(f, &avg_call(c, "S", "1"), Some(empty_input(c)), None, fault("E_FORMULA_EMPTY")));
    v.push(test_line(f, &avg_call(c, "S", "1"), None, None, num("7")));
    v.push(test_line(f, &avg_call(c, "N", "1"), Some(two_members_input(c)), None, num("7")));
    v.push(test_line(f, &avg_call(c, "N", "1"), None, None, num("3")));
    v
}

async fn served(label: &str) -> Served {
    serve_lite(label, &lines(&calc()), None, |_| {}).await
}

/// The decoded text of a piece of HTML: tags removed, entities decoded, white space collapsed.
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
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The part of the page from `id="<id>"` to the next section (or the end).
fn section(h: &str, id: &str) -> String {
    let at = h.find(&format!("id=\"{id}\"")).unwrap_or_else(|| panic!("no section {id} in the page"));
    let rest = &h[at..];
    let end = rest[1..].find("<section").map(|i| i + 1).unwrap_or(rest.len());
    rest[..end].to_string()
}

/// The text of each list item of the tests section that holds `call`.
fn lines_of(tests_html: &str, call: &str) -> Vec<String> {
    tests_html.split("<li").skip(1).map(|li| plain(&format!("<li{li}"))).filter(|t| t.contains(call)).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_page_tells_an_empty_slice_test_from_its_twin() {
    let c = calc();
    let m = served("h054-fnp-html").await;
    let r = get(&m, &format!("/fn/{}", c.avg_log), &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let tests = section(&r.text(), "tests");

    // the twins on S: two lines with the call, and not the same line
    let s = lines_of(&tests, &avg_call(&c, "S", "1"));
    assert_eq!(s.len(), 2, "two cases with the call on S: {s:#?}");
    assert_ne!(s[0], s[1], "the two cases on S read the same: {s:#?}");
    let (empty, plain_twin): (Vec<&String>, Vec<&String>) = s.iter().partition(|t| t.contains("E_FORMULA_EMPTY"));
    assert_eq!((empty.len(), plain_twin.len()), (1, 1), "{s:#?}");
    let want = format!("with {} given as empty", slice(&c, "S"));
    assert!(empty[0].contains(&want), "the empty-slice case says `{want}`: {}", empty[0]);
    assert!(!plain_twin[0].contains("given"), "the case without inputs says nothing of inputs: {}", plain_twin[0]);

    // the twins on N: one with two members given
    let n = lines_of(&tests, &avg_call(&c, "N", "1"));
    assert_eq!(n.len(), 2, "two cases with the call on N: {n:#?}");
    assert_ne!(n[0], n[1], "the two cases on N read the same: {n:#?}");
    let given: Vec<&String> = n.iter().filter(|t| t.contains("2 members given")).collect();
    assert_eq!(given.len(), 1, "exactly one case on N says `with 2 members given`: {n:#?}");
    assert!(!given[0].contains("given as empty"), "{}", given[0]);
    let other = n.iter().find(|t| !t.contains("2 members given")).unwrap();
    assert!(!other.contains("given"), "the case without inputs says nothing of inputs: {other}");

    // every case is still on the page, each its own line
    for call in [avg_call(&c, "N", "2"), avg_call(&c, "N", "-1")] {
        assert_eq!(lines_of(&tests, &call).len(), 1, "{call}: {tests}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_json_carries_each_case_inputs() {
    let c = calc();
    let m = served("h054-fnp-json").await;
    let r = get(&m, &format!("/fn/{}?format=json", c.avg_log), &[]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let j = r.json();
    let cases = j["tests"]["cases"].as_array().unwrap_or_else(|| panic!("no cases: {j}"));
    assert_eq!(cases.len(), 6, "{j}");
    let find = |call: &str, expect_has: &str| -> &Value {
        let hits: Vec<&Value> = cases
            .iter()
            .filter(|x| x["call"] == call && x["expect"].as_str().is_some_and(|e| e.contains(expect_has)))
            .collect();
        assert_eq!(hits.len(), 1, "{call} expecting {expect_has}: {}", j["tests"]);
        hits[0]
    };
    let none = |x: &Value| x.get("inputs").is_none_or(|i| i.is_null() || i.as_object().is_some_and(|o| o.is_empty()));

    let empty = find(&avg_call(&c, "S", "1"), "E_FORMULA_EMPTY");
    assert_eq!(empty.get("inputs"), Some(&empty_input(&c)), "the empty-slice case carries its inputs: {empty}");
    let twin = find(&avg_call(&c, "S", "1"), "7");
    assert!(none(twin), "the case without inputs carries none: {twin}");

    let given = find(&avg_call(&c, "N", "1"), "7");
    assert_eq!(given.get("inputs"), Some(&two_members_input(&c)), "the case carries its members: {given}");
    let twin = find(&avg_call(&c, "N", "1"), "3");
    assert!(none(twin), "the case without inputs carries none: {twin}");

    for (call, e) in [(avg_call(&c, "N", "2"), "6"), (avg_call(&c, "N", "-1"), "-3")] {
        let x = find(&call, e);
        assert!(none(x), "{x}");
    }
    // the two cases with one call are two distinct entries
    assert_ne!(find(&avg_call(&c, "S", "1"), "E_FORMULA_EMPTY"), find(&avg_call(&c, "S", "1"), "7"));
}
