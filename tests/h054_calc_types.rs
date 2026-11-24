//! http 0.5.4, CONTRACT-next-patch item 33 (added by the user): **`/x/calc` takes its result type from the compiled
//! body.** Written first; FAIL until then: today the body is compiled against a placeholder signature that returns a
//! number (`src/calc.rs`), so any other result is refused with `E_FORMULA_RETURN_TYPE`.
//!
//! What these tests specify (the implementer follows):
//! * a body whose result is text, a date, an instant or a boolean answers 200 with `kind: "calculation"`, `type`
//!   (`string`, `date`, `instant`, `boolean`) and `value` (the lexical form, as a number's is today), whether the
//!   value is read from a stored function of that type or computed (`if ... then "high" else "low"`, a comparison);
//! * no answer is `E_FORMULA_RETURN_TYPE`;
//! * a number is unchanged: `type: "number"`, its `unit`, its value.

#[path = "common/calculus.rs"]
mod calculus;
mod common;

use calculus::*;
use common::pub031::function_line;
use common::readpath::*;
use serde_json::{json, Value};

const STR: &str = r#"{"type":"string","valid":false}"#;

fn n(x: &str) -> String {
    format!("{PFX}/{x}")
}

/// `<x>(site) -> <ty>`, stored.
fn sig(ty: &str) -> String {
    format!(r#"{{"args":{{"site":{STR}}},"order":["site"],"returns":"{ty}"}}"#)
}

fn cell(function: &str, value: Value) -> String {
    json!({"cell": {"fn": function, "args": {"site": {"type": "string", "value": "N"}}}, "licence": "CC0-1.0",
        "summary": "Synthetic test data.", "value": value})
    .to_string()
}

/// The calculus data and four stored functions of other result types, each with a value at site N.
fn lines(c: &Calc) -> Vec<String> {
    let mut v = whatif_lines(c);
    for (x, ty, value) in [
        ("label", "string", json!({"type": "string", "value": "North"})),
        ("opened", "date", json!({"type": "date", "value": "2026-03-01"})),
        ("stamp", "instant", json!({"type": "instant", "value": "2026-03-01T12:00:00.000000Z"})),
        ("active", "boolean", json!({"type": "boolean", "value": true})),
    ] {
        v.push(function_line(&n(x), &sig(ty), None));
        v.push(cell(&n(x), value));
    }
    v
}

/// The members of the answer, at the top level or under `calculation`.
fn ans(r: &Reply) -> &Value {
    match r.json.get("calculation") {
        Some(c) => c,
        None => &r.json,
    }
}

async fn calc(m: &Served, f: &str) -> Reply {
    m.post_raw("/x/calc", serde_json::to_vec(&json!({ "formula": f })).unwrap()).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn calc_answers_text_date_instant_and_boolean_results() {
    let c = calc_names();
    let m = serve_lite("h054-calc-types", &lines(&c), None, |_| {}).await;
    let reading = format!("{}(site: \"N\")", c.reading);
    let cases: Vec<(&str, String, &str, &str)> = vec![
        ("a stored text", format!("{}(site: \"N\")", n("label")), "string", "North"),
        ("a computed text", format!("if {reading} > 5 then \"high\" else \"low\""), "string", "high"),
        ("a stored date", format!("{}(site: \"N\")", n("opened")), "date", "2026-03-01"),
        ("a stored instant", format!("{}(site: \"N\")", n("stamp")), "instant", "2026-03-01T12:00:00.000000Z"),
        ("a stored boolean", format!("{}(site: \"N\")", n("active")), "boolean", "true"),
        ("a comparison", format!("{reading} > 5"), "boolean", "true"),
    ];
    let mut wrong = Vec::new();
    for (what, f, ty, value) in cases {
        let r = calc(&m, &f).await;
        let text = String::from_utf8_lossy(&r.body).to_string();
        let a = ans(&r);
        let ok = r.status == 200
            && !text.contains("E_FORMULA_RETURN_TYPE")
            && a["kind"] == "calculation"
            && a["type"] == ty
            && a["value"] == value
            && a["formula"] == f.as_str();
        if !ok {
            wrong.push(format!("{what} ({f}): want 200, type {ty}, value {value}; got {} {text}", r.status));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A number is answered as before: type, unit and value.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn calc_answers_a_number_as_before() {
    let c = calc_names();
    let m = serve_lite("h054-calc-number", &lines(&c), None, |_| {}).await;
    let r = calc(&m, &format!("{}(site: \"N\") * 100", c.reading)).await;
    let text = String::from_utf8_lossy(&r.body).to_string();
    assert_eq!(r.status, 200, "{text}");
    let a = ans(&r);
    assert_eq!(
        (a["kind"].as_str(), a["type"].as_str(), a["value"].as_str()),
        (Some("calculation"), Some("number"), Some("1000")),
        "{text}"
    );
    assert_eq!(a["unit"], "1", "{text}");
}

fn calc_names() -> Calc {
    calculus::calc()
}
