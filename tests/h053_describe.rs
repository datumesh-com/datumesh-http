//! http 0.5.3, CONTRACT-protocol-part1 H1 (D5): `/describe` of a function carries `order`.
//! Written first; FAIL until then.
//!
//! `order` is the descriptor's `signature.order`, else the argument names in the order `/cell` uses for a positional
//! call. It is a member of the describe answer, outside the descriptor, so no hash changes.

mod common;
#[path = "common/h053.rs"]
mod h053;

use h053::*;

const PAR_YIELD: &str = r#"{"args":{"date":{"type":"date","valid":true},"tenor":{"type":"string","valid":false}},"order":["tenor","date"],"returns":{"type":"number","unit":"percent"}}"#;
const NO_ORDER: &str = r#"{"args":{"zeta":{"type":"string","valid":false},"alpha":{"type":"string","valid":false},"mid":{"type":"string","valid":false}},"returns":{"type":"number","unit":"percent"}}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h1_describe_carries_the_declared_order_which_is_not_alphabetical() {
    let a = Node::start(A, |c| c.rate.enabled = false).await;
    let h = a.stored(&format!("{A}/par_yield"), PAR_YIELD);
    let r = a.get(&format!("/describe/{A}/par_yield")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = jv(&r);
    assert_eq!(v["order"], serde_json::json!(["tenor", "date"]), "{v}");
    // a member of the answer beside `args`, not part of the stored descriptor
    assert_eq!(v["fn_hash"], h.as_str(), "no hash changes: the stored descriptor is as it was");
    assert!(v["args"].get("order").is_none(), "{v}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h1_a_formula_function_describes_its_order_too() {
    let a = Node::start(A, |c| c.rate.enabled = false).await;
    a.stored(&format!("{A}/py"), PAR_YIELD);
    a.formula(
        &format!("{A}/py2"),
        &format!("{A}/py(tenor: tenor, date: date)"),
        PAR_YIELD,
    );
    let v = jv(&a.get(&format!("/describe/{A}/py2")).await);
    assert_eq!(v["order"], serde_json::json!(["tenor", "date"]), "{v}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h1_without_a_declared_order_it_is_the_order_cell_uses_for_a_positional_call() {
    let a = Node::start(A, |c| c.rate.enabled = false).await;
    a.stored(&format!("{A}/noord"), NO_ORDER);
    let r = a.get(&format!("/describe/{A}/noord")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = jv(&r);
    // the engine's canonical argument order is the ascending names (SPEC 5.3): `/cell` reads a positional call that way
    assert_eq!(v["order"], serde_json::json!(["alpha", "mid", "zeta"]), "{v}");
    // and the positional call agrees: `noord("a", "m", "z")` is alpha=a, mid=m, zeta=z
    a.put_value(
        &format!("{A}/noord"),
        &args(&[("alpha", s("a")), ("mid", s("m")), ("zeta", s("z"))]),
        n("7"),
        T1,
    );
    let call = "mesh-a.example/noord(\"a\", \"m\", \"z\")";
    let r = a.get(&format!("/cell/{}", call.replace('"', "%22").replace(' ', "%20"))).await;
    assert_eq!(jv(&r)["receipt"]["value"]["value"], "7", "{}", r.text());
}
