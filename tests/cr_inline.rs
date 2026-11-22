//! The classes round (CONTRACT-classes-round.md), K7: a class with its methods in one publish step. Failing tests: a class
//! line's `methods` takes function hashes only, and a body has no `this` before its class exists.
//!
//! In `{"class": N, "descriptor": {...}}` a method is a function hash, as today, or `{"stored": <signature>}`, or
//! `{"formula": <source>, "signature": <signature>}`. The inline step defines the stored methods, then the formula methods in
//! an order where each comes after the methods it reads with `this` (mutual reads are refused, 16.3), each taking the class keys
//! first (its signature says so, 16.2 step 6), and binds the class and the methods' names in one transaction. A later
//! `catalog`, cell or test line names `N/m` as usual, and the public tests rule applies to inline formula methods.
//!
//! Also here: a variable-key receiver in a body (`N(sku).price()`, sugar for the plain read), and K5's `this` through an
//! inherited method (a subclass's inline formula method reads the parent's cells).

mod common;
#[path = "common/crworld.rs"]
mod crworld;

use common::lite::*;
use common::pub031::{descriptor_hash, opts, snapshot, stored_descriptor, write};
use common::readpath::*;
use crworld::*;
use datumesh_http::publish::{publish_stored_checked, PublishError, PublishErrorKind, Published};
use datumesh_store::Store;
use serde_json::{json, Value};

fn item() -> String {
    format!("{HOST}/shop/item")
}

fn sku_sig() -> Value {
    json!({"args": {"sku": {"type": "string", "valid": false}}, "order": ["sku"], "returns": {"type": "number", "unit": "1"}})
}

fn steward() -> Value {
    json!({"namespace": HOST, "process": format!("https://{HOST}/classes/change")})
}

/// The class line of `shop/item`: key `sku`, with the given inline methods `(name, method value)`.
fn item_class(methods: Vec<(&str, Value)>) -> String {
    let m: serde_json::Map<String, Value> = methods.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    json!({"class": item(), "descriptor": {
        "keys": {"sku": {"key_normalisation": "none", "type": "string"}},
        "methods": m,
        "order": ["sku"],
        "steward": steward()}})
    .to_string()
}

fn price() -> Value {
    json!({"stored": sku_sig()})
}

fn formula(src: &str) -> Value {
    json!({"formula": src, "signature": sku_sig()})
}

/// The methods: `price` stored; `double` is `this.price() * 2`; `quad` is `this.double() * 2` (one method reading another).
fn methods() -> Vec<(&'static str, Value)> {
    vec![("price", price()), ("double", formula("this.price() * 2")), ("quad", formula("this.double() * 2"))]
}

fn price_cell(sku: &str, v: &str) -> String {
    cell_line(&format!("{}/price", item()), &[("sku", "string", sku)], ("number", v))
}

fn t(function: &str, sku: &str, expect: Value) -> String {
    json!({"test": function, "call": format!("{function}(sku: \"{sku}\")"), "expect": expect}).to_string()
}

fn val(v: &str) -> Value {
    json!({"type": "number", "value": v})
}

fn absent() -> Value {
    json!({"fault": "E_FORMULA_ABSENT"})
}

/// The tests of `double` and `quad` that satisfy the public rule: a value, and the absent input.
fn tests() -> Vec<String> {
    let (d, q) = (format!("{}/double", item()), format!("{}/quad", item()));
    vec![
        t(&d, "A", val("20")),
        t(&d, "ZZ", absent()),
        t(&q, "A", val("40")),
        t(&q, "ZZ", absent()),
    ]
}

/// The whole step in one file: the class with its inline methods, the catalogue (class and methods), cells, tests.
fn one_file() -> Vec<String> {
    let mut v = vec![item_class(methods())];
    v.push(catalog_line_of(&item(), "class", json!({"args": {}})));
    v.push(catalog_line_of(&format!("{}/double", item()), "function", json!({"args": {"sku": "the sku"}})));
    v.push(price_cell("A", "10"));
    v.push(price_cell("B", "3"));
    v.extend(tests());
    v
}

fn checked_run(lite: &Lite, lines: &[String]) -> Result<Published, PublishError> {
    let f = write(lite, "f.jsonl", lines);
    publish_stored_checked(&*lite.store, &lite.key, HOST, &[HOST.to_string()], &f, &opts(NOW))
}

