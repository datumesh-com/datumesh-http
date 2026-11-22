//! The 0.7 round (CONTRACT-round-0.7.md), R2: **a name can be closed** (SPEC 9.7 "Closing a name", 16.3, D124). Failing tests: the
//! behaviour is not written yet.
//!
//! A publish line `{"close": "<full name>"}`, under the namespace's key like any binding, ends the name's binding at the mesh's
//! clock at the write (no backdating). From that instant the name is unbound: `/bindings` answers it with an empty `bindings`
//! object, a read is `not_covered` with `E_NAME_UNBOUND`, `describe` and `find` do not list it; before that instant every answer
//! is unchanged and every receipt still verifies. Closing a class closes the class name and every `N/m` its binding bound. A
//! closed name may be bound again later (a new binding with its own `since`). An unbound name cannot be closed
//! (`E_NAME_UNBOUND`). The bundle member `"close"` is in `r07_close_bundle.rs` (it needs a builder API of its own).
//!
//! The publish clock of each run is the "mesh's clock at the write": `NOW`, then `T2`, then `T3`.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/crworld.rs"]
mod crworld;

use calculus::*;
use common::lite::*;
use common::pub031::{keys, refused_whole, BEFORE, T2};
use common::readpath::{catalog_line, names, Served};
use common::world::qenc;
use crworld::*;
use datumesh_store::Store;
use serde_json::{json, Value};

const T3: &str = "2026-11-15T12:00:03.000000Z";
const BETWEEN: &str = "2026-11-15T12:00:01.000000Z";
const MIDDLE: &str = "2026-11-15T12:00:02.500000Z";

fn close_line(name: &str) -> String {
    json!({ "close": name }).to_string()
}

/// `/bindings` of `names` as of `asof` (now when absent): the `bindings` object.
async fn bound(m: &Served, names: &[&str], asof: Option<&str>) -> Value {
    let mut p = String::from("/bindings?");
    p.push_str(&names.iter().map(|n| format!("name={}", qenc(n))).collect::<Vec<_>>().join("&"));
    if let Some(a) = asof {
        p.push_str(&format!("&asof={}", qenc(a)));
    }
    let v = m.get(&p).await;
    v["bindings"].clone()
}

fn unbound_read(a: &Ans) {
    assert_eq!((a.status, a.outcome(), a.error()), (404, "not_covered", Some("E_NAME_UNBOUND")), "{}", a.text);
}

/// A cell answer as of `asof` is `ok` and checks as a client checks it (core's `check_answer`, against the mesh's key document).
fn verifies(m: &Served, a: &Ans, asof: &str) {
    assert_eq!(a.outcome(), "ok", "{}", a.text);
    let req = datumesh_core::Request::new(&a.cell_id(), Some(asof)).unwrap();
    datumesh_core::check_answer(a.status, a.text.as_bytes(), &keys(&m.lite), &req)
        .unwrap_or_else(|e| panic!("the receipt no longer verifies: {e}: {}", a.text));
}

const LABEL: &str = "Quokka readings";

/// The calculus data and a catalogue line for `reading` at `NOW`; with `close`, a run at `T2` that closes `reading`.
async fn function_world(label: &str, close: bool) -> Served {
    let c = calc();
    let mut first = data_lines(&c);
    first.push(catalog_line(
        &c.reading,
        "function",
        LABEL,
        "Quokka readings at a site",
        json!({"site": "the site"}),
        json!({}),
    ));
    let mut runs: Vec<(&'static str, Vec<String>)> = vec![(NOW, first)];
    if close {
        runs.push((T2, vec![close_line(&c.reading)]));
    }
    serve_runs(label, &runs, None).await
}

// ---------------------------------------------------------------------------------------------------------------------

/// A closed function name: bound before the close (with its `since`), unbound from the close (`T2`, the clock of the write: not
/// earlier), now unbound too; a read as of now, and as of `T2`, is `E_NAME_UNBOUND`; as of before the close the value answers,
/// and its receipt still verifies; `/cell` of a cell that has a value does not hide the history.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r2_a_closed_name_is_unbound_from_the_close() {
    let c = calc();
    let m = function_world("r07-close-fn", true).await;
    let name = c.reading.as_str();
    let b = bound(&m, &[name], Some(BETWEEN)).await;
    assert_eq!(b[name]["since"], NOW, "{b}");
    assert!(b[name]["fn_hash"].as_str().is_some_and(|h| h.len() == 64), "{b}");
    for asof in [Some(T2), None] {
        let b = bound(&m, &[name], asof).await;
        assert_eq!(b, json!({}), "unbound as of {asof:?}: an empty bindings object");
    }
    // the binding ends AT the clock of the write, not before it: one microsecond earlier it is still bound
    let b = bound(&m, &[name], Some("2026-11-15T12:00:01.999999Z")).await;
    assert!(b.get(name).is_some(), "bound until the close: {b}");
    for asof in [None, Some(T2)] {
        let mut p = format!("/cell/{name}?site=N");
        if let Some(a) = asof {
            p.push_str(&format!("&asof={a}"));
        }
        unbound_read(&ask(&m, &p).await);
    }
    let was = ask(&m, &format!("/cell/{name}?site=N&asof={BETWEEN}")).await;
    assert_eq!(was.value(), Some("10"), "before the close nothing changed: {}", was.text);
    verifies(&m, &was, BETWEEN);
}

