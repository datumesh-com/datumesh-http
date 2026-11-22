//! The classes round (CONTRACT-classes-round.md), K5 (BREAKING): an inherited method reads the cells of the class that defines
//! it (SPEC 16.2, 16.3, 16.6; the vectors `formula/class-inherited*`). Failing tests: the mesh still reads `S/m`.
//!
//! The world is `common::crworld::rates`: the parent `rates/fx_pair` (keys base, quote; methods `rate` stored, `cross` and
//! `inverse` computed) and its subclass `rates/fx_ndf` (parent `fx_pair`; adds the key `fixing`, the stored method `fixed`,
//! and overrides `cross`). Cells exist for `fx_pair/rate` only: EUR/USD 1.2 and EUR/GBP 0.8 on 2026-03-02.
//!
//! * a subclass object reading an inherited stored method gets the defining class's cells (no override needed), and an
//!   inherited computed one the defining class's computed cell;
//! * `resolved` says `method` -> `D/m`; an override stays `S/m`; `describe` of the class names `D/m` for an inherited method;
//! * in a body (a dispatch, an object with literal keys) the read is `D/m`: the receipt's inputs hold D's cell;
//! * binding the subclass binds only its own methods: `S/rate` is unbound, a plain read of it is `E_NAME_UNBOUND` in a request
//!   and `E_FORMULA_UNRESOLVED` in a body;
//! * D is found through the names bound to the parent hash as of the request's asof: with none bound it is `E_NAME_UNBOUND`.

mod common;
#[path = "common/crworld.rs"]
mod crworld;

use common::lite::*;
use common::readpath::*;
use crworld::*;
use datumesh_store::Store;

/// The cell id of the plain read `name?...` as the mesh answers it.
async fn plain_id(m: &Served, name: &str, query: &str) -> (String, Ans) {
    let a = ask(m, &format!("/cell/{name}?{query}")).await;
    assert_eq!(a.outcome(), "ok", "{name}: {}", a.text);
    (a.cell_id(), a)
}

/// A stored method inherited, read through the subclass: the parent's cell, by the same receipt, with `resolved` saying
/// `method` -> `fx_pair/rate`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_inherited_stored_method_reads_the_defining_classes_cells() {
    let m = rates("cr-k5-stored", false).await;
    let (p, s) = (rates_p(), rates_s());
    let (id, plain) = plain_id(&m, &format!("{p}/rate"), &format!("base=EUR&date={DAY}&quote=USD")).await;
    assert_eq!(plain.value(), Some("1.2"));
    // the three keys of the subclass's object, the method's date: the cell of P/rate, not an empty cell of S/rate
    for call in
        [format!("{s}(EUR, USD, F1).rate({DAY})"), format!("{s}(base: EUR, quote: USD, fixing: F1).rate(date: {DAY})")]
    {
        let a = ask(&m, &path(&call, None)).await;
        assert_eq!((a.status, a.outcome(), a.value()), (200, "ok", Some("1.2")), "{call}: {}", a.text);
        assert_eq!(a.cell_id(), id, "{call}: the cell of the defining class");
        assert_eq!(a.v["receipt"], plain.v["receipt"], "{call}: one receipt");
        let ms: Vec<_> = a.resolutions().into_iter().filter(|r| r.1 == "method").collect();
        assert_eq!(ms, vec![(format!("{s}(...).rate"), "method".into(), format!("{p}/rate"))], "{call}");
    }
    // another key of the subclass shares the parent's cell: the fixing is not part of it
    let a = ask(&m, &path(&format!("{s}(EUR, USD, F2).rate({DAY})"), None)).await;
    assert_eq!((a.outcome(), a.value()), ("ok", Some("1.2")), "{}", a.text);
    assert_eq!(a.cell_id(), id);
}

/// A computed method inherited: the computed cell of the defining class (its name and its hash are the parent's), the same
/// value, and `resolved` says `method` -> `fx_pair/inverse`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_inherited_computed_method_reads_the_defining_classes_computed_cell() {
    let m = rates("cr-k5-computed", false).await;
    let (p, s) = (rates_p(), rates_s());
    let plain = ask(&m, &format!("/cell/{p}/inverse?base=EUR&date={DAY}&quote=USD")).await;
    assert_eq!(plain.outcome(), "ok", "{}", plain.text);
    let via = ask(&m, &path(&format!("{s}(EUR, USD, F1).inverse({DAY})"), None)).await;
    assert_eq!(via.outcome(), "ok", "{}", via.text);
    assert_eq!(via.cell_id(), plain.cell_id(), "the parent's computed cell, not S/inverse");
    assert_eq!(via.value(), plain.value());
    assert!(
        via.resolutions().contains(&(format!("{s}(...).inverse"), "method".into(), format!("{p}/inverse"))),
        "{}",
        via.text
    );
}

/// An override stays the subclass's own: `S/cross`, a cell of its own function (twice the parent's cross, 3), with `method` ->
/// `fx_ndf/cross`; the subclass's own `fixed` is `S/fixed` as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_override_stays_the_subclasss_own() {
    let m = rates("cr-k5-override", false).await;
    let (p, s) = (rates_p(), rates_s());
    let via = ask(&m, &path(&format!("{s}(GBP, USD, F1).cross({DAY})"), None)).await;
    assert_eq!((via.outcome(), via.value()), ("ok", Some("3")), "{}", via.text);
    let own = ask(&m, &format!("/cell/{s}/cross?base=GBP&date={DAY}&quote=USD")).await;
    assert_eq!(via.cell_id(), own.cell_id(), "the cell of S/cross");
    assert!(
        via.resolutions().contains(&(format!("{s}(...).cross"), "method".into(), format!("{s}/cross"))),
        "{}",
        via.text
    );
    // the parent's own cross is untouched: 1.5
    let parent = ask(&m, &path(&format!("{p}(GBP, USD).cross({DAY})"), None)).await;
    assert_eq!(parent.value(), Some("1.5"), "{}", parent.text);
    // an own stored method: no cell, but the name is S's
    let fixed = ask(&m, &path(&format!("{s}(EUR, USD, F1).fixed({DAY})"), None)).await;
    assert!(
        fixed.resolutions().contains(&(format!("{s}(...).fixed"), "method".into(), format!("{s}/fixed"))),
        "{}",
        fixed.text
    );
}

