//! CONTRACT-1.7a H4, found by the production rehearsal: a file whose cell lines leave out `tx`
//! re-run at a later clock must change nothing ("re-running a file changes nothing", the module's
//! own rule). Without this, every re-run signs a second version of every cell at the new clock (the
//! rehearsal's re-run of the Tanakh file wrote 24,185 versions again). A cell line without `tx`
//! whose value, licence and summary equal its latest version's is skipped; a changed value is
//! still a new version (a correction).

mod common;

use common::lite::*;
use common::pub031::*;

const F: &str = "mesh-test.datumesh.com/scratch_1_7a/fx_later";

#[test]
fn a_rerun_at_a_later_clock_changes_nothing() {
    let lite = Lite::new("h4-later");
    let f = write(&lite, "f.jsonl", &[function_line(F, &rate_sig(), None), rate_cell(F, "1.0832", None, None, None)]);
    run(&lite, &f, NOW).unwrap();
    let before = snapshot(&lite);
    let again = run(&lite, &f, T2).unwrap();
    assert_eq!((again.functions, again.versions), (0, 0), "{again:?}");
    assert_eq!(snapshot(&lite), before, "a re-run at a later clock wrote something");
}

#[test]
fn a_changed_value_at_a_later_clock_is_a_new_version() {
    let lite = Lite::new("h4-later-changed");
    let f = write(&lite, "f.jsonl", &[function_line(F, &rate_sig(), None), rate_cell(F, "1.0832", None, None, None)]);
    run(&lite, &f, NOW).unwrap();
    let g = write(&lite, "g.jsonl", &[rate_cell(F, "1.0833", None, None, None)]);
    let p = run(&lite, &g, T2).unwrap();
    assert_eq!(p.versions, 1, "{p:?}");
}
