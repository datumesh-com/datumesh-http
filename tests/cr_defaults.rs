//! The classes round (CONTRACT-classes-round.md), K13: literal defaults (SPEC 7, 16.7). Failing tests: a request that leaves
//! an argument out is `E_ARG_MISSING`, and a body is `E_FORMULA_ARG_MISSING`.
//!
//! An argument declaration may carry `"default": <a lexical value of its type>` (`"1"` for a number with unit `1`), besides the
//! `"asof"` default. An unbound argument in a request takes it and `resolved` records a `default` resolution; in a body the
//! canonical AST holds the literal, so a body that leaves the argument out and one that writes it are one function; a default
//! that is not a valid value of the type is `E_SCHEMA` at define. The world is the shape C3's `source(rank)` needs: a function
//! `catalog/pick(subject, rank)` and a class `catalog/subject` whose method `source(rank)` have `rank` default `"1"`.

mod common;
#[path = "common/crworld.rs"]
mod crworld;

use common::lite::*;
use common::pub031::{function_line, snapshot, BEFORE};
use common::readpath::*;
use crworld::*;
use datumesh_core::Resolver as _;
use serde_json::json;

fn pick() -> String {
    format!("{HOST}/catalog/pick")
}
fn subject() -> String {
    format!("{HOST}/catalog/subject")
}

