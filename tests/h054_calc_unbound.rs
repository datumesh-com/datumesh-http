//! http 0.5.4, CONTRACT-next-patch item 16: `/x/calc` builds the callee environment in one pass from the formula
//! crate's `read_names` (the names a body reads), in place of the loop that matched the "has no binding" message.
//! **The behaviour is unchanged.** This regression test pins what http 0.5.3 answers to a body that names a
//! function with no binding, byte for byte (status and body), in every place a name can stand: alone, beside a bound
//! name (before it and after it), in a slice read under an aggregate, called positionally, on another host, as the
//! argument of a bound function; and an `@alias` the mesh does not have. It passes before the change and must pass
//! after it. The positional `/x/calc` tests (`h053_asof.rs`) stay as they are.

#[path = "common/calculus.rs"]
mod calculus;
mod common;

use calculus::*;
use common::readpath::*;
use serde_json::json;

const UNRESOLVED: &str = r#"{"error":"E_FORMULA_UNRESOLVED","outcome":"bad_request"}"#;
const ALIAS: &str = r#"{"error":"E_FORMULA_ALIAS","outcome":"bad_request"}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_body_with_an_unbound_name_answers_as_before() {
    let c = calc();
    let m = serve_lite("h054-calc-unbound", &whatif_lines(&c), None, |_| {}).await;
    let nobody = format!("{PFX}/nobody");
    let cases: Vec<(&str, String, &str)> = vec![
        ("alone", format!("{nobody}(site: \"N\") + 1"), UNRESOLVED),
        ("after a bound name", format!("{}(site: \"N\") + {nobody}(site: \"N\")", c.reading), UNRESOLVED),
        ("before a bound name", format!("{nobody}(site: \"N\") + {}(site: \"N\")", c.reading), UNRESOLVED),
        ("a slice read under an aggregate", format!("sum({nobody}(site: *))"), UNRESOLVED),
        ("called positionally", format!("{nobody}(\"N\")"), UNRESOLVED),
        ("on another host", "other.example/x/f(site: \"N\")".to_string(), UNRESOLVED),
        ("as the argument of a bound function", format!("{}(site: {nobody}(site: \"N\"))", c.double), UNRESOLVED),
        ("an alias the mesh does not have", "@nope(site: \"N\")".to_string(), ALIAS),
    ];
    for (what, f, want) in cases {
        let r = m.post_raw("/x/calc", serde_json::to_vec(&json!({ "formula": f })).unwrap()).await;
        let body = String::from_utf8_lossy(&r.body).to_string();
        assert_eq!((r.status, body.as_str()), (400, want), "{what}: {f}");
    }
    // the control: the bound names alone answer
    let r = m
        .post_raw(
            "/x/calc",
            serde_json::to_vec(&json!({ "formula": format!("{}(site: \"N\") + 1", c.reading) })).unwrap(),
        )
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
}
