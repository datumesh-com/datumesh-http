//! CONTRACT-1.7a H1: a `{"descriptor": ...}` line in `publish-stored` puts a stored function's
//! descriptor and binds nothing; a class line (later in the file or in a later run) may name its
//! hash as a method. SQLite store, the mesh clock fixed at 2026-11-15T12:00:00.000000Z (rule A, W1).

mod common;

use common::lite::*;
use common::pub031::*;
use datumesh_core::Resolver;
use datumesh_store::Store;

const CLASS: &str = "mesh-test.datumesh.com/scratch_1_7a/verse2";
const RATE: &str = "mesh-test.datumesh.com/scratch_1_7a/verse2/rate";

/// A descriptor line followed by a class line naming its hash: one descriptor counted, one class,
/// the descriptor stored under its canonical hash, and `<class>/rate` has exactly one binding,
/// whose start is the class's (the clock's now, as the class line has no `bound`).
#[test]
fn a_descriptor_line_then_a_class_line_binds_the_method_once() {
    let lite = Lite::new("h1-a");
    let d = stored_descriptor(&rate_sig());
    let h = descriptor_hash(&d);
    let f = write(&lite, "f.jsonl", &[descriptor_line(&d), class_line(CLASS, &pair_class(&[("rate", &h)]), None)]);
    let p = run(&lite, &f, NOW).unwrap();
    assert_eq!(descriptors_of(&p), 1, "{p:?}");
    assert_eq!((p.functions, p.classes), (0, 1), "{p:?}");

    // The store returns the descriptor's canonical bytes under the core's hash.
    let stored = lite.store.descriptor(&h).expect("the descriptor is stored under its core hash");
    assert_eq!(stored.as_ref(), descriptor_canonical(&d).as_slice());

    // One binding of the method, from the class's start: nothing earlier, the class's tx at NOW.
    let b = lite.store.resolve_binding(RATE, END).unwrap().expect("the method is bound");
    assert_eq!((b.fn_hash.as_str(), b.tx.as_str()), (h.as_str(), NOW));
    assert!(lite.store.resolve_binding(RATE, BEFORE).unwrap().is_none());
    let c = lite.store.resolve_class_binding(CLASS, END).unwrap().expect("the class is bound");
    assert_eq!(c.tx, NOW);
}

/// The line itself binds nothing: a file of descriptor lines alone leaves every name unbound and
/// counts the descriptors; the second run counts 0 (the store's `put` is idempotent) and changes
/// nothing.
#[test]
fn a_descriptor_line_binds_nothing_and_a_rerun_counts_zero() {
    let lite = Lite::new("h1-b");
    let d1 = stored_descriptor(&rate_sig());
    let d2 = stored_descriptor(&convert_sig());
    let f = write(&lite, "f.jsonl", &[descriptor_line(&d1), descriptor_line(&d2)]);
    let p = run(&lite, &f, NOW).unwrap();
    assert_eq!(descriptors_of(&p), 2, "{p:?}");
    assert_eq!((p.functions, p.classes, p.cells), (0, 0, 0), "{p:?}");
    assert!(lite.store.descriptor(&descriptor_hash(&d1)).is_some());
    assert!(lite.store.descriptor(&descriptor_hash(&d2)).is_some());
    assert!(!lite.store.has_bindings_under(NS).unwrap(), "nothing is bound");

    let before = snapshot(&lite);
    let again = run(&lite, &f, NOW).unwrap();
    assert_eq!(descriptors_of(&again), 0, "{again:?}");
    assert_eq!(snapshot(&lite), before);
}

/// The same descriptor twice in one file counts once (newly put), and one already in the store
/// (put by a function line) counts 0.
#[test]
fn only_a_descriptor_newly_put_is_counted() {
    let lite = Lite::new("h1-c");
    let d = stored_descriptor(&rate_sig());
    let f = write(&lite, "f.jsonl", &[descriptor_line(&d), descriptor_line(&d)]);
    let p = run(&lite, &f, NOW).unwrap();
    assert_eq!(descriptors_of(&p), 1, "{p:?}");

    let lite = Lite::new("h1-c2");
    let name = format!("{NS}/plain");
    let g = write(&lite, "g.jsonl", &[function_line(&name, &rate_sig(), None), descriptor_line(&d)]);
    let p = run(&lite, &g, NOW).unwrap();
    assert_eq!(p.functions, 1, "{p:?}");
    assert_eq!(descriptors_of(&p), 0, "the function line put it already: {p:?}");
}