/// One file with a class whose methods are inline (stored and formula, `this`, one method reading another), the catalogue, cells
/// and tests is accepted with the public rule on; the class and every method's name are bound from one transaction; the class
/// descriptor holds the hashes of the functions the names are bound to; a second run changes nothing.
#[test]
fn k7_one_publish_step_defines_the_class_and_its_methods() {
    let lite = Lite::new("cr-k7-one");
    let p = checked_run(&lite, &one_file()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(p.classes, 1, "{p:?}");
    assert_eq!(p.cells, 2, "{p:?}");
    let class = class_hash(&lite, &item());
    let bytes = lite.store.class(&class).unwrap().unwrap();
    let d = datumesh_core::ClassDescriptor::from_json(&bytes).unwrap();
    assert_eq!(d.methods.keys().map(String::as_str).collect::<Vec<_>>(), ["double", "price", "quad"]);
    let mut txs = std::collections::BTreeSet::new();
    for (m, h) in &d.methods {
        let b = lite.store.resolve_binding(&format!("{}/{m}", item()), END).unwrap().unwrap_or_else(|| panic!("{m} unbound"));
        assert_eq!(&b.fn_hash, h, "{m}: the name is bound to the hash the class holds");
        txs.insert(b.tx);
    }
    let cb = lite.store.resolve_class_binding(&item(), END).unwrap().unwrap();
    assert_eq!(txs.len(), 1, "the methods' names are bound in one transaction: {txs:?}");
    assert_eq!(txs.iter().next().unwrap(), &cb.tx, "and the class with them (bind_class)");
    // the stored method is the function `{"stored": sig}` makes: the descriptor of 7 with the lang `stored`
    let stored = stored_descriptor(&sku_sig().to_string());
    assert_eq!(d.methods["price"], descriptor_hash(&stored), "the hash of the stored descriptor");
    // the catalogue and the tests were taken
    let fh = d.methods["double"].clone();
    assert_eq!(lite.store.function_tests(&fh).unwrap().len(), 2, "the public tests of an inline formula method are recorded");
    // a second run changes nothing
    let before = snapshot(&lite);
    let again = checked_run(&lite, &one_file()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((again.functions, again.classes, again.versions), (0, 0, 0), "{again:?}");
    assert_eq!(snapshot(&lite), before);
}

/// The class is served: `this` in an inline formula method reads the cell of the method of the same object (`double` is 20),
/// and a method reading another with `this` reads the computed cell of that one (`quad` is 40, its input is `double`'s cell).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k7_this_and_a_method_reading_another() {
    let lite = Lite::new("cr-k7-this");
    checked_run(&lite, &one_file()).unwrap_or_else(|e| panic!("{e}"));
    let m = build_served(lite, None, |_| {}).await;
    let n = item();
    let price = ask(&m, &format!("/cell/{n}/price?sku=A")).await;
    assert_eq!(price.value(), Some("10"), "{}", price.text);
    let double = ask(&m, &format!("/cell/{n}/double?sku=A")).await;
    assert_eq!((double.outcome(), double.value()), ("ok", Some("20")), "{}", double.text);
    assert_eq!(double.inputs(), vec![price.receipt_hash()], "`this.price()` is the read of `{n}/price(sku: sku)`: its receipt is the input");
    let quad = ask(&m, &path(&format!("{n}(A).quad()"), None)).await;
    assert_eq!((quad.outcome(), quad.value()), ("ok", Some("40")), "{}", quad.text);
    assert_eq!(quad.inputs(), vec![double.receipt_hash()], "`this.double()` is the read of `{n}/double(sku: sku)`");
    // an absent input is the fault of the cell
    let zz = ask(&m, &format!("/cell/{n}/quad?sku=ZZ")).await;
    // SPEC 15.12: a single-cell read with no value is E_FORMULA_NOT_COVERED, plain not_covered with no fault member
    // (E_FORMULA_ABSENT is for a lookup with no qualifying member or a computed slice member without value).
    assert_eq!((zz.outcome(), zz.fault()), ("not_covered", None), "{}", zz.text);
}

/// Two methods that read each other with `this` are refused (16.3: the class holds each method's function hash, and two
/// functions cannot each be defined after the other): the file is refused at the class line and nothing is written.
#[test]
fn k7_a_mutual_read_is_refused() {
    let lite = Lite::new("cr-k7-mutual");
    let before = snapshot(&lite);
    let e = checked_run(&lite, &[item_class(vec![("price", price()), ("a", formula("this.b() + 1")), ("b", formula("this.a() + 1"))])])
        .expect_err("a mutual read is refused");
    assert_eq!(e.line, 1, "{e}");
    assert_eq!(snapshot(&lite), before, "nothing is written: {e}");
    assert!(lite.store.resolve_class(&item(), END).unwrap().is_none());
    assert!(lite.store.resolve_binding(&format!("{}/a", item()), END).unwrap().is_none());
    // a read of itself is refused too
    let e = checked_run(&lite, &[item_class(vec![("price", price()), ("a", formula("this.a() + 1"))])])
        .expect_err("a method that reads itself is refused");
    assert_eq!(e.line, 1, "{e}");
    assert_eq!(snapshot(&lite), before);
    // the control: a one-way chain in the other order of names is accepted (the order of definition is the reads', not the names')
    let ok = item_class(vec![("price", price()), ("a", formula("this.z() + 1")), ("z", formula("this.price() + 1"))]);
    let tests = [
        t(&format!("{}/a", item()), "A", val("12")),
        t(&format!("{}/a", item()), "ZZ", absent()),
        t(&format!("{}/z", item()), "A", val("11")),
        t(&format!("{}/z", item()), "ZZ", absent()),
    ];
    let mut v = vec![ok, price_cell("A", "10")];
    v.extend(tests);
    checked_run(&lite, &v).unwrap_or_else(|e| panic!("a reads z, z reads price: {e}"));
}

/// The public rule applies to inline formula methods: a file whose formula method has no tests is `E_TESTS_MISSING`, with the
/// derived cases, and writes nothing; a method whose test fails is `E_TESTS_FAILING` at the test's line, and writes nothing,
/// the class and the names included.
#[test]
fn k7_a_formula_method_that_fails_its_tests_writes_nothing() {
    // without tests
    let lite = Lite::new("cr-k7-missing");
    let before = snapshot(&lite);
    let e = checked_run(&lite, &[item_class(methods()), price_cell("A", "10")]).expect_err("tests missing");
    assert!(matches!(e.kind, PublishErrorKind::TestsMissing(_)), "{e}");
    assert!(e.detail.contains(&format!("{}/double", item())) && e.detail.contains("absent("), "{e}");
    assert_eq!(snapshot(&lite), before);
    // a failing test
    let lite = Lite::new("cr-k7-failing");
    let before = snapshot(&lite);
    let mut v = one_file();
    let at = v.iter().position(|l| l.contains(r#""test":"mesh-test.datumesh.com/shop/item/double""#) && l.contains(r#""value":"20""#)).unwrap();
    v[at] = t(&format!("{}/double", item()), "A", val("21"));
    let e = checked_run(&lite, &v).expect_err("a failing test");
    assert!(matches!(e.kind, PublishErrorKind::TestsFailing(_)), "{e}");
    assert_eq!(e.line, at + 1, "{e}");
    assert!(e.detail.contains("20") && e.detail.contains("21"), "{e}");
    assert_eq!(snapshot(&lite), before, "the class and the methods' names are not written");
    assert!(lite.store.resolve_class(&item(), END).unwrap().is_none());
    assert!(lite.store.resolve_binding(&format!("{}/price", item()), END).unwrap().is_none());
}

/// An inline method must take the class keys first, in class order (16.2 step 6): a formula method whose signature lacks the
/// key is refused at the class line.
#[test]
fn k7_an_inline_method_takes_the_class_keys_first() {
    // the control: the same method with the key first is accepted
    let with_key = json!({"stored": {"args": {"date": {"type": "date", "valid": true}, "sku": {"type": "string", "valid": false}},
        "order": ["sku", "date"], "returns": {"type": "number", "unit": "1"}}});
    let ok = Lite::new("cr-k7-keys-ok");
    checked_run(&ok, &[item_class(vec![("price", price()), ("by_date", with_key)])]).unwrap_or_else(|e| panic!("control: {e}"));
    let lite = Lite::new("cr-k7-keys");
    let before = snapshot(&lite);
    let no_key = json!({"stored": {"args": {"date": {"type": "date", "valid": true}}, "order": ["date"], "returns": {"type": "number", "unit": "1"}}});
    let e = checked_run(&lite, &[item_class(vec![("price", price()), ("by_date", no_key)])]).expect_err("keys first");
    assert_eq!(e.line, 1, "{e}");
    assert_eq!(snapshot(&lite), before);
}

/// A receiver may have keys that are not literals in a body: `N(sku).price()` with `sku` the formula's own argument is the
/// plain read `N/price(sku: sku)`: the formula defines (it was `E_FORMULA_PLACEMENT`) and evaluates, and the cell it reads is
/// the plain read's. In a class-typed position the keys stay literal.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k7_a_variable_key_receiver_in_a_body() {
    let lite = Lite::new("cr-k7-variable");
    checked_run(&lite, &one_file()).unwrap_or_else(|e| panic!("{e}"));
    let n = item();
    let f = formula_line(&format!("{HOST}/shop/price_of"), &format!("{n}(sku).price()"), &sku_sig().to_string());
    let g = formula_line(&format!("{HOST}/shop/price_plus"), &format!(r#"{n}(sku).double() + {n}(upper(sku)).price()"#), &sku_sig().to_string());
    publish(&lite, "variable.jsonl", &[f, g], NOW).unwrap_or_else(|e| panic!("a receiver with a variable key: {e}"));
    let m = build_served(lite, None, |_| {}).await;
    let plain = ask(&m, &format!("/cell/{n}/price?sku=A")).await;
    let v = ask(&m, &format!("/cell/{HOST}/shop/price_of?sku=A")).await;
    assert_eq!((v.outcome(), v.value()), ("ok", Some("10")), "{}", v.text);
    assert_eq!(v.inputs(), vec![plain.receipt_hash()], "sugar for the plain read with those expressions");
    // computed keys are expressions as well: `upper("A")` is "A"
    let w = ask(&m, &format!("/cell/{HOST}/shop/price_plus?sku=A")).await;
    assert_eq!((w.outcome(), w.value()), ("ok", Some("30")), "{}", w.text);
}

/// K5 through `this` and K7 together: a subclass's inline formula method reads the parent's stored method with `this`: the
/// read is the cell of the defining class (`fx_pair/rate`), whose value is 1.2, so the method is 2.4. The parent is a class
/// of one file (inline `rate`), the subclass another (its parent's hash from the store).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k7_this_reads_an_inherited_method_of_the_defining_class() {
    let lite = Lite::new("cr-k7-inherit");
    let p = format!("{HOST}/rates/fx_pair");
    let s = format!("{HOST}/rates/fx_ndf");
    let rate_sig = json!({"args": {"base": {"type": "string", "valid": false}, "date": {"type": "date", "valid": true},
        "quote": {"type": "string", "valid": false}}, "order": ["base", "quote", "date"], "returns": {"type": "number", "unit": "1"}});
    let pkeys = json!({"base": {"key_normalisation": "none", "type": "string"}, "quote": {"key_normalisation": "none", "type": "string"}});
    let parent = json!({"class": p, "descriptor": {"keys": pkeys, "methods": {"rate": {"stored": rate_sig}},
        "order": ["base", "quote"], "steward": steward()}})
    .to_string();
    let cell = cell_line(&format!("{p}/rate"), &[("base", "string", "EUR"), ("date", "date", DAY), ("quote", "string", "USD")], ("number", "1.2"));
    publish(&lite, "parent.jsonl", &[parent, cell], BEFORE_T).unwrap_or_else(|e| panic!("parent: {e}"));
    let via_sig = json!({"args": {"base": {"type": "string", "valid": false}, "date": {"type": "date", "valid": true},
        "fixing": {"type": "string", "valid": false}, "quote": {"type": "string", "valid": false}},
        "order": ["base", "quote", "fixing", "date"], "returns": {"type": "number", "unit": "1"}});
    let child = json!({"class": s, "descriptor": {
        "keys": {"fixing": {"key_normalisation": "none", "type": "string"}},
        "methods": {"via_this": {"formula": "this.rate(date: date) * 2", "signature": via_sig}},
        "order": ["fixing"], "parent": class_hash(&lite, &p), "steward": steward()}})
    .to_string();
    publish(&lite, "child.jsonl", &[child], NOW).unwrap_or_else(|e| panic!("a formula method reading an inherited one with this: {e}"));
    let m = build_served(lite, None, |_| {}).await;
    let a = ask(&m, &path(&format!("{s}(EUR, USD, F1).via_this({DAY})"), None)).await;
    assert_eq!((a.outcome(), a.value()), ("ok", Some("2.4")), "{}", a.text);
    let plain = ask(&m, &format!("/cell/{p}/rate?base=EUR&date={DAY}&quote=USD")).await;
    assert_eq!(a.inputs(), vec![plain.receipt_hash()], "the read through this is the defining class's cell");
}

/// The first file's clock of the last test: before the second's, so that the subclass binds after its parent.
const BEFORE_T: &str = "2026-11-15T11:59:59.999999Z";
