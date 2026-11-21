//! http 0.4.10: a class's method bindings are in the publish-time overlay. A file that publishes a class
//! and, in the same file, a formula whose tests call one of the class's methods (`<class>/<method>`) must
//! publish: the publish-time test run reads the file's planned bindings, and the methods a class binds in
//! that file are among them (before the fix `overlay_of` ignored `Op::Class`, so the test run failed with
//! `E_FORMULA_UNRESOLVED`, "... has no binding").

#[path = "common/calculus.rs"]
mod calculus;
mod common;

use calculus::*;
use common::lite::*;
use common::pub031::*;
use common::readpath::*;
use common::world::call_path;
use datumesh_store::Store;
use serde_json::json;

const P: &str = "mesh-test.datumesh.com/scratch_h0410o";
const FILING: &str = "mesh-test.datumesh.com/scratch_h0410o/filing";

const STR: &str = r#"{"type":"string","valid":false}"#;

fn filed() -> String {
    format!("{FILING}/filed")
}

fn filed_of() -> String {
    format!("{P}/filed_of")
}

/// The method `filed(accession) -> date`: a stored function, with no arguments after the class key but
/// `accession`.
fn filed_sig() -> String {
    r#"{"args":{"accession":{"type":"string","valid":false}},"order":["accession"],"returns":"date"}"#.to_string()
}

fn filed_of_sig() -> String {
    format!(r#"{{"args":{{"accession":{STR}}},"order":["accession"],"returns":"date"}}"#)
}

fn filed_cell(accession: &str, date: &str) -> String {
    json!({"cell": {"fn": filed(), "args": {"accession": {"type": "string", "value": accession}}},
        "licence": "CC0-1.0", "summary": "Synthetic test data.", "value": {"type": "date", "value": date}})
    .to_string()
}

/// One publish file: the method's descriptor, the class that binds it, its cells, and the formula over
/// the method with its tests (the public rule's absent case included).
fn one_file() -> Vec<String> {
    let d = stored_descriptor(&filed_sig());
    let h = descriptor_hash(&d);
    let class = common::world::class_json(
        r#"{"accession":{"key_normalisation":"none","type":"string"}}"#,
        &["accession"],
        &[("filed", &h)],
        HOST,
        "",
    );
    let mut v = vec![
        descriptor_line(&d),
        class_line(FILING, &class, None),
        filed_cell("A1", "2026-01-01"),
        filed_cell("A2", "2026-03-01"),
        formula_line(&filed_of(), &format!("{}(accession: accession)", filed()), &filed_of_sig(), None),
    ];
    let date = |s: &str| json!({"type": "date", "value": s});
    v.push(test_line(&filed_of(), &format!("{}(accession: \"A1\")", filed_of()), None, None, date("2026-01-01")));
    v.push(test_line(&filed_of(), &format!("{}(accession: \"A2\")", filed_of()), None, None, date("2026-03-01")));
    v.push(test_line(
        &filed_of(),
        &format!("{}(accession: \"ZZ\")", filed_of()),
        None,
        None,
        fault("E_FORMULA_ABSENT"),
    ));
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_class_and_a_formula_over_its_method_publish_in_one_file() {
    let m = serve_lite("h0410-class-overlay", &one_file(), None, |_| {}).await;
    // the method is bound by the class, and the formula evaluates through it
    assert!(m.lite.store.resolve_name(&filed(), END).unwrap().is_some(), "{} is unbound", filed());
    let v = m.get(&format!("/cell/{}", call_path(&format!("{}(accession: \"A2\")", filed_of())))).await;
    assert_eq!(v["receipt"]["value"], json!("2026-03-01"), "{v}");
}
