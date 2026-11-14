//! Classes in the mesh, faults (spec-0.5.0, 15.12, 16.4, 16.6; `CONTRACT-http-0.3.0-classes.md`
//! item 7): `E_FORMULA_KEY` (a computed key not canonical under its rule) and
//! `E_FORMULA_DISPATCH_KEY` (a value outside the dispatch set) are answered `not_covered` with
//! `fault`, deterministically (the second request is the first's answer), through engine 0.4.2.

mod common;

use common::world::*;
use common::*;
use datumesh_core::json::Value;

const A: &str = "mesh-a.example";

fn name_of(w: &World, f: &str) -> String {
    format!("{}/ecb/{f}", w.host)
}

/// A request of `f` with `date` and one more argument, checked as a client checks an envelope.
async fn read(a: &Node, f: &str, extra: (&str, &str)) -> Resp {
    let name = name_of(&World::new(&a.host), f);
    let r = a.get(&format!("/cell/{name}?date={DAY}&{}={}", extra.0, qenc(extra.1))).await;
    let id = a.cell_id(&name, &args(&[("date", d(DAY)), (extra.0, s(extra.1))]));
    assert_eq!(check(&r, &a.keys(), &id, None, None), Ok("not_covered".to_string()), "{}", r.text());
    r
}

fn fault(r: &Resp) -> Option<String> {
    r.json().get("fault").and_then(Value::as_str).map(str::to_string)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_computed_key_outside_its_rule_is_the_fault_e_formula_key() {
    let a = Node::start(A, |_| {}).await;
    install(&a);
    // lower("EUR") is "eur": not canonical under upper_ascii, the rule fx_upper/rate's key carries.
    let r = read(&a, "lower_rate", ("x", "EUR")).await;
    assert_eq!(r.status, 404, "{}", r.text());
    assert_eq!(fault(&r).as_deref(), Some("E_FORMULA_KEY"), "{}", r.text());
    assert!(r.json().get("error").is_none() && r.json().get("receipt").is_none());
    // Deterministic: the cell has no value, and asking again says the same.
    let again = read(&a, "lower_rate", ("x", "EUR")).await;
    assert_eq!((again.status, again.body.clone()), (r.status, r.body.clone()));
    // A computed key that is canonical is no fault: "123" has no lowercase letter; no cell has a value.
    let r = a.get(&format!("/cell/{}?date={DAY}&x=123", name_of(&World::new(A), "lower_rate"))).await;
    assert_eq!((r.status, r.outcome()), (404, "not_covered".to_string()), "{}", r.text());
    assert_eq!(fault(&r), None, "{}", r.text());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dispatch_on_a_value_outside_the_set_is_the_fault_e_formula_dispatch_key() {
    let a = Node::start(A, |_| {}).await;
    let w = install(&a);
    // The key of a class of the set: dispatched, and the receipt's lineage is the stored cell's.
    let good = format!(r#"{}(base:"EUR",quote:"USD")"#, w.upper);
    let name = name_of(&w, "upper_rate");
    let r = a.get(&format!("/cell/{name}?date={DAY}&pair={}", qenc(&good))).await;
    assert_eq!(r.outcome(), "ok", "{}", r.text());
    let env = r.json();
    assert_eq!(env.get("receipt").and_then(|x| x.get("value")).and_then(Value::as_str), Some("1.2"));
    let inputs: Vec<String> = match env.get("receipt").and_then(|x| x.get("inputs")) {
        Some(Value::Array(v)) => v.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        _ => vec![],
    };
    assert_eq!(inputs.len(), 1, "{}", r.text());
    let stored = Receipt_of(&a, &format!("{}/rate", w.upper));
    assert_eq!(inputs[0], stored, "lineage records the receipt of the function actually called");

    // Outside the set: a malformed encoding, a class that is not in the set, the keys of another
    // class's shape: the fault, not an error.
    let other_class = format!(r#"{}(base:"EUR",quote:"USD")"#, w.pair);
    let missing_key = format!(r#"{}(base:"EUR")"#, w.upper);
    for pair in ["notakey", other_class.as_str(), missing_key.as_str(), ""] {
        let r = read(&a, "upper_rate", ("pair", pair)).await;
        assert_eq!(r.status, 404, "{pair}: {}", r.text());
        assert_eq!(fault(&r).as_deref(), Some("E_FORMULA_DISPATCH_KEY"), "{pair}: {}", r.text());
        let again = read(&a, "upper_rate", ("pair", pair)).await;
        assert_eq!(again.body, r.body, "{pair}: deterministic");
    }
    // A well-formed key of a class of the set that is not canonical under the class's rule still
    // decodes as that class: the read proceeds, and the method's argument rule gives E_FORMULA_KEY.
    let lower = format!(r#"{}(base:"eur",quote:"USD")"#, w.upper);
    let r = read(&a, "upper_rate", ("pair", &lower)).await;
    assert_eq!(fault(&r).as_deref(), Some("E_FORMULA_KEY"), "{}", r.text());
}

/// The hash of the receipt of `name`'s stored EUR/USD cell.
#[allow(non_snake_case)]
fn Receipt_of(a: &Node, name: &str) -> String {
    let id = a.cell_id(name, &args(&[("base", s("EUR")), ("date", d(DAY)), ("quote", s("USD"))]));
    bl(|| a.store.latest(&id).unwrap().expect("a stored version").receipt_hash)
}
