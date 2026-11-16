//! CONTRACT-1.7a H2: no partial writes. Each case is a file whose last line fails; pass one must
//! find the failure, so the store (compared by a snapshot before and after) has no new binding,
//! descriptor, class, blob, receipt or version. SQLite store, mesh clock fixed at
//! 2026-11-15T12:00:00.000000Z.

mod common;

use common::lite::*;
use common::pub031::*;
use datumesh_core::Resolver;
use datumesh_store::Store;

const CLASS: &str = "mesh-test.datumesh.com/scratch_1_7a/pair";
const RATE: &str = "mesh-test.datumesh.com/scratch_1_7a/pair/rate";
const F: &str = "mesh-test.datumesh.com/scratch_1_7a/fx_f";
const G: &str = "mesh-test.datumesh.com/scratch_1_7a/fx_g";

/// A function (`Y`, hash returned) in the store that is not under the class's names.
fn put_method_function(lite: &Lite) -> String {
    let d = stored_descriptor(&rate_sig());
    let h = lite.store.put_descriptor(&descriptor_canonical(&d)).unwrap();
    assert_eq!(h, descriptor_hash(&d));
    lite.store.bind_name(&format!("{NS}/y_rate"), &h, K_FROM).unwrap();
    h
}

// ---------------------------------------------------------------------------------------------
// 1. Planned binds per name

/// W1b, exactly: in one run a function line binding `N/m` (no `bound`: now) and a class line
/// binding `N`, which binds `N/m` too (no `bound`: the same now), are a conflict. Refused in pass
/// one at the class line; in 0.3.0 line 1 was written and line 2 failed.
#[test]
fn w1b_a_function_line_and_a_class_line_binding_one_name_in_one_run() {
    let lite = Lite::new("h2-w1b");
    let y = put_method_function(&lite);
    let lines = [
        function_line(RATE, &rate_sig_plain(), None),
        class_line(CLASS, &pair_class(&[("rate", &y)]), None),
    ];
    refused_whole(&lite, "f.jsonl", &lines, 2, NOW);
    assert!(unbound(&lite, RATE), "line 1's binding was not written");
    assert!(lite.store.resolve_class(CLASS, END).unwrap().is_none());
}

/// The same with a `bound` on both lines at one instant.
#[test]
fn w1b_with_one_explicit_instant() {
    let lite = Lite::new("h2-w1b-explicit");
    let y = put_method_function(&lite);
    let lines = [
        function_line(RATE, &rate_sig_plain(), Some(T1)),
        class_line(CLASS, &pair_class(&[("rate", &y)]), Some(T1)),
    ];
    refused_whole(&lite, "f.jsonl", &lines, 2, NOW);
}

/// A class bind whose method bind is not later than the latest existing binding of `N/m` is
/// refused, whatever the file's other lines: the earlier lines are not written.
#[test]
fn a_class_method_bind_not_later_than_the_existing_binding_is_refused() {
    let lite = Lite::new("h2-existing");
    let y = put_method_function(&lite);
    let first = write(&lite, "first.jsonl", &[function_line(RATE, &rate_sig_plain(), None)]);
    run(&lite, &first, NOW).unwrap();
    assert_eq!(lite.bound_tx(RATE).as_deref(), Some(NOW));

    let lines = [
        function_line(&format!("{NS}/other"), &rate_sig(), None),
        class_line(CLASS, &pair_class(&[("rate", &y)]), None),
    ];
    refused_whole(&lite, "second.jsonl", &lines, 2, NOW);
    assert!(unbound(&lite, &format!("{NS}/other")));
}

/// A second bind of one name is accepted at a later instant: the same two lines, the clock later.
#[test]
fn a_second_bind_of_a_name_is_accepted_at_a_later_instant() {
    let lite = Lite::new("h2-later");
    let y = put_method_function(&lite);
    let first = write(&lite, "first.jsonl", &[function_line(RATE, &rate_sig_plain(), None)]);
    run(&lite, &first, NOW).unwrap();
    let f = write(&lite, "second.jsonl", &[class_line(CLASS, &pair_class(&[("rate", &y)]), None)]);
    let p = run(&lite, &f, T1).unwrap();
    assert_eq!(p.classes, 1, "{p:?}");
    assert_eq!(lite.bound_tx(RATE).as_deref(), Some(T1));
    assert_eq!(lite.store.resolve_binding(RATE, NOW).unwrap().unwrap().tx, NOW, "the first binding stands as of NOW");
}

