//! The classes round (CONTRACT-classes-round.md), K8: labels (SPEC 16.1, 16.4, 16.7, 16.8). Failing tests: a request's label is
//! not folded, the rule `label_fold` is not applied by publish, a withdrawn label is not a refusal, and `describe` and a bare
//! `N[label]` at `/cell` are not served.
//!
//! The world is `common::crworld::shop`: the class `shop/product` (key `sku` under `upper_ascii`, so that keys can be shown
//! to stay strict) with the lookup `from_label`, whose `label` argument is under `label_fold`. Its cells: `apple` -> A1,
//! `green apple` -> A2, `strasse` -> S1 and `old` -> A3 (withdrawn at T2 in the second world).

mod common;
#[path = "common/crworld.rs"]
mod crworld;

use common::lite::*;
use common::pub031::{snapshot, BEFORE};
use common::world::call_path;
use crworld::*;

fn price_call(label: &str) -> String {
    format!("{}[{label}].price({SHOP_DAY})", product())
}

/// `[apple]`, `[  Apple   ]`, `[APPLE]` and the quoted spellings resolve to one object: the cell of `price(A1)`, by one
/// receipt, and `resolved` says the lookup was made (its `from` the label as written, its `to` the key encoding). A label that
/// folds to a stored one with a space (`green apple`) and one that needs folding beyond ASCII (`Straße`, `strasse`) resolve.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k8_labels_are_folded_in_a_request() {
    let m = shop("cr-k8-fold", false).await;
    let n = product();
    let plain = ask(&m, &format!("/cell/{n}/price?date={SHOP_DAY}&sku=A1")).await;
    assert_eq!((plain.outcome(), plain.value()), ("ok", Some("1.5")), "{}", plain.text);
    for label in ["apple", "Apple", "APPLE", "  Apple   ", r#""apple""#, r#""  Apple   ""#, r#""APPLE""#] {
        let a = ask(&m, &path(&price_call(label), None)).await;
        assert_eq!((a.status, a.outcome(), a.value()), (200, "ok", Some("1.5")), "[{label}]: {}", a.text);
        assert_eq!(a.cell_id(), plain.cell_id(), "[{label}]");
        let rs = a.resolutions();
        assert_eq!(rs[0].1, "lookup", "[{label}]: {rs:?}");
        assert_eq!(rs[0].2, pk("A1"), "[{label}]: the key encoding the lookup read");
    }
    let a = ask(&m, &path(&price_call("APPLE"), None)).await;
    assert_eq!(a.resolutions()[0].0, "APPLE", "the lookup's from is the label as written");
    // a label with spaces: a run of white space (spaces and NBSP) is one space, ends trimmed, case folded
    let green = ask(&m, &format!("/cell/{n}/price?date={SHOP_DAY}&sku=A2")).await;
    for label in [r#""green apple""#, r#""Green   APPLE""#, r#""  green  apple ""#, "\"GREEN\u{a0}APPLE\"", "\"green \u{a0} \u{a0}apple\"", "\"\u{a0}Green\u{a0}\u{a0}apple  \""] {
        let a = ask(&m, &path(&price_call(label), None)).await;
        assert_eq!((a.outcome(), a.value()), ("ok", Some("2.5")), "[{label}]: {}", a.text);
        assert_eq!(a.cell_id(), green.cell_id(), "[{label}]");
    }
    // a control character is refused by the string rule (3.5) before any folding: K8 repairs label_fold only, and
    // keeps every other rule strict. A TAB in a request label is E_CANON_STRING, not a space.
    let tab = ask(&m, &path(&price_call("\"green\tapple\""), None)).await;
    assert_eq!(tab.error(), Some("E_CANON_STRING"), "{}", tab.text);
    assert_ne!(tab.status, 200, "{}", tab.text);
    // full case folding, then NFC
    let s1 = ask(&m, &format!("/cell/{n}/price?date={SHOP_DAY}&sku=S1")).await;
    for label in ["\"Stra\u{df}e\"", "\"STRASSE\"", "\"strasse\""] {
        let a = ask(&m, &path(&price_call(label), None)).await;
        assert_eq!((a.outcome(), a.value()), ("ok", Some("9")), "[{label}]: {}", a.text);
        assert_eq!(a.cell_id(), s1.cell_id(), "[{label}]");
    }
}

