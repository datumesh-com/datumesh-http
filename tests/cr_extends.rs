//! The classes round (CONTRACT-classes-round.md), K9: `describe` of an object lists extension methods (SPEC 16.3, 16.8).
//! Failing tests: the catalogue line takes no `extends`, and an object answer has no `extensions`.
//!
//! A function's catalogue entry may carry `"extends": "<full class name>"`, the publisher's statement, outside every hash.
//! Publish checks that the function takes that class's keys first, in class order, with their types (the extension-method rule
//! of 16.3), else refuses it; the function may be under any namespace the mesh holds. An object answer of `describe` gains
//! `extensions`: `[{"fn", "fn_hash", "publisher"}]`, ascending by `fn`, for the functions that extend its class; an answer
//! without it is still valid. The world is `common::crworld::shop` (`shop/product`, key `sku`), with two extension functions
//! under other namespaces of the mesh, `alpha/margin(sku, pct)` and `zeta/note(sku)`.

mod common;
#[path = "common/crworld.rs"]
mod crworld;

use common::lite::*;
use common::pub031::{function_line, snapshot, BEFORE};
use common::readpath::*;
use common::world::call_path;
use crworld::*;
use serde_json::json;

fn margin() -> String {
    format!("{HOST}/alpha/margin")
}
fn note() -> String {
    format!("{HOST}/zeta/note")
}