/// `describe` and `find` do not list a closed name; the same world without the close lists it (the control).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r2_describe_and_find_do_not_list_a_closed_name() {
    let c = calc();
    let open = function_world("r07-close-find-open", false).await;
    let d = ask(&open, &format!("/describe/{}", c.reading)).await;
    assert_eq!(d.status, 200, "control: {}", d.text);
    let f = open.find("Quokka").await;
    assert!(names(&f).contains(&c.reading), "control: {:?}", names(&f));

    let closed = function_world("r07-close-find", true).await;
    let d = ask(&closed, &format!("/describe/{}", c.reading)).await;
    assert_eq!(d.status, 404, "{}", d.text);
    let f = closed.find("Quokka").await;
    assert!(!names(&f).contains(&c.reading), "{:?}", names(&f));
    // the other function of the file is untouched
    let l = ask(&closed, &format!("/describe/{}", c.log)).await;
    assert_eq!(l.status, 200, "{}", l.text);
}

// ---------------------------------------------------------------------------------------------------------------------
// classes

fn method_names() -> Vec<String> {
    let p = product();
    ["price", "name", "successor", "from_label"].iter().map(|m| format!("{p}/{m}")).collect()
}

/// The shop at `NOW`; the lines of its classes are returned too (a rebinding needs them: the method hashes are read before the
/// close). With `close`, the class `shop/product` is closed at `T2`.
async fn shop_world(label: &str, close: bool, again: bool) -> Served {
    let lite = Lite::new(label);
    publish(&lite, "functions.jsonl", &shop_functions(), BEFORE).unwrap_or_else(|e| panic!("functions: {e}"));
    let classes = shop_classes(&lite);
    let mut v = classes.clone();
    v.extend(shop_cells());
    publish(&lite, "classes.jsonl", &v, NOW).unwrap_or_else(|e| panic!("classes and cells: {e}"));
    if close {
        publish(&lite, "close.jsonl", &[close_line(&product())], T2).unwrap_or_else(|e| panic!("close: {e}"));
    }
    if again {
        // a class binds only its own methods' names, which the close ended too: the functions are bound again first
        publish(&lite, "again-f.jsonl", &shop_functions(), T3).unwrap_or_else(|e| panic!("functions again: {e}"));
        publish(&lite, "again-c.jsonl", &classes, T3).unwrap_or_else(|e| panic!("classes again: {e}"));
    }
    common::readpath::build_served(lite, None, |_| {}).await
}

/// Closing a class closes the class name and every `N/m` its binding bound; the other class of the file is untouched; reads
/// through the class (a key, a label, a chain, `describe`) are `E_NAME_UNBOUND`; as of before the close all of it is as it was.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r2_closing_a_class_closes_its_name_and_its_methods() {
    let m = shop_world("r07-close-class", true, false).await;
    let n = product();
    let ms = method_names();
    let mut all: Vec<&str> = vec![n.as_str()];
    all.extend(ms.iter().map(String::as_str));
    let before = bound(&m, &all, Some(BETWEEN)).await;
    assert_eq!(before.as_object().unwrap().len(), 5, "{before}");
    assert!(before[&n]["class_hash"].is_string() && before[&n].get("fn_hash").is_none(), "{before}");
    for method in &ms {
        assert!(before[method]["fn_hash"].is_string(), "{method}: {before}");
    }
    for asof in [Some(T2), None] {
        assert_eq!(bound(&m, &all, asof).await, json!({}), "every name closed as of {asof:?}");
    }
    // the class stays closed for every kind of request
    for p in [
        format!("/cell/{n}/price?date={SHOP_DAY}&sku=A1"),
        path(&format!("{n}(A1).price({SHOP_DAY})"), None),
        path(&format!("{n}[apple].price({SHOP_DAY})"), None),
        path(&format!("{n}(A1).successor().price({SHOP_DAY})"), None),
        format!("/cell/{n}/from_label?label=apple"),
    ] {
        unbound_read(&ask(&m, &p).await);
    }
    let d = ask(&m, &format!("/describe/{}", common::world::call_path(&pk("A1")))).await;
    assert_eq!(d.status, 404, "{}", d.text);
    // as of before the close: the same answers as ever, receipts verifying
    let was = ask(&m, &path(&format!("{n}[apple].price({SHOP_DAY})"), Some(BETWEEN))).await;
    assert_eq!(was.value(), Some("1.5"), "{}", was.text);
    verifies(&m, &was, BETWEEN);
    // the other class is untouched
    let k = crate_class();
    let b = bound(&m, &[k.as_str()], None).await;
    assert!(b[&k]["class_hash"].is_string(), "{b}");
}

