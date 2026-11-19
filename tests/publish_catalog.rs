//! Build 2 Part 2, R5: `publish-stored` files may carry `catalog` lines: `{"catalog": "<name>",
//! "kind": "function"|"class", "label" (at most 120 bytes), "description" (1 to 1024 bytes),
//! "args": {<arg>: <text>}, "examples": [{"call", "expected"}], "dictionary": {<key>: [{"value",
//! "label", "aliases"?}]}}`, signed by the operator's run like the rest of the file. SQLite store,
//! the clock fixed at NOW.

mod common;

use common::lite::*;
use common::pub031::*;
use common::readpath::*;
use serde_json::json;

const F: &str = "mesh-test.datumesh.com/scratch_b2/pc/level";

fn fline() -> String {
    function_line(F, &site_sig(), None)
}

fn cat(extra: serde_json::Value) -> String {
    let mut v = json!({"catalog": F, "kind": "function", "label": "Level", "description": "Water level readings",
        "args": {"site": "the site"}, "examples": [], "dictionary": {}});
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    v.to_string()
}

/// A file with a function and its catalog line is accepted and re-running it changes nothing.
#[test]
fn a_catalog_line_is_accepted_and_idempotent() {
    let lite = Lite::new("pc-1");
    let f = write(&lite, "a.jsonl", &[fline(), cat(json!({}))]);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    let before = snapshot(&lite);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(snapshot(&lite), before, "a second run wrote something");
    // examples with expected values and dictionaries with aliases are accepted
    let lite = Lite::new("pc-1b");
    let rich = cat(json!({
        "examples": [{"call": format!("{F}(site: \"N\")"), "expected": {"type": "number", "value": "1"}}],
        "dictionary": {"site": [{"value": "N", "label": "North pier", "aliases": ["np"]}]},
    }));
    let f = write(&lite, "b.jsonl", &[fline(), rich]);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
}

/// The catalog line is read: it is in `/find` with the label, the description and the template.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_catalog_line_reaches_find() {
    let m = serve_lite("pc-2", &[fline(), cat(json!({}))], None, |_| {}).await;
    let v = m.find("level readings").await;
    let r = result(&v, F);
    assert_eq!((r["label"].as_str(), r["description"].as_str()), (Some("Level"), Some("Water level readings")));
    assert_eq!(r["call"], format!("{F}(site: <string>)"));
}

/// A catalog line that breaks its limits refuses the whole file at its line, and writes nothing.
#[test]
fn bad_catalog_lines_refuse_the_file() {
    let cases: Vec<(&str, String)> = vec![
        ("label over 120 bytes", cat(json!({"label": "L".repeat(121)}))),
        ("empty description", cat(json!({"description": ""}))),
        ("description over 1024 bytes", cat(json!({"description": "d".repeat(1025)}))),
        ("unknown kind", cat(json!({"kind": "table"}))),
        ("a name that is bound nowhere", cat(json!({"catalog": "mesh-test.datumesh.com/scratch_b2/pc/ghost"}))),
        ("another host's name", cat(json!({"catalog": "other.example/scratch/level"}))),
        ("a dictionary entry without a label", cat(json!({"dictionary": {"site": [{"value": "N"}]}}))),
    ];
    // control: the same file with a good catalog line is accepted
    let lite = Lite::new("pc-3-ok");
    let f = write(&lite, "ok.jsonl", &[fline(), cat(json!({}))]);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("control: {e}"));
    for (i, (what, line)) in cases.into_iter().enumerate() {
        let lite = Lite::new(&format!("pc-3-{i}"));
        let e = refused_whole(&lite, "bad.jsonl", &[fline(), line], 2, NOW);
        eprintln!("{what}: {e}");
    }
}

/// Exactly at the limits is fine: a label of 120 bytes, a description of 1 and of 1024 bytes.
#[test]
fn the_limits_are_inclusive() {
    for (i, extra) in
        [json!({"label": "L".repeat(120)}), json!({"description": "d"}), json!({"description": "d".repeat(1024)})]
            .into_iter()
            .enumerate()
    {
        let lite = Lite::new(&format!("pc-4-{i}"));
        let f = write(&lite, "ok.jsonl", &[fline(), cat(extra)]);
        run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    }
}