fn margin_sig() -> String {
    format!(r#"{{"args":{{"pct":{{"type":"number","unit":"1","valid":false}},"sku":{STR}}},"order":["sku","pct"],"returns":{NUM1}}}"#)
}
fn note_sig() -> String {
    format!(r#"{{"args":{{"sku":{SKU_ARG}}},"order":["sku"],"returns":"string"}}"#)
}

fn extends(class: &str) -> serde_json::Value {
    json!({"extends": class})
}

/// The shop, then (at `T2`) the two extension functions with their catalogue lines.
fn lite_with_shop(label: &str) -> Lite {
    let lite = Lite::new(label);
    publish(&lite, "functions.jsonl", &shop_functions(), BEFORE).unwrap_or_else(|e| panic!("functions: {e}"));
    let mut v = shop_classes(&lite);
    v.extend(shop_cells());
    publish(&lite, "classes.jsonl", &v, NOW).unwrap_or_else(|e| panic!("classes and cells: {e}"));
    lite
}

fn extension_lines(class: &str) -> Vec<String> {
    vec![
        function_line(&margin(), &margin_sig(), None),
        catalog_line_of(&margin(), "function", extends(class)),
        function_line(&note(), &note_sig(), None),
        catalog_line_of(&note(), "function", extends(class)),
    ]
}

async fn shop_with_extensions(label: &str) -> Served {
    let lite = lite_with_shop(label);
    publish(&lite, "ext.jsonl", &extension_lines(&product()), T2).unwrap_or_else(|e| panic!("extends that matches: {e}"));
    build_served(lite, None, |_| {}).await
}

fn describe_path(id: &str) -> String {
    format!("/describe/{}", call_path(id))
}

/// `extends` that matches is accepted (the keys of the class first, in class order, their types; a key rule on the function's
/// argument or none), under namespaces other than the class's; and it is outside every hash: the functions' hashes are what
/// they are without it.
#[test]
fn k9_extends_that_matches_is_accepted() {
    let lite = lite_with_shop("cr-k9-accept");
    let p = publish(&lite, "ext.jsonl", &extension_lines(&product()), T2).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(p.functions, 2, "{p:?}");
    // a second run changes nothing
    let before = snapshot(&lite);
    let again = publish(&lite, "ext.jsonl", &extension_lines(&product()), T2).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(again.functions, 0, "{again:?}");
    assert_eq!(snapshot(&lite), before);
    // outside every hash: the same functions without the catalogue have the same hashes
    let bare = lite_with_shop("cr-k9-bare");
    publish(&bare, "ext.jsonl", &[function_line(&margin(), &margin_sig(), None), function_line(&note(), &note_sig(), None)], T2).unwrap();
    for f in [margin(), note()] {
        assert_eq!(fn_hash(&lite, &f), fn_hash(&bare, &f), "{f}");
    }
}

/// `extends` that does not match the keys is refused at the catalogue line and nothing is written: the first argument has
/// another name, another type, or the key is not first; the class is not bound; the entry is a class's.
#[test]
fn k9_extends_that_does_not_match_is_refused() {
    let sku_first = |args: &str, order: &str| format!(r#"{{"args":{args},"order":{order},"returns":{NUM1}}}"#);
    let num = r#"{"type":"number","unit":"1","valid":false}"#;
    let cases: Vec<(&str, String, String)> = vec![
        ("another name", sku_first(&format!(r#"{{"item":{STR}}}"#), r#"["item"]"#), product()),
        ("another type", sku_first(&format!(r#"{{"sku":{num}}}"#), r#"["sku"]"#), product()),
        ("no key", sku_first(&format!(r#"{{"pct":{num}}}"#), r#"["pct"]"#), product()),
        ("the key not first", sku_first(&format!(r#"{{"pct":{num},"sku":{STR}}}"#), r#"["pct","sku"]"#), product()),
        ("a class not bound", margin_sig(), format!("{HOST}/shop/ghost")),
    ];
    for (i, (what, sig, class)) in cases.into_iter().enumerate() {
        let lite = lite_with_shop(&format!("cr-k9-refuse-{i}"));
        let before = snapshot(&lite);
        let lines = [function_line(&margin(), &sig, None), catalog_line_of(&margin(), "function", extends(&class))];
        let e = publish(&lite, "ext.jsonl", &lines, T2).expect_err(what);
        assert_eq!(e.line, 2, "{what}: refused at the catalogue line: {e}");
        assert_eq!(snapshot(&lite), before, "{what}: nothing is written");
    }
    // `extends` is for a function entry: a class entry that has it is refused
    let lite = lite_with_shop("cr-k9-refuse-class");
    let before = snapshot(&lite);
    let e = publish(&lite, "ext.jsonl", &[catalog_line_of(&crate_class(), "class", extends(&product()))], T2).expect_err("a class entry");
    assert_eq!(e.line, 1, "{e}");
    assert_eq!(snapshot(&lite), before);
    // the control: the same function with a matching key is accepted
    let lite = lite_with_shop("cr-k9-refuse-control");
    publish(&lite, "ext.jsonl", &extension_lines(&product()), T2).unwrap_or_else(|e| panic!("{e}"));
}

/// `describe` of an object lists the extension: `fn`, `fn_hash` (the function's hash as bound), `publisher` (the host of `fn`),
/// ascending by `fn`; an object of a class that nothing extends has none.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k9_describe_of_an_object_lists_the_extensions() {
    let m = shop_with_extensions("cr-k9-describe").await;
    for object in [pk("A1"), pk("ZZ9")] {
        let a = ask(&m, &describe_path(&object)).await;
        assert_eq!((a.status, a.kind()), (200, Some("object")), "{}", a.text);
        let ext = a.v["extensions"].as_array().unwrap_or_else(|| panic!("no extensions: {}", a.text));
        assert_eq!(
            ext.iter().map(|e| e["fn"].as_str().unwrap()).collect::<Vec<_>>(),
            [margin().as_str(), note().as_str()],
            "ascending by fn: {}",
            a.text
        );
        for e in ext {
            let f = e["fn"].as_str().unwrap();
            assert_eq!(e["fn_hash"].as_str(), Some(fn_hash(&m.lite, f).as_str()), "{f}");
            assert_eq!(e["publisher"], HOST, "{f}: the publisher is the host of fn");
            assert_eq!(e.as_object().unwrap().len(), 3, "exactly fn, fn_hash, publisher: {e}");
        }
    }
    // by label, too (K8): the same object, the same extensions
    let by_label = ask(&m, &describe_path(&format!("{}[apple]", product()))).await;
    assert_eq!(by_label.v["extensions"], ask(&m, &describe_path(&pk("A1"))).await.v["extensions"], "{}", by_label.text);
    // an object of a class that nothing extends has no `extensions`
    let c = ask(&m, &describe_path(&format!(r#"{}(id:"c1")"#, crate_class()))).await;
    assert_eq!((c.status, c.kind()), (200, Some("object")), "{}", c.text);
    assert!(c.v.get("extensions").is_none(), "{}", c.text);
    // a class answer is the class's own statement: no `extensions`
    let class = ask(&m, &describe_path(&product())).await;
    assert_eq!(class.kind(), Some("class"), "{}", class.text);
    assert!(class.v.get("extensions").is_none(), "{}", class.text);
}

/// An answer without `extensions` is still valid: before an extension is published, and for a class nothing extends, the
/// object answer has exactly the members of 16.8 and no `extensions` (a control that holds today; the schema itself is the
/// spec submodule's, which this worktree does not hold, so the member set is checked here).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k9_an_answer_without_extensions_is_valid() {
    let lite = lite_with_shop("cr-k9-plain");
    let m = build_served(lite, None, |_| {}).await;
    for object in [pk("A1"), format!(r#"{}(id:"c1")"#, crate_class())] {
        let a = ask(&m, &describe_path(&object)).await;
        assert_eq!((a.status, a.kind()), (200, Some("object")), "{}", a.text);
        assert!(a.v.get("extensions").is_none(), "{}", a.text);
        let mut keys: Vec<&str> = a.v.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["call", "kind", "methods", "object", "object_id", "short_id"], "{}", a.text);
    }
}