/// Two function lines of one name at one instant with different hashes are a conflict too, and a
/// bind skipped because the name is already bound to that hash stays skipped (counted 0).
#[test]
fn two_binds_of_one_name_at_one_instant_conflict_and_a_skip_stays_a_skip() {
    let lite = Lite::new("h2-two");
    let lines = [function_line(F, &rate_sig(), None), function_line(F, &rate_sig_plain(), None)];
    refused_whole(&lite, "f.jsonl", &lines, 2, NOW);
    assert!(unbound(&lite, F));

    let a = write(&lite, "a.jsonl", &[function_line(F, &rate_sig(), None)]);
    assert_eq!(run(&lite, &a, NOW).unwrap().functions, 1);
    let b = write(&lite, "b.jsonl", &[function_line(F, &rate_sig(), None), function_line(G, &rate_sig(), None)]);
    let p = run(&lite, &b, NOW).unwrap();
    assert_eq!(p.functions, 1, "F is bound to that hash already: skipped; G binds: {p:?}");
}

// ---------------------------------------------------------------------------------------------
// 2. Cells

/// The valid first part of every cell case: a function and a cell of it (a blob cell with its
/// file), then the failing last line.
fn good_prefix(lite: &Lite) -> Vec<String> {
    std::fs::write(lite.path("good.txt"), b"good blob").unwrap();
    vec![
        function_line(F, &rate_sig(), None),
        blob_cell(F, "good.txt", NOW),
        function_line(G, &rate_sig(), None),
        rate_cell(G, "1.0832", Some(NOW), Some(NOW), None),
    ]
}

fn with_last(lite: &Lite, last: String) -> Vec<String> {
    let mut lines = good_prefix(lite);
    lines.push(last);
    lines
}

/// A cell whose function is not bound at all.
#[test]
fn a_cell_of_an_unbound_function_is_refused_in_pass_one() {
    let lite = Lite::new("h2-c-unbound");
    let last = rate_cell(&format!("{NS}/nowhere"), "1", Some(NOW), Some(NOW), None);
    let lines = with_last(&lite, last);
    refused_whole(&lite, "f.jsonl", &lines, 5, NOW);
    assert!(unbound(&lite, F) && unbound(&lite, G));
}

/// The function is resolved as of `tx`, counting binds planned earlier in the file: the function
/// binds at the clock's now, and a cell with `tx` one microsecond before it is not bound as of tx.
#[test]
fn a_cell_before_its_functions_binding_is_refused_in_pass_one() {
    let lite = Lite::new("h2-c-before");
    let last = rate_cell(G, "1.09", Some(BEFORE), Some(BEFORE), None);
    let lines = with_last(&lite, last);
    refused_whole(&lite, "f.jsonl", &lines, 5, NOW);
}

/// A blob file that does not exist.
#[test]
fn a_missing_blob_file_is_refused_in_pass_one() {
    let lite = Lite::new("h2-c-noblob");
    let lines = with_last(&lite, blob_cell(F, "absent.txt", NOW));
    refused_whole(&lite, "f.jsonl", &lines, 5, NOW);
}

/// A blob path that exists but cannot be read (a directory).
#[test]
fn an_unreadable_blob_file_is_refused_in_pass_one() {
    let lite = Lite::new("h2-c-badblob");
    std::fs::create_dir_all(lite.path("a_directory")).unwrap();
    let lines = with_last(&lite, blob_cell(F, "a_directory", NOW));
    refused_whole(&lite, "f.jsonl", &lines, 5, NOW);
}

/// `tx` after `asof` (W2).
#[test]
fn tx_after_asof_is_refused_in_pass_one() {
    let lite = Lite::new("h2-c-tx-asof");
    let last = rate_cell(G, "1.09", Some(T2), Some(T1), None);
    let lines = with_last(&lite, last);
    refused_whole(&lite, "f.jsonl", &lines, 5, NOW);
}