/// A publisher stores folded labels: a label that is not its own folding is refused at publish, `E_KEY_NORMALISATION`, at its
/// line, and nothing is written; the folded one is accepted.
#[test]
fn k8_a_stored_unfolded_label_is_refused() {
    for (i, bad) in ["Apple", "green  apple", " apple", "apple ", "Stra\u{df}e", "APPLE\u{a0}PIE"].into_iter().enumerate() {
        let lite = Lite::new(&format!("cr-k8-stored-{i}"));
        publish(&lite, "functions.jsonl", &shop_functions(), BEFORE).unwrap_or_else(|e| panic!("functions: {e}"));
        let mut v = shop_classes(&lite);
        v.push(label_cell("apple", &pk("A1")));
        publish(&lite, "classes.jsonl", &v, NOW).unwrap_or_else(|e| panic!("a folded label: {e}"));
        let before = snapshot(&lite);
        let e = publish(&lite, "bad.jsonl", &[label_cell(bad, &pk("A1"))], NOW).expect_err("an unfolded label is refused");
        assert_eq!(e.line, 1, "{bad:?}: {e}");
        assert!(e.detail.contains("E_KEY_NORMALISATION"), "{bad:?}: {e}");
        assert_eq!(snapshot(&lite), before, "{bad:?}: nothing written");
    }
}

/// Keys are never folded: a key not canonical under its rule is `E_KEY_NORMALISATION`, `bad_request`, in a method call and in
/// the plain read of a call, as before (the cell URI form is not said, see classes_requests.rs); and the label's repair does not reach it (`product(a1)` is not `product(A1)`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k8_keys_are_not_folded() {
    let m = shop("cr-k8-keys", false).await;
    let n = product();
    for path in [
        path(&format!("{n}(a1).price({SHOP_DAY})"), None),
        path(&format!("{n}(\"a1\").price({SHOP_DAY})"), None),
        path(&format!("{n}/price(a1, {SHOP_DAY})"), None),
    ] {
        let a = ask(&m, &path).await;
        assert_eq!((a.status, a.outcome(), a.error()), (400, "bad_request", Some("E_KEY_NORMALISATION")), "{path}: {}", a.text);
    }
    // and the control: the folded key is a key
    let ok = ask(&m, &path(&format!("{n}(A1).price({SHOP_DAY})"), None)).await;
    assert_eq!(ok.value(), Some("1.5"));
    // the lookup's label is the only repaired value: a label read through the plain read of the lookup is a stored label, so an
    // unfolded one is refused there too (it is an argument under label_fold: it is repaired by the mesh in requests)
    let a = ask(&m, &format!("/cell/{n}/from_label?label=apple")).await;
    assert_eq!(a.value(), Some(pk("A1").as_str()), "{}", a.text);
}

/// A label is withdrawn by a new version of its lookup cell whose value is the empty string: as of now the label is
/// `E_NAME_UNBOUND` as an unknown one is; as of an instant before the withdrawal it still names the object (a cell's history
/// stays).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k8_a_withdrawn_label_is_e_name_unbound() {
    let m = shop("cr-k8-withdrawn", true).await;
    let was = ask(&m, &path(&price_call("old"), Some(BETWEEN))).await;
    assert_eq!((was.outcome(), was.value()), ("ok", Some("7")), "as of {BETWEEN}: {}", was.text);
    for asof in [None, Some(T2)] {
        let a = ask(&m, &path(&price_call("old"), asof)).await;
        assert_eq!((a.status, a.outcome(), a.error()), (404, "not_covered", Some("E_NAME_UNBOUND")), "{asof:?}: {}", a.text);
    }
    // an unknown label is the same, and a label that was not withdrawn is unaffected
    let nope = ask(&m, &path(&price_call("nope"), None)).await;
    assert_eq!((nope.status, nope.error()), (404, Some("E_NAME_UNBOUND")), "{}", nope.text);
    let apple = ask(&m, &path(&price_call("apple"), None)).await;
    assert_eq!(apple.value(), Some("1.5"));
    // the lookup cell itself holds the empty string now
    let cell = ask(&m, &format!("/cell/{}/from_label?label=old", product())).await;
    assert_eq!((cell.outcome(), cell.value()), ("ok", Some("")), "{}", cell.text);
}