/// In a body: a dispatch over `[fx_ndf]` (`pair.rate(date: date)`) and an object with literal keys
/// (`fx_ndf("EUR", "USD", "F1").rate(date: date)`) read the cell of `fx_pair/rate`: it is the receipt's one input.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_body_reads_the_defining_class_in_a_dispatch_and_in_an_object() {
    let m = rates_with("cr-k5-body", false, true).await;
    let (p, s) = (rates_p(), rates_s());
    let (_id, plain) = plain_id(&m, &format!("{p}/rate"), &format!("base=EUR&date={DAY}&quote=USD")).await;
    let d = ask(&m, &path(&format!("{}(pair: {s}(EUR, USD, F1), date: {DAY})", via_pair()), None)).await;
    assert_eq!((d.outcome(), d.value()), ("ok", Some("1.2")), "dispatch: {}", d.text);
    assert_eq!(d.inputs(), vec![plain.receipt_hash()], "the dispatch read the parent's cell: its receipt is the input");
    let o = ask(&m, &path(&format!("{}(date: {DAY})", via_object()), None)).await;
    assert_eq!((o.outcome(), o.value()), ("ok", Some("1.2")), "object: {}", o.text);
    assert_eq!(o.inputs(), vec![plain.receipt_hash()], "the object read the parent's cell: its receipt is the input");
}

/// Binding a subclass binds only its own methods: `fx_ndf/cross` and `fx_ndf/fixed`, not `fx_ndf/rate` or `/inverse`.
/// The plain read of an inherited method's name under the subclass is `E_NAME_UNBOUND` in a request, and in a body
/// `E_FORMULA_UNRESOLVED` (the file is refused and nothing is written).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn binding_a_subclass_binds_only_its_own_methods() {
    let m = rates("cr-k5-binding", false).await;
    let (p, s) = (rates_p(), rates_s());
    let bound = |n: &str| m.lite.store.resolve_name(n, END).unwrap().is_some();
    for own in ["cross", "fixed"] {
        assert!(bound(&format!("{s}/{own}")), "{s}/{own} is the subclass's own");
    }
    for inherited in ["rate", "inverse"] {
        assert!(!bound(&format!("{s}/{inherited}")), "{s}/{inherited} is inherited: not bound under the subclass");
        assert!(bound(&format!("{p}/{inherited}")), "{p}/{inherited}");
    }
    let a = ask(&m, &format!("/cell/{s}/rate?base=EUR&date={DAY}&quote=USD")).await;
    assert_eq!((a.status, a.outcome(), a.error()), (404, "not_covered", Some("E_NAME_UNBOUND")), "{}", a.text);
    // in a body
    let sig = format!(r#"{{"args":{{"date":{DATE_V}}},"order":["date"],"returns":{NUM1}}}"#);
    let f = formula_line(
        &format!("{HOST}/rates/plain_inherited"),
        &format!(r#"{s}/rate(base: "EUR", quote: "USD", date: date)"#),
        &sig,
    );
    let e = publish(&m.lite, "plain.jsonl", &[f], NOW).expect_err("the plain read of an inherited method is refused");
    assert!(e.detail.contains("E_FORMULA_UNRESOLVED"), "{e}");
}

/// `describe` of the subclass names `D/m` as the function of an inherited method (16.8): `rate` and `inverse` are the
/// parent's, `cross` and `fixed` the subclass's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describe_of_the_class_names_the_defining_class() {
    let m = rates("cr-k5-describe", false).await;
    let (p, s) = (rates_p(), rates_s());
    let a = ask(&m, &format!("/describe/{}", common::world::call_path(&s))).await;
    assert_eq!((a.status, a.kind()), (200, Some("class")), "{}", a.text);
    let fn_of = |name: &str| a.v["methods"][name]["fn"].as_str().map(str::to_string);
    assert_eq!(fn_of("rate"), Some(format!("{p}/rate")), "{}", a.text);
    assert_eq!(fn_of("inverse"), Some(format!("{p}/inverse")));
    assert_eq!(fn_of("cross"), Some(format!("{s}/cross")));
    assert_eq!(fn_of("fixed"), Some(format!("{s}/fixed")));
}

/// D is found through the names bound to the parent's hash as of the request's asof. After a second run binds the parent's
/// name to another descriptor, no name is bound to the hash the subclass's `parent` holds: a request as of now is
/// `E_NAME_UNBOUND`; as of an instant between the runs, it reads the parent's cell as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unbound_defining_class_is_e_name_unbound() {
    let m = rates("cr-k5-unbound", true).await;
    let s = rates_s();
    let call = format!("{s}(EUR, USD, F1).rate({DAY})");
    let then = ask(&m, &path(&call, Some(BETWEEN))).await;
    assert_eq!((then.outcome(), then.value()), ("ok", Some("1.2")), "as of {BETWEEN}: {}", then.text);
    for asof in [None, Some(T2)] {
        let now = ask(&m, &path(&call, asof)).await;
        assert_eq!(
            (now.status, now.outcome(), now.error()),
            (404, "not_covered", Some("E_NAME_UNBOUND")),
            "{asof:?}: {}",
            now.text
        );
    }
}