fn rank_arg(default: &str) -> String {
    format!(r#"{{"default":"{default}","type":"number","unit":"1","valid":false}}"#)
}

fn sig_with(extra_arg: &str) -> String {
    format!(r#"{{"args":{{"rank":{extra_arg},"subject":{STR}}},"order":["subject","rank"],"returns":"string"}}"#)
}

fn functions() -> Vec<String> {
    vec![
        function_line(&pick(), &sig_with(&rank_arg("1")), None),
        function_line(&format!("{}/pick_plain", pick().rsplit_once('/').unwrap().0), &sig_with(NUM_ARG), None),
        function_line(&format!("{}/source", subject()), &sig_with(&rank_arg("1")), None),
    ]
}

const NUM_ARG: &str = r#"{"type":"number","unit":"1","valid":false}"#;

fn cells(f: &str) -> Vec<String> {
    let c = |rank: &str, v: &str| cell_line(f, &[("rank", "number", rank), ("subject", "string", "s1")], ("string", v));
    vec![c("1", "ns/alpha"), c("2", "ns/beta")]
}

async fn world(label: &str, formulas: bool) -> Served {
    let lite = Lite::new(label);
    publish(&lite, "functions.jsonl", &functions(), BEFORE).unwrap_or_else(|e| panic!("functions (a literal default): {e}"));
    let class = crate::common::world::class_json(
        &format!(r#"{{"subject":{NONE_STR}}}"#),
        &["subject"],
        &[("source", &fn_hash(&lite, &format!("{}/source", subject())))],
        HOST,
        "",
    );
    let mut v = vec![crate::common::pub031::class_line(&subject(), &class, None)];
    v.extend(cells(&pick()));
    v.extend(cells(&format!("{}/source", subject())));
    publish(&lite, "classes.jsonl", &v, NOW).unwrap_or_else(|e| panic!("classes and cells: {e}"));
    if formulas {
        let sig = format!(r#"{{"args":{{"subject":{STR}}},"order":["subject"],"returns":"string"}}"#);
        let a = formula_line(&format!("{HOST}/catalog/top_a"), &format!("{}(subject: subject)", pick()), &sig);
        let b = formula_line(&format!("{HOST}/catalog/top_b"), &format!("{}(subject: subject, rank: 1)", pick()), &sig);
        publish(&lite, "formulas.jsonl", &[a, b], NOW).unwrap_or_else(|e| panic!("a body that leaves the argument out: {e}"));
    }
    build_served(lite, None, |_| {}).await
}

/// A request with the argument left out takes the default: positional and labelled, the cell is the one with `rank: 1`, and
/// `resolved` records a `default` resolution (`from` the argument, `to` its lexical value) and the call with it written out. A
/// request that gives the argument takes its own value and resolves nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k13_a_request_with_the_argument_left_out() {
    let m = world("cr-k13-request", false).await;
    let p = pick();
    let plain = ask(&m, &format!("/cell/{p}?rank=1&subject=s1")).await;
    assert_eq!((plain.outcome(), plain.value()), ("ok", Some("ns/alpha")), "{}", plain.text);
    for call in [format!("{p}(s1)"), format!("{p}(subject: s1)"), format!("{p}(\"s1\")")] {
        let a = ask(&m, &path(&call, None)).await;
        assert_eq!((a.status, a.outcome(), a.value()), (200, "ok", Some("ns/alpha")), "{call}: {}", a.text);
        assert_eq!(a.cell_id(), plain.cell_id(), "{call}: the default is bound in the cell");
        assert_eq!(a.resolutions(), vec![("rank".to_string(), "default".to_string(), "1".to_string())], "{call}");
        assert_eq!(a.v["resolved"]["call"], format!(r#"{p}(rank: 1, subject: "s1")"#), "{call}");
    }
    // the value given is the value read, with no resolution
    let two = ask(&m, &path(&format!("{p}(s1, 2)"), None)).await;
    assert_eq!((two.outcome(), two.value()), ("ok", Some("ns/beta")), "{}", two.text);
    assert!(two.v.get("resolved").is_none(), "nothing resolved: {}", two.text);
    // the cell URI form has no short form: the argument is missing there, as before
    let uri = ask(&m, &format!("/cell/{p}?subject=s1")).await;
    assert_eq!((uri.status, uri.error()), (400, Some("E_ARG_MISSING")), "{}", uri.text);
    // an argument with no default is still missing
    let pp = ask(&m, &path(&format!("{HOST}/catalog/pick_plain(s1)"), None)).await;
    assert_eq!((pp.status, pp.error()), (400, Some("E_ARG_MISSING")), "{}", pp.text);
    // the receipt verifies: the cell is the plain one
    let keys = {
        let mut k = datumesh_core::Keys::new();
        k.insert_response(HOST, 200, Some(&datumesh_http::keydoc::with_key(HOST, None, &m.lite.key, K_FROM).unwrap()));
        k
    };
    let a = ask(&m, &path(&format!("{p}(s1)"), None)).await;
    let req = datumesh_core::Request::new(&plain.cell_id(), None).unwrap();
    datumesh_core::check_answer(a.status, a.text.as_bytes(), &keys, &req).unwrap_or_else(|e| panic!("{e}: {}", a.text));
}

/// The method form: `subject(s1).source()` takes the default; `resolved` lists the method and then the default.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k13_a_method_call_with_the_argument_left_out() {
    let m = world("cr-k13-method", false).await;
    let (s, src) = (subject(), format!("{}/source", subject()));
    let a = ask(&m, &path(&format!("{s}(s1).source()"), None)).await;
    assert_eq!((a.status, a.outcome(), a.value()), (200, "ok", Some("ns/alpha")), "{}", a.text);
    assert_eq!(
        a.resolutions(),
        vec![
            (format!("{s}(...).source"), "method".to_string(), src.clone()),
            ("rank".to_string(), "default".to_string(), "1".to_string()),
        ],
        "{}",
        a.text
    );
    assert_eq!(a.v["resolved"]["call"], format!(r#"{src}(rank: 1, subject: "s1")"#));
    let plain = ask(&m, &format!("/cell/{src}?rank=1&subject=s1")).await;
    assert_eq!(a.cell_id(), plain.cell_id());
    let two = ask(&m, &path(&format!("{s}(s1).source(2)"), None)).await;
    assert_eq!((two.outcome(), two.value()), ("ok", Some("ns/beta")), "{}", two.text);
    assert_eq!(two.resolution_kinds(), ["method"], "{}", two.text);
}

/// A body that leaves the argument out has the same canonical AST as one that writes the literal: here the two formulas are one
/// function (one descriptor, so one hash, the same body and inputs), and each evaluates through the default.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k13_a_body_with_the_argument_left_out_is_the_literal() {
    let m = world("cr-k13-body", true).await;
    let (a, b) = (format!("{HOST}/catalog/top_a"), format!("{HOST}/catalog/top_b"));
    let (ha, hb) = (fn_hash(&m.lite, &a), fn_hash(&m.lite, &b));
    assert_eq!(ha, hb, "one body: the same descriptor");
    let d = |h: &str| String::from_utf8(m.lite.store.descriptor(h).unwrap().into_owned()).unwrap();
    assert_eq!(d(&ha), d(&hb));
    for f in [&a, &b] {
        let v = ask(&m, &format!("/cell/{f}?subject=s1")).await;
        assert_eq!((v.outcome(), v.value()), ("ok", Some("ns/alpha")), "{f}: {}", v.text);
    }
}

/// The default is part of the descriptor, so it is hashed: `pick` (default `"1"`) and `pick_plain` (none) differ, and so do
/// two defaults.
#[test]
fn k13_the_default_is_hashed() {
    use common::pub031::descriptor_hash;
    let d = |a: &str| descriptor_hash(&common::pub031::stored_descriptor(&sig_with(a)));
    assert_ne!(d(&rank_arg("1")), d(NUM_ARG));
    assert_ne!(d(&rank_arg("1")), d(&rank_arg("2")));
    // and a function with a default publishes (a stored one, and a formula over it)
    let lite = Lite::new("cr-k13-hash");
    publish(&lite, "f.jsonl", &functions(), NOW).unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(fn_hash(&lite, &pick()), fn_hash(&lite, &format!("{HOST}/catalog/pick_plain")));
}

/// A default that is not a valid value of the argument's type is `E_SCHEMA` at define (and nothing is written): a word for a
/// number, a number that is not in lexical form, a date that is no date, a boolean that is not `true` or `false`, a JSON number
/// where a string is written; a valid one is accepted for each type. (Core 0.3.12 checks the descriptor, so this one holds today:
/// it keeps the check from being lost on the way through publish.)
#[test]
fn k13_a_default_of_the_wrong_type_is_refused() {
    let with = |arg: &str, ty: &str| {
        format!(r#"{{"args":{{"a":{arg},"k":{STR}}},"order":["k","a"],"returns":"{ty}"}}"#)
    };
    let ok = [
        (r#"{"default":"1","type":"number","unit":"1","valid":false}"#, "string"),
        (r#"{"default":"-2.5","type":"number","unit":"1","valid":false}"#, "string"),
        (r#"{"default":"EUR","type":"string","valid":false}"#, "string"),
        (r#"{"default":"2026-03-16","type":"date","valid":false}"#, "string"),
        (r#"{"default":"true","type":"boolean","valid":false}"#, "string"),
        (r#"{"default":"asof","type":"date","valid":true}"#, "string"),
    ];
    for (i, (arg, ty)) in ok.iter().enumerate() {
        let lite = Lite::new(&format!("cr-k13-ok-{i}"));
        let f = function_line(&format!("{HOST}/catalog/d{i}"), &with(arg, ty), None);
        publish(&lite, "f.jsonl", &[f], NOW).unwrap_or_else(|e| panic!("{arg}: {e}"));
    }
    let bad = [
        (r#"{"default":"abc","type":"number","unit":"1","valid":false}"#, "E_SCHEMA"),
        (r#"{"default":"1e3","type":"number","unit":"1","valid":false}"#, "E_SCHEMA"),
        (r#"{"default":"01","type":"number","unit":"1","valid":false}"#, "E_SCHEMA"),
        (r#"{"default":1,"type":"number","unit":"1","valid":false}"#, "E_SCHEMA"),
        (r#"{"default":"2026-13-45","type":"date","valid":false}"#, "E_SCHEMA"),
        (r#"{"default":"yes","type":"boolean","valid":false}"#, "E_SCHEMA"),
        (r#"{"default":true,"type":"boolean","valid":false}"#, "E_SCHEMA"),
        // "asof" stays for a valid-time date or instant only
        (r#"{"default":"asof","type":"number","unit":"1","valid":false}"#, "E_CANON_TYPE"),
    ];
    for (i, (arg, code)) in bad.iter().enumerate() {
        let lite = Lite::new(&format!("cr-k13-bad-{i}"));
        let before = snapshot(&lite);
        let f = function_line(&format!("{HOST}/catalog/b{i}"), &with(arg, "string"), None);
        let e = publish(&lite, "f.jsonl", &[f], NOW).expect_err(arg);
        assert_eq!(e.line, 1, "{arg}: {e}");
        assert!(e.detail.contains(code), "{arg}: {e}");
        assert_eq!(snapshot(&lite), before, "{arg}: nothing written");
    }
    // a formula's signature goes through the same check
    let lite = Lite::new("cr-k13-formula");
    let f = formula_line(&format!("{HOST}/catalog/fbad"), "k", &with(r#"{"default":"abc","type":"number","unit":"1","valid":false}"#, "string"));
    let e = publish(&lite, "f.jsonl", &[f], NOW).expect_err("a formula with a bad default");
    assert!(e.detail.contains("E_SCHEMA"), "{e}");
    let _ = json!(null);
}
