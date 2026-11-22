//! The catalogue, phase 1 (CONTRACT-classes-round.md), C1: the reserved `catalog` namespace.
//!
//! `<host>/catalog/...` (and `<host>/catalog` itself) is the mesh's own, as `series`, `fn`, `takeover` and `x` are
//! (CONTRACT-0.4.9 H8): `publish-stored` and bundle import (the command and `POST /bundles`) refuse such a name with
//! `E_NAME_RESERVED`, the file or bundle whole, nothing written. The one exception is the catalogue job's own run (C5), signed
//! by the mesh's key with the job flag; the run itself is the test of that exception (`cat_build.rs`), and here only that the
//! exception does not outlive it. Look-alikes (`catalogue`, `catalog_x`, `h/catalog`) and the names of other hosts are not
//! reserved.
//!
//! Choices where the contract is silent: `declare` is not a write path of this item (its names are functions of the mesh, not
//! catalogue names) and has no test here.

mod common;
#[path = "common/crworld.rs"]
mod crworld;

use common::lite::*;
use common::pub031::{function_line, rate_sig, refused_whole, run, snapshot, unbound, write};
use common::*;
use datumesh_http::{bundle, Config};
use datumesh_store::Store;
use serde_json::json;

const CODE: &str = "E_NAME_RESERVED";

fn reserved() -> Vec<String> {
    vec![
        format!("{HOST}/catalog"),
        format!("{HOST}/catalog/a"),
        format!("{HOST}/catalog/subject"),
        format!("{HOST}/catalog/subject/source"),
        format!("{HOST}/catalog/deep/er/a"),
    ]
}

fn fine() -> Vec<String> {
    vec![
        format!("{HOST}/catalogue/a"),
        format!("{HOST}/catalog_x/a"),
        format!("{HOST}/catalogs/a"),
        format!("{HOST}/h/catalog/a"),
    ]
}

fn split(name: &str) -> (String, String) {
    let (ns, f) = name.rsplit_once('/').unwrap();
    (ns.to_string(), f.to_string())
}

// ---- publish-stored ----------------------------------------------------------------------------

/// A function line of a reserved name, on line 2 of a file whose line 1 is good: the file is refused whole at line 2 with the
/// code, and the good function of line 1 is not written either.
#[test]
fn c1_publish_stored_refuses_a_reserved_function_whole() {
    for name in reserved() {
        let lite = Lite::new("c1-publish");
        let ok = format!("{HOST}/c1/ok/level");
        let lines = [function_line(&ok, &rate_sig(), None), function_line(&name, &rate_sig(), None)];
        let e = refused_whole(&lite, "f.jsonl", &lines, 2, NOW);
        assert!(e.detail.contains(CODE), "{name}: {e}");
        assert!(unbound(&lite, &ok), "{name}: the good function was written");
        assert!(unbound(&lite, &name), "{name}");
    }
}