/// A closed name may be bound again later: a new binding with its own `since` (`T3`); between the close and the rebinding it is
/// unbound; the stored cells (same function hashes) answer again, and the receipts of before the close still verify.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r2_a_closed_class_can_be_bound_again() {
    let m = shop_world("r07-close-again", true, true).await;
    let n = product();
    assert_eq!(bound(&m, &[n.as_str()], Some(MIDDLE)).await, json!({}), "unbound between the close and the rebinding");
    let now = bound(&m, &[n.as_str()], None).await;
    assert_eq!(now[&n]["since"], T3, "a new binding with its own since: {now}");
    let first = bound(&m, &[n.as_str()], Some(BETWEEN)).await;
    assert_eq!(first[&n]["since"], NOW, "the history keeps the old binding: {first}");
    let a = ask(&m, &path(&format!("{n}[apple].price({SHOP_DAY})"), None)).await;
    assert_eq!((a.outcome(), a.value()), ("ok", Some("1.5")), "{}", a.text);
    unbound_read(&ask(&m, &path(&format!("{n}[apple].price({SHOP_DAY})"), Some(MIDDLE))).await);
    let was = ask(&m, &path(&format!("{n}[apple].price({SHOP_DAY})"), Some(BETWEEN))).await;
    verifies(&m, &was, BETWEEN);
}

// ---------------------------------------------------------------------------------------------------------------------
// refusals

/// An unbound name cannot be closed: `E_NAME_UNBOUND`, at the line, and nothing is written; nor can a name that is already
/// closed.
#[test]
fn r2_closing_an_unbound_name_is_refused() {
    let lite = Lite::new("r07-close-unbound");
    publish(&lite, "functions.jsonl", &shop_functions(), BEFORE).unwrap_or_else(|e| panic!("{e}"));
    let mut v = shop_classes(&lite);
    v.extend(shop_cells());
    publish(&lite, "classes.jsonl", &v, NOW).unwrap_or_else(|e| panic!("{e}"));
    let e = refused_whole(&lite, "c1.jsonl", &[close_line(&format!("{HOST}/shop/nothing"))], 1, T2);
    assert!(e.detail.contains("E_NAME_UNBOUND"), "{e}");
    // a good close first, then the same close again
    publish(&lite, "c2.jsonl", &[close_line(&crate_class())], T2).unwrap_or_else(|e| panic!("{e}"));
    assert!(lite.store.resolve_class(&crate_class(), END).unwrap().is_none(), "closed");
    let e = refused_whole(&lite, "c3.jsonl", &[close_line(&crate_class())], 1, T3);
    assert!(e.detail.contains("E_NAME_UNBOUND"), "{e}");
    // a line after a refused one is not written: the file is atomic
    let e =
        refused_whole(&lite, "c4.jsonl", &[close_line(&product()), close_line(&format!("{HOST}/shop/nothing"))], 2, T3);
    assert!(e.detail.contains("E_NAME_UNBOUND"), "{e}");
    assert!(lite.store.resolve_class(&product(), END).unwrap().is_some(), "the valid close before it was not written");
}

/// A close is at the mesh's clock at the write, and is not backdated (9.7): a close whose write clock is before the binding's own
/// start is refused, and nothing is written; the name stays bound.
#[test]
fn r2_a_close_is_not_backdated() {
    let lite = Lite::new("r07-close-backdated");
    publish(&lite, "functions.jsonl", &shop_functions(), BEFORE).unwrap_or_else(|e| panic!("{e}"));
    let mut v = shop_classes(&lite);
    v.extend(shop_cells());
    publish(&lite, "classes.jsonl", &v, NOW).unwrap_or_else(|e| panic!("{e}"));
    // the control: a close at a clock after the binding's start is accepted (so the refusals below are the rule's)
    publish(&lite, "ok.jsonl", &[close_line(&crate_class())], T2).unwrap_or_else(|e| panic!("a close at T2: {e}"));
    assert!(lite.store.resolve_class(&crate_class(), END).unwrap().is_none(), "closed");
    refused_whole(&lite, "back.jsonl", &[close_line(&product())], 1, BEFORE);
    assert!(lite.store.resolve_class(&product(), END).unwrap().is_some(), "still bound");
    // an explicit earlier end is not a form of the line either
    let back = json!({"close": product(), "bound": BEFORE}).to_string();
    refused_whole(&lite, "back2.jsonl", &[back], 1, T2);
    assert!(lite.store.resolve_class(&product(), END).unwrap().is_some(), "still bound");
}
