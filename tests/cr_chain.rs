//! The classes round (CONTRACT-classes-round.md), K10: requests chain methods (SPEC 5.8, 16.6, 16.7). Failing tests: a call of
//! a class names one method (`E_PARSE_URI` otherwise).
//!
//! `N(k).m(a).n(b)`: each method after the first is called on the key the one before it returned (a method returning
//! `{"type": "string", "classes": [...]}`), a dispatch over those classes read as of the request's asof; the answer is the
//! envelope of the last cell. `resolved` gains one `chain` resolution per link, `{"asof", "from", "kind": "chain", "to"}`, in
//! order, after `lookup` and before `method`.
//!
//! The world is `common::crworld::shop`: `successor` returns a key of `shop/product` (A1 -> P1 -> Q1), A2's successor is a key of
//! another class (`shop/crate`), K1 has none, `name` returns a plain string.

mod common;
#[path = "common/crworld.rs"]
mod crworld;

use crworld::*;

fn call(tail: &str) -> String {
    format!("{}{tail}", product())
}

/// A two-link chain: `product(A1).successor().price(2026-03-16)` is the price of A1's successor P1 (3), the very cell of
/// `price(P1)`, and `resolved` says the chain link (the key it read, as of when) and then the method.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k10_a_two_link_chain() {
    let m = shop("cr-k10-two", false).await;
    let n = product();
    let plain = ask(&m, &format!("/cell/{n}/price?date={SHOP_DAY}&sku=P1")).await;
    assert_eq!(plain.value(), Some("3"), "{}", plain.text);
    for c in [format!("(A1).successor().price({SHOP_DAY})"), format!("(sku: A1).successor().price(date: {SHOP_DAY})")] {
        let a = ask(&m, &path(&call(&c), None)).await;
        assert_eq!((a.status, a.outcome(), a.value()), (200, "ok", Some("3")), "{c}: {}", a.text);
        assert_eq!(a.cell_id(), plain.cell_id(), "{c}: the answer is the envelope of the last cell");
        assert_eq!(a.v["receipt"], plain.v["receipt"], "{c}");
        assert_eq!(a.resolution_kinds(), ["chain", "method"], "{c}: {}", a.text);
        let rs = a.resolutions();
        assert_eq!(rs[0].0, "successor", "{c}: from is the method as written");
        assert_eq!(rs[0].2, pk("P1"), "{c}: to is the key encoding the cell held");
        assert_eq!(rs[1], (format!("{n}(...).price"), "method".to_string(), format!("{n}/price")), "{c}");
        assert!(
            a.v["resolved"]["resolutions"][0]["asof"].as_str().is_some_and(|t| t.len() == 27),
            "{c}: the link's asof"
        );
        assert_eq!(a.v["resolved"]["call"], format!("{n}/price(date: {SHOP_DAY}, sku: \"P1\")"), "{c}");
    }
    // the link is read as of the request's asof
    let at = "2026-11-15T12:30:00.000000Z";
    let a = ask(&m, &path(&call(&format!("(A1).successor().price({SHOP_DAY})")), Some(at))).await;
    assert_eq!(a.outcome(), "ok", "{}", a.text);
    assert_eq!(a.v["resolved"]["resolutions"][0]["asof"], at, "{}", a.text);
    // three links: A1 -> P1 -> Q1
    let q = ask(&m, &path(&call(&format!("(A1).successor().successor().price({SHOP_DAY})")), None)).await;
    assert_eq!((q.outcome(), q.value()), ("ok", Some("4")), "{}", q.text);
    assert_eq!(q.resolution_kinds(), ["chain", "chain", "method"], "{}", q.text);
    assert_eq!((q.resolutions()[0].2.clone(), q.resolutions()[1].2.clone()), (pk("P1"), pk("Q1")));
}