/// `describe/{id}` accepts `N[label]` and answers the object's describe, with `resolved` (the lookup resolution, whose `to` is
/// the object's key encoding and whose `asof` is the instant the lookup was read as of); a describe by key has no `resolved`.
/// A folded spelling gives the same object; a withdrawn label is not found.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k8_describe_by_label() {
    let m = shop("cr-k8-describe", true).await;
    let n = product();
    let by_key = ask(&m, &format!("/describe/{}", call_path(&pk("A1")))).await;
    assert_eq!((by_key.status, by_key.kind()), (200, Some("object")), "{}", by_key.text);
    assert!(by_key.v.get("resolved").is_none(), "a describe by key has no resolved");
    for label in ["apple", "Apple", "APPLE"] {
        let a = ask(&m, &format!("/describe/{}", call_path(&format!("{n}[{label}]")))).await;
        assert_eq!((a.status, a.kind()), (200, Some("object")), "[{label}]: {}", a.text);
        for k in ["object", "object_id", "short_id", "call", "methods"] {
            assert_eq!(a.v[k], by_key.v[k], "[{label}] {k}");
        }
        let r = &a.v["resolved"];
        assert_eq!(r["call"], by_key.v["call"], "resolved.call is the object's call");
        let rs = r["resolutions"].as_array().unwrap_or_else(|| panic!("{}", a.text));
        assert_eq!(rs.len(), 1, "{}", a.text);
        assert_eq!((rs[0]["kind"].as_str(), rs[0]["to"].as_str()), (Some("lookup"), Some(pk("A1").as_str())), "{}", a.text);
        assert!(rs[0]["asof"].as_str().is_some_and(|t| t.len() == 27 && t.ends_with('Z')), "{}", a.text);
    }
    // a label that was withdrawn, and one never known: not found
    for label in ["old", "nope"] {
        let a = ask(&m, &format!("/describe/{}", call_path(&format!("{n}[{label}]")))).await;
        assert_eq!(a.status, 404, "[{label}]: {}", a.text);
    }
}

/// K8 on the wire as clients send it: the brackets RAW (legal in a URL path), an unquoted label, no method. The
/// test above percent-encodes them; this one does not. It was `400 E_CANON_NAME`. `/describe/` is the same.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k8_raw_brackets_unquoted_label_with_no_method_answers_describe() {
    let m = shop("cr-k8-raw", false).await;
    let n = product();
    let by_key = ask(&m, &format!("/describe/{}", call_path(&pk("A1")))).await;
    for verb in ["cell", "describe"] {
        for label in ["[apple]", "[APPLE]", "[%20Apple%20]"] {
            let p = format!("/{verb}/{n}{label}");
            let a = ask(&m, &p).await;
            assert_eq!((a.status, a.kind()), (200, Some("object")), "{p}: {}", a.text);
            assert_eq!(a.v["object_id"], by_key.v["object_id"], "{p}");
            assert_eq!(a.resolution_kinds(), ["lookup"], "{p}: {}", a.text);
            assert_eq!(a.resolutions()[0].2, pk("A1"), "{p}");
        }
    }
}

/// A request of `N[label]` with no method, at `/cell`, answers that object's `describe` with HTTP 200 and `kind` `"object"`,
/// `resolved` saying the lookup was made. It was `E_PARSE_URI`. An unknown label is `E_NAME_UNBOUND`; a class without lookup
/// has no labels.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k8_a_label_with_no_method_at_cell_answers_describe() {
    let m = shop("cr-k8-cell", false).await;
    let n = product();
    let by_key = ask(&m, &format!("/describe/{}", call_path(&pk("A1")))).await;
    for call in [format!("{n}[apple]"), format!("{n}[APPLE]"), format!("{n}[\"  Apple \"]")] {
        let a = ask(&m, &path(&call, None)).await;
        assert_eq!((a.status, a.kind()), (200, Some("object")), "{call}: {}", a.text);
        assert_eq!(a.v["object_id"], by_key.v["object_id"], "{call}");
        assert_eq!(a.v["call"], by_key.v["call"], "{call}");
        assert_eq!(a.resolution_kinds(), ["lookup"], "{call}: {}", a.text);
        assert_eq!(a.resolutions()[0].2, pk("A1"), "{call}");
    }
    let a = ask(&m, &path(&format!("{n}[nope]"), None)).await;
    assert_eq!((a.status, a.outcome(), a.error()), (404, "not_covered", Some("E_NAME_UNBOUND")), "{}", a.text);
    // the class `shop/crate` has no lookup: a label of it is E_NAME_UNBOUND, not a describe
    let c = ask(&m, &path(&format!("{}[x]", crate_class()), None)).await;
    assert_eq!((c.status, c.error()), (404, Some("E_NAME_UNBOUND")), "{}", c.text);
}