/// Every other line kind that names a catalogue-namespace name is refused as well: a cell line, a formula line, a class line, a
/// catalogue line (of a good name, whose facets are not the point).
#[test]
fn c1_every_line_kind_is_refused() {
    let name = format!("{HOST}/catalog/x");
    let formula =
        crworld::formula_line(&name, "1 + 1", r#"{"args":{},"order":[],"returns":{"type":"number","unit":"1"}}"#);
    let cell = common::pub031::rate_cell(&name, "1", None, None, None);
    let class = {
        let d = common::pub031::pair_class(&[]);
        common::pub031::class_line(&name, &d, None)
    };
    let cat = crworld::catalog_line_of(&name, "function", json!({}));
    for (what, line) in [("formula", formula), ("cell", cell), ("class", class), ("catalog", cat)] {
        let lite = Lite::new("c1-kinds");
        let f = write(&lite, "f.jsonl", &[line]);
        let before = snapshot(&lite);
        let e = run(&lite, &f, NOW).expect_err(&format!("{what} line of a reserved name must be refused"));
        assert_eq!(e.line, 1, "{what}: {e}");
        assert!(e.detail.contains(CODE), "{what}: {e}");
        assert_eq!(snapshot(&lite), before, "{what}: wrote something");
    }
}

#[test]
fn c1_publish_stored_takes_names_that_only_look_alike() {
    for name in fine() {
        let lite = Lite::new("c1-fine");
        let f = write(&lite, "f.jsonl", &[function_line(&name, &rate_sig(), None)]);
        run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{name} is not reserved: {e}"));
        assert!(!unbound(&lite, &name), "{name}");
    }
}

#[test]
fn c1_the_names_of_other_hosts_are_not_reserved() {
    let lite = Lite::new("c1-other");
    let name = "other.example/catalog/a";
    let f = write(&lite, "f.jsonl", &[function_line(name, &rate_sig(), None)]);
    // refused or not for other reasons (the mesh does not own that host), never as reserved
    if let Err(e) = run(&lite, &f, NOW) {
        assert!(!e.detail.contains(CODE), "{e}");
    }
}

// ---- import-signed -----------------------------------------------------------------------------

fn stored_fn() -> datumesh_core::FunctionDescriptor {
    datumesh_core::FunctionDescriptor::from_json(common::pub031::stored_descriptor(&rate_sig()).as_bytes()).unwrap()
}

fn bundle_of(ns: &str, name: &str) -> Vec<u8> {
    bundle::build_with_catalog(
        ns,
        vec![stored_fn().canonical()],
        vec![(name.to_string(), stored_fn().hash())],
        vec![],
        vec![],
        &seed_for(HOST),
    )
    .unwrap()
}

fn import(lite: &Lite, bytes: &[u8]) -> Result<bundle::Imported, bundle::BundleError> {
    let owned: Vec<String> = Vec::new();
    bundle::import_signed(&*lite.store, HOST, &owned, &FixedClock(NOW), bytes)
}

/// A bundle whose namespace or names are under `<host>/catalog` is refused whole, nothing written (a bundle's namespace needs a
/// segment after the host, so `<host>/catalog` is a namespace and `<host>/catalog/..` names are under it).
#[test]
fn c1_import_signed_refuses_a_reserved_name_whole() {
    for name in reserved().into_iter().filter(|n| n.matches('/').count() >= 2) {
        let (ns, _) = split(&name);
        let Ok(b) = std::panic::catch_unwind(|| bundle_of(&ns, &name)) else { continue };
        let lite = Lite::new("c1-import");
        let before = snapshot(&lite);
        let e = import(&lite, &b).expect_err(&format!("{name} must be refused"));
        assert!(format!("{e}").contains(CODE), "{name}: {e}");
        assert!(lite.store.resolve_name(&name, END).map_or(true, |b| b.is_none()), "{name} is bound");
        assert_eq!(snapshot(&lite), before, "{name}: wrote something");
    }
}

#[test]
fn c1_import_signed_still_takes_ordinary_names() {
    for name in fine() {
        let (ns, _) = split(&name);
        let lite = Lite::new("c1-import-fine");
        import(&lite, &bundle_of(&ns, &name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(lite.store.resolve_name(&name, END).unwrap().is_some(), "{name}");
    }
}

// ---- POST /bundles -----------------------------------------------------------------------------

fn config(log: &std::path::Path) -> Config {
    let env = [
        ("DATUMESH_HOST".to_string(), HOST.to_string()),
        ("DATUMESH_KEY_NAMES".to_string(), "1".to_string()),
        ("DATUMESH_LOG_DIR".to_string(), log.to_string_lossy().into_owned()),
    ];
    Config::from_vars(move |k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).unwrap()
}

async fn post(m: &common::readpath::Served, body: Vec<u8>) -> (u16, serde_json::Value) {
    let r = m
        .http
        .post(format!("{}/bundles", m.base))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    let b = r.bytes().await.unwrap();
    (status, serde_json::from_slice(&b).unwrap_or(serde_json::Value::Null))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c1_post_bundles_refuses_a_reserved_name_whole() {
    let lite = Lite::new("c1-post");
    let log = lite.path("logs");
    std::fs::create_dir_all(&log).unwrap();
    let m = common::readpath::build_served(lite, Some(config(&log)), |_| {}).await;
    for name in reserved().into_iter().filter(|n| n.matches('/').count() >= 2) {
        let (ns, _) = split(&name);
        let Ok(b) = std::panic::catch_unwind(|| bundle_of(&ns, &name)) else { continue };
        let (status, j) = post(&m, b).await;
        assert_eq!(status, 422, "{name}: {j}");
        assert_eq!(j["error"], CODE, "{name}: {j}");
        assert!(j.get("detail").is_some(), "{name}: {j}");
        assert!(m.lite.store.resolve_name(&name, END).map_or(true, |b| b.is_none()), "{name} is bound");
    }
    let name = format!("{HOST}/catalogue/a");
    let (ns, _) = split(&name);
    let (status, j) = post(&m, bundle_of(&ns, &name)).await;
    assert_eq!(status, 201, "{j}");
}