/// `asof` after `issued` (W2), with `issued` given.
#[test]
fn asof_after_issued_is_refused_in_pass_one() {
    let lite = Lite::new("h2-c-asof-issued");
    let last = rate_cell(G, "1.09", Some(NOW), Some(T1), Some(NOW));
    let lines = with_last(&lite, last);
    refused_whole(&lite, "f.jsonl", &lines, 5, NOW);
}

/// A version of the cell at the same `tx` that is not this receipt (another value): refused, and
/// the lines before it are not written.
#[test]
fn a_different_version_at_the_same_tx_is_refused_in_pass_one() {
    let lite = Lite::new("h2-c-version");
    let first = write(
        &lite,
        "first.jsonl",
        &[function_line(F, &rate_sig(), None), rate_cell(F, "1.0832", Some(NOW), Some(NOW), None)],
    );
    run(&lite, &first, NOW).unwrap();
    let lines = [
        function_line(G, &rate_sig(), None),
        rate_cell(G, "1.0832", Some(NOW), Some(NOW), None),
        rate_cell(F, "9.9999", Some(NOW), Some(NOW), None),
    ];
    refused_whole(&lite, "second.jsonl", &lines, 3, NOW);
    assert!(unbound(&lite, G));
}

/// A version at a later `tx` than the new cell's: also refused ("at or after tx").
#[test]
fn a_version_after_tx_is_refused_in_pass_one() {
    let lite = Lite::new("h2-c-later");
    let first = write(
        &lite,
        "first.jsonl",
        &[function_line(F, &rate_sig(), None), rate_cell(F, "1.0832", Some(T2), Some(T2), None)],
    );
    run(&lite, &first, NOW).unwrap();
    let lines = [
        function_line(G, &rate_sig(), None),
        rate_cell(G, "1.0832", Some(NOW), Some(NOW), None),
        rate_cell(F, "1.09", Some(T1), Some(T1), None),
    ];
    refused_whole(&lite, "second.jsonl", &lines, 3, NOW);
}

/// The same receipt again is not a conflict: a re-run of the file that published it changes
/// nothing and is not refused.
#[test]
fn the_same_receipt_again_is_accepted() {
    let lite = Lite::new("h2-c-same");
    let f = write(
        &lite,
        "f.jsonl",
        &[function_line(F, &rate_sig(), None), rate_cell(F, "1.0832", Some(NOW), Some(NOW), None)],
    );
    let p = run(&lite, &f, NOW).unwrap();
    assert_eq!((p.functions, p.versions), (1, 1), "{p:?}");
    let before = snapshot(&lite);
    let again = run(&lite, &f, NOW).unwrap();
    assert_eq!((again.functions, again.versions), (0, 0), "{again:?}");
    assert_eq!(snapshot(&lite), before);
}

// ---------------------------------------------------------------------------------------------
// 3. Class define

/// A class whose method names a hash that is neither in the store nor put earlier in the file.
#[test]
fn a_class_naming_an_unknown_method_hash_is_refused_in_pass_one() {
    let lite = Lite::new("h2-d-unknown");
    let y = put_method_function(&lite);
    let missing = "0".repeat(64);
    let d = stored_descriptor(&rate_sig());
    let h = descriptor_hash(&d);
    let lines = [
        descriptor_line(&d),
        class_line(CLASS, &pair_class(&[("rate", &h)]), None),
        class_line(&format!("{NS}/pair2"), &pair_class(&[("rate", &missing)]), None),
    ];
    refused_whole(&lite, "f.jsonl", &lines, 3, NOW);
    assert!(lite.store.descriptor(&h).is_none(), "the descriptor line before it is not put");
    let _ = y;
}

/// A class defined against a descriptor put earlier in the file is accepted (define sees it), and
/// one under a namespace the mesh does not own is refused with the owner error, nothing written.
#[test]
fn a_class_under_a_foreign_host_is_refused_and_nothing_is_written() {
    let lite = Lite::new("h2-d-owner");
    let d = stored_descriptor(&rate_sig());
    let h = descriptor_hash(&d);
    let lines = [
        descriptor_line(&d),
        class_line(CLASS, &pair_class(&[("rate", &h)]), None),
        class_line("other.example/scratch_1_7a/pair", &pair_class(&[("rate", &h)]), None),
    ];
    let e = refused_whole(&lite, "f.jsonl", &lines, 3, NOW);
    assert!(e.detail.contains("E_CLASS_OWNER"), "{e}");
}