/// A chain from a label: the lookup comes first, then the links, then the method.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k10_a_chain_from_a_label() {
    let m = shop("cr-k10-label", false).await;
    let n = product();
    let a = ask(&m, &path(&call(&format!("[apple].successor().price({SHOP_DAY})")), None)).await;
    assert_eq!((a.status, a.outcome(), a.value()), (200, "ok", Some("3")), "{}", a.text);
    assert_eq!(a.resolution_kinds(), ["lookup", "chain", "method"], "{}", a.text);
    let rs = a.resolutions();
    assert_eq!((rs[0].0.as_str(), rs[0].2.as_str()), ("apple", pk("A1").as_str()));
    assert_eq!((rs[1].0.as_str(), rs[1].2.as_str()), ("successor", pk("P1").as_str()));
    assert_eq!(rs[2].2, format!("{n}/price"));
    // a folded label too
    let b = ask(&m, &path(&call(&format!("[APPLE].successor().price({SHOP_DAY})")), None)).await;
    assert_eq!(b.value(), Some("3"), "{}", b.text);
}

/// A link after a method that does not return a key is `E_PARSE_URI` (`name` returns a plain string; `price` a number).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k10_a_link_after_a_non_key_method_is_e_parse_uri() {
    let m = shop("cr-k10-nonkey", false).await;
    // the control: a link after a method that does return a key is a chain, so the refusals below are about the method
    let ok = ask(&m, &path(&call(&format!("(A1).successor().price({SHOP_DAY})")), None)).await;
    assert_eq!((ok.outcome(), ok.value()), ("ok", Some("3")), "{}", ok.text);
    for c in [
        format!("(A1).name().price({SHOP_DAY})"),
        format!("(A1).price({SHOP_DAY}).successor()"),
        format!("(A1).successor().name().price({SHOP_DAY})"),
    ] {
        let a = ask(&m, &path(&call(&c), None)).await;
        assert_eq!((a.status, a.outcome(), a.error()), (400, "bad_request", Some("E_PARSE_URI")), "{c}: {}", a.text);
    }
    // the single-method call is unchanged (a control, true today)
    let ok = ask(&m, &path(&call(&format!("(A1).price({SHOP_DAY})")), None)).await;
    assert_eq!(ok.value(), Some("1.5"));
}

/// An absent intermediate: a link whose previous cell has no value is that cell's outcome, `not_covered`, not an error.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k10_an_absent_intermediate_is_not_covered() {
    let m = shop("cr-k10-absent", false).await;
    let a = ask(&m, &path(&call(&format!("(K1).successor().price({SHOP_DAY})")), None)).await;
    assert_eq!((a.status, a.outcome()), (404, "not_covered"), "{}", a.text);
    assert_ne!(a.error(), Some("E_PARSE_URI"), "{}", a.text);
    assert!(a.v.get("receipt").is_none(), "the envelope of the cell that has no value: {}", a.text);
    // the same for a label whose object has no successor, and for a key with no cell at all
    let b = ask(&m, &path(&call(&format!("(ZZ9).successor().price({SHOP_DAY})")), None)).await;
    assert_eq!((b.status, b.outcome()), (404, "not_covered"), "{}", b.text);
    // the last cell absent is the last cell's own outcome: P1 has no price on another day
    let c = ask(&m, &path(&call("(A1).successor().price(2020-01-01)"), None)).await;
    assert_eq!((c.status, c.outcome()), (404, "not_covered"), "{}", c.text);
    assert_eq!(c.resolution_kinds(), ["chain", "method"], "{}", c.text);
}

/// A key outside the classes the method declares is the formula fault `E_FORMULA_DISPATCH_KEY`, a fault of the cell
/// (`not_covered`): A2's successor is a key of `shop/crate`, and `successor` declares `[shop/product]`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k10_a_key_outside_the_declared_classes_is_a_fault() {
    let m = shop("cr-k10-outside", false).await;
    let a = ask(&m, &path(&call(&format!("(A2).successor().price({SHOP_DAY})")), None)).await;
    assert_eq!((a.status, a.outcome(), a.fault()), (404, "not_covered", Some("E_FORMULA_DISPATCH_KEY")), "{}", a.text);
    // and the cells read directly are fine
    let s = ask(&m, &format!("/cell/{}/successor?sku=A2", product())).await;
    assert_eq!(s.value(), Some(format!(r#"{}(id:"c1")"#, crate_class()).as_str()), "{}", s.text);
}
