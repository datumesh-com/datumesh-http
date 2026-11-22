//! The classes round (CONTRACT-classes-round.md), K11: unquoted runs of digits and hyphens are strings in string positions
//! (SPEC 5.8). Failing tests: today `0000320193` is `E_CANON_NUMBER` and `0000320193-24-000123` too, in a string position.
//!
//! In a request, in a `string` position (a string key included), an unquoted token that is a run of ASCII digits and `-`,
//! beginning with a digit, that no number, date or period lexer accepts as a whole, is that string. Anywhere else the run
//! keeps today's code. The world is `common::crworld::filing`: the class `sec/company` (key `cik`) with the method
//! `filed(accession)`, and the function `sec/reading(cik, accession, n)` with a number position.

mod common;
#[path = "common/crworld.rs"]
mod crworld;

use crworld::*;

/// Both forms unquoted, in key and argument positions, in a method call, labelled or not, and in a plain read: one cell, the
/// one the quoted spelling names, with the same receipt; `resolved.call` prints the strings quoted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k11_digit_runs_are_strings_in_string_positions() {
    let m = filing("cr-k11-strings").await;
    let (c, r) = (company(), reading());
    let quoted = ask(&m, &path(&format!("{c}(\"{CIK}\").filed(\"{ACCN}\")"), None)).await;
    assert_eq!((quoted.outcome(), quoted.value()), ("ok", Some("2024-11-01")), "{}", quoted.text);
    for call in [
        format!("{c}({CIK}).filed({ACCN})"),
        format!("{c}({CIK}).filed(\"{ACCN}\")"),
        format!("{c}(\"{CIK}\").filed({ACCN})"),
        format!("{c}(cik: {CIK}).filed(accession: {ACCN})"),
        format!("{c}(cik:{CIK}).filed(accession:{ACCN})"),
    ] {
        let a = ask(&m, &path(&call, None)).await;
        assert_eq!((a.status, a.outcome(), a.value()), (200, "ok", Some("2024-11-01")), "{call}: {}", a.text);
        assert_eq!(a.cell_id(), quoted.cell_id(), "{call}: the cell of the quoted spelling");
        assert_eq!(
            a.v["resolved"]["call"],
            format!(r#"{c}/filed(accession: "{ACCN}", cik: "{CIK}")"#),
            "{call}: strings in the canonical call"
        );
    }
    // the key alone: an object with the CIK unquoted, with the quoted accession, and a CIK alone (the digits of a CIK only)
    let a = ask(&m, &path(&format!("{c}({CIK}).filed(\"{ACCN}\")"), None)).await;
    assert_eq!(a.cell_id(), quoted.cell_id(), "{}", a.text);
    // a plain read: string positions unquoted, the number position a number
    let want = ask(&m, &path(&format!("{r}(\"{CIK}\", \"{ACCN}\", 5)"), None)).await;
    assert_eq!((want.outcome(), want.value()), ("ok", Some("42")), "{}", want.text);
    for call in [
        format!("{r}({CIK}, {ACCN}, 5)"),
        format!("{r}(cik: {CIK}, accession: {ACCN}, n: 5)"),
        format!("{r}({CIK}, \"{ACCN}\", 5)"),
    ] {
        let a = ask(&m, &path(&call, None)).await;
        assert_eq!((a.status, a.outcome(), a.value()), (200, "ok", Some("42")), "{call}: {}", a.text);
        assert_eq!(a.cell_id(), want.cell_id(), "{call}");
    }
    // a digit run with only digits and one hyphen is a string too (no lexer accepts `0000320193-24` whole)
    let b = ask(&m, &path(&format!("{r}({CIK}, 0000320193-24, 5)"), None)).await;
    assert_eq!((b.status, b.outcome()), (404, "not_covered"), "a string, no cell has it: {}", b.text);
}

/// The same runs in a number position give today's codes (the digit run is no number there), as does a run in the other
/// kinds of position and a run with a letter in it (the digit-led-word rule is unchanged).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn k11_the_same_runs_elsewhere_keep_their_codes() {
    let m = filing("cr-k11-codes").await;
    let (c, r) = (company(), reading());
    let strs = format!("\"{CIK}\", \"{ACCN}\"");
    for (call, code) in [
        (format!("{r}({strs}, 0000320193)"), "E_CANON_NUMBER"),
        (format!("{r}({strs}, 0000320193-24-000123)"), "E_CANON_NUMBER"),
        (format!("{r}({strs}, 0001)"), "E_CANON_NUMBER"),
        (format!("{r}({strs}, 00)"), "E_CANON_NUMBER"),
        (format!("{r}({strs}, 1-1)"), "E_PARSE_URI"),
        (format!("{r}({strs}, 12-34)"), "E_PARSE_URI"),
        (format!("{r}({strs}, 5-)"), "E_PARSE_URI"),
        (format!("{r}({strs}, 2024-11-01)"), "E_CANON_TYPE"),
        // a run with a letter keeps the digit-led-word rule: in a string position it is `E_CANON_NUMBER` still
        (format!("{c}(\"{CIK}\").filed(0000320193-24-000123x)"), "E_CANON_NUMBER"),
    ] {
        let a = ask(&m, &path(&call, None)).await;
        assert_eq!((a.status, a.outcome(), a.error()), (400, "bad_request", Some(code)), "{call}: {}", a.text);
    }
    // the control: what a string position took before, it takes now
    for call in [format!("{c}(\"{CIK}\").filed(2024-11-01)"), format!("{c}(\"{CIK}\").filed(12D)")] {
        let a = ask(&m, &path(&call, None)).await;
        assert_eq!((a.status, a.outcome()), (404, "not_covered"), "{call}: {}", a.text);
    }
}