/// A class line of a later run finds the descriptor in the store (the `verse2` case of the test
/// mesh), at the later run's clock.
#[test]
fn a_class_in_a_later_run_names_a_stored_descriptor() {
    let lite = Lite::new("h1-d");
    let d = stored_descriptor(&rate_sig());
    let h = descriptor_hash(&d);
    let f1 = write(&lite, "d.jsonl", &[descriptor_line(&d)]);
    assert_eq!(descriptors_of(&run(&lite, &f1, NOW).unwrap()), 1);
    let f2 = write(&lite, "c.jsonl", &[class_line(CLASS, &pair_class(&[("rate", &h)]), None)]);
    let p = run(&lite, &f2, T1).unwrap();
    assert_eq!((p.classes, descriptors_of(&p)), (1, 0), "{p:?}");
    let b = lite.store.resolve_binding(RATE, END).unwrap().unwrap();
    assert_eq!((b.fn_hash.as_str(), b.tx.as_str()), (h.as_str(), T1));
}

/// Cells under `<class>/rate`, with `tx` at or after the method's start, are published and verify.
#[test]
fn cells_under_the_class_method_verify() {
    let lite = Lite::new("h1-e");
    let d = stored_descriptor(&rate_sig());
    let h = descriptor_hash(&d);
    let f1 = write(&lite, "c.jsonl", &[descriptor_line(&d), class_line(CLASS, &pair_class(&[("rate", &h)]), None)]);
    run(&lite, &f1, NOW).unwrap();

    let f2 = write(
        &lite,
        "cells.jsonl",
        &[
            rate_cell(RATE, "1.0832", Some(NOW), Some(NOW), Some(NOW)),
            rate_cell(RATE, "1.09", Some(T1), Some(T1), None),
        ],
    );
    let p = run(&lite, &f2, T1).unwrap();
    assert_eq!((p.cells, p.versions), (2, 2), "{p:?}");
    let id = rate_cell_id(RATE, &h);
    let ks = keys(&lite);
    for (asof, value) in [(NOW, "1.0832"), (T1, "1.09")] {
        let v = lite.store.version_asof(&id, asof).unwrap().expect("a version");
        let receipt = lite.store.receipt(&v.receipt_hash).expect("the receipt").into_owned();
        let r = datumesh_core::verify(&receipt, &ks, None, None);
        assert!(r.ok, "{r:?}");
        let text = String::from_utf8(receipt).unwrap();
        assert!(text.contains(value), "{text}");
    }
}

/// A formula descriptor in a descriptor line is refused at that line, in pass one: a stored
/// descriptor and a function line before it are not written.
#[test]
fn a_formula_descriptor_is_refused_and_nothing_is_written() {
    let lite = Lite::new("h1-f");
    let good = stored_descriptor(&rate_sig());
    let formula = format!(r#"{{"code":{{"body":"1","lang":"formula"}},"inputs":[],"signature":{}}}"#, rate_sig());
    let lines = [
        descriptor_line(&good),
        function_line(&format!("{NS}/plain"), &rate_sig(), None),
        descriptor_line(&formula),
    ];
    refused_whole(&lite, "f.jsonl", &lines, 3, NOW);
    assert!(lite.store.descriptor(&descriptor_hash(&good)).is_none());
    assert!(unbound(&lite, &format!("{NS}/plain")));
}

/// Only `code.lang` = `stored`, `code.body` = `""` and `inputs` = `[]` are accepted: a stored
/// descriptor with a body is refused at its line too.
#[test]
fn a_stored_descriptor_with_a_body_is_refused() {
    let lite = Lite::new("h1-g");
    let good = stored_descriptor(&rate_sig());
    let with_body = format!(r#"{{"code":{{"body":"x","lang":"stored"}},"inputs":[],"signature":{}}}"#, rate_sig());
    let lines = [descriptor_line(&good), descriptor_line(&with_body)];
    refused_whole(&lite, "f.jsonl", &lines, 2, NOW);
    assert!(lite.store.descriptor(&descriptor_hash(&good)).is_none());
}
