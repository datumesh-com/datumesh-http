//! CONTRACT-1.7a H4: a cell line of `publish-stored` may leave out `tx` and `asof`; both then
//! default to the run's clock now, the same instant as the default binding start, so one file can
//! bind and publish with one clock reading (W1, W2). SQLite store, clock fixed at
//! 2026-11-15T12:00:00.000000Z.

mod common;

use common::lite::*;
use common::pub031::*;
use datumesh_core::json::{self, Value};
use datumesh_core::Resolver;
use datumesh_store::Store;

const F: &str = "mesh-test.datumesh.com/scratch_1_7a/fx_f";

fn instants(lite: &Lite, id: &str, asof: &str) -> (String, String, String) {
    let v = lite.store.version_asof(id, asof).unwrap().expect("a version");
    let receipt = lite.store.receipt(&v.receipt_hash).expect("the receipt").into_owned();
    let r = datumesh_core::verify(&receipt, &keys(lite), None, None);
    assert!(r.ok, "{r:?}");
    let doc = json::parse(&receipt).unwrap();
    let get = |k: &str| -> String {
        let find = |v: &Value| v.get(k).and_then(Value::as_str).map(str::to_string);
        find(&doc)
            .or_else(|| doc.get("payload").and_then(find))
            .unwrap_or_else(|| panic!("no {k} in {}", String::from_utf8_lossy(&receipt)))
    };
    assert_eq!(v.tx, get("tx"));
    (get("tx"), get("asof"), get("issued"))
}

/// A function with no `bound` and a cell with no `tx`, `asof` or `issued`, in one file: the
/// function binds at the clock's now, and the receipt has `tx = asof = issued =` that now and
/// verifies.
#[test]
fn a_cell_without_tx_and_asof_takes_the_clock_now() {
    let lite = Lite::new("h4-a");
    let f = write(&lite, "f.jsonl", &[function_line(F, &rate_sig(), None), rate_cell(F, "1.0832", None, None, None)]);
    let p = run(&lite, &f, NOW).unwrap();
    assert_eq!((p.functions, p.cells, p.receipts, p.versions), (1, 1, 1, 1), "{p:?}");
    assert_eq!(lite.bound_tx(F).as_deref(), Some(NOW));

    let h = lite.store.resolve_name(F, END).unwrap().unwrap();
    let id = rate_cell_id(F, &h);
    let (tx, asof, issued) = instants(&lite, &id, NOW);
    assert_eq!((tx.as_str(), asof.as_str(), issued.as_str()), (NOW, NOW, NOW));
    // And a later clock gives later instants: the default is the run's clock, not a constant.
    let g = write(&lite, "g.jsonl", &[rate_cell(F, "1.09", None, None, None)]);
    let p = run(&lite, &g, T1).unwrap();
    assert_eq!((p.cells, p.versions), (1, 1), "{p:?}");
    let (tx, asof, issued) = instants(&lite, &id, T1);
    assert_eq!((tx.as_str(), asof.as_str(), issued.as_str()), (T1, T1, T1));
}

/// Each of the two may be left out alone: the one left out is the clock's now.
#[test]
fn tx_or_asof_alone_may_be_left_out() {
    let lite = Lite::new("h4-b");
    let f =
        write(&lite, "f.jsonl", &[function_line(F, &rate_sig(), None), rate_cell(F, "1.0832", Some(NOW), None, None)]);
    let p = run(&lite, &f, NOW).unwrap();
    assert_eq!(p.versions, 1, "{p:?}");
    let h = lite.store.resolve_name(F, END).unwrap().unwrap();
    let id = rate_cell_id(F, &h);
    assert_eq!(instants(&lite, &id, NOW), (NOW.to_string(), NOW.to_string(), NOW.to_string()));

    let lite = Lite::new("h4-c");
    let f =
        write(&lite, "f.jsonl", &[function_line(F, &rate_sig(), None), rate_cell(F, "1.0832", None, Some(NOW), None)]);
    let p = run(&lite, &f, NOW).unwrap();
    assert_eq!(p.versions, 1, "{p:?}");
    let h = lite.store.resolve_name(F, END).unwrap().unwrap();
    let id = rate_cell_id(F, &h);
    assert_eq!(instants(&lite, &id, NOW), (NOW.to_string(), NOW.to_string(), NOW.to_string()));
}

/// `issued` defaults as today: a given `issued` (at or after asof) is kept while `tx` and `asof`
/// default to now.
#[test]
fn issued_still_defaults_as_before() {
    let lite = Lite::new("h4-d");
    let f =
        write(&lite, "f.jsonl", &[function_line(F, &rate_sig(), None), rate_cell(F, "1.0832", None, None, Some(T1))]);
    run(&lite, &f, NOW).unwrap();
    let h = lite.store.resolve_name(F, END).unwrap().unwrap();
    let id = rate_cell_id(F, &h);
    assert_eq!(instants(&lite, &id, NOW), (NOW.to_string(), NOW.to_string(), T1.to_string()));
}

/// Re-running the same file at the same clock changes nothing (the same receipt).
#[test]
fn a_rerun_at_the_same_clock_changes_nothing() {
    let lite = Lite::new("h4-e");
    let f = write(&lite, "f.jsonl", &[function_line(F, &rate_sig(), None), rate_cell(F, "1.0832", None, None, None)]);
    run(&lite, &f, NOW).unwrap();
    let before = snapshot(&lite);
    let again = run(&lite, &f, NOW).unwrap();
    assert_eq!((again.functions, again.versions), (0, 0), "{again:?}");
    assert_eq!(snapshot(&lite), before);
}
