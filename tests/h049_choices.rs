//! Choices of the 0.4.9 tests (CONTRACT-0.4.9), where the contract leaves a detail open. The implementer
//! follows them or says why not.
//!
//! Choices:
//! * Test knobs for "before the first build" (each holds the first background build back for that many
//!   seconds, default 0, like `DATUMESH_SERIES_FIRST_BUILD_DELAY_SECS`):
//!   - H2 takeover page: `DATUMESH_SERIES_FIRST_BUILD_DELAY_SECS` (the series thread builds it);
//!   - H3 object-id describe: `DATUMESH_INDEX_FIRST_BUILD_DELAY_SECS`;
//!   - H4 `/find?summary=1` and the empty query: `DATUMESH_FIND_FIRST_BUILD_DELAY_SECS`.
//!   A cold answer is `503`, `Retry-After` in 1..=300 whole seconds, `Cache-Control: no-store`.
//! * H1 JSON: `tests.last_run` is exactly `{result, at, pass, fail, error, not_run}`; `result` is `error`,
//!   `fail`, `not run` or `pass`; `at` is null (or absent) when nothing ever ran; `describe`'s `tests` keeps
//!   exactly `{count, branches, covered, counterexamples}`. `counterexamples` = stored fail + error rows of the
//!   function's own name. The HTML says "Showing the first 100 of N tests" (as 0.4.8) and "last run <at>".
//!   Stored branch counts (`set_function_branches`) are what `branches`/`covered` show.
//! * H1 tests write rows straight into the store (`put_function_test`, `record_function_test`,
//!   `set_function_branches`, and the new `name_function_tests`) to tamper with stored results.
//! * H5: `/slice` past the cap is cut to the first `slice_max` members, no `next` past them (the 0.4.8
//!   behaviour, pinned); `/x/interval` of a year series: exactly 50 years is allowed (`to` <= `from` + 50
//!   years, as 10 for months), more is `400` `E_RANGE_TOO_LARGE`, for JSON, CSV and Arrow alike.
//! * H6: "the old tests are not rerun" is read as: the rows of tests the file did not write keep their
//!   `last_run_at` (the file writes new cells of the same function; their tests run, old ones do not). Bound:
//!   under 20 s on SQLite in a debug build, with 2,000 old tests.
//! * H7: the limit is the process environment variable `DATUMESH_DICTIONARY_MAX` (set to 5 in the test
//!   binary before anything runs); the library `publish_stored_with` reads it when it needs the limit, and
//!   `Config::from_vars` reads it too. "Past the limit" means the function would have more than MAX distinct
//!   values for the key; exactly MAX is allowed. A catalogue line's `"open_keys": ["site"]` is a new member.
//!   `datumesh_dictionary_open_keys_total` is the number of open keys the store holds (a gauge in spirit,
//!   named `_total` by the contract), 2 in the test. Store reads used by the tests: `key_value_count(owner,
//!   key)`, `has_key_value`, `key_is_open` (S7). `find` filling an open key only from an exact cell value
//!   (H7.6) is NOT tested here.
//! * H8: the error is `E_NAME_RESERVED`, found in `PublishError::detail`, in the Display of `BundleError` and
//!   of the declare error, and as `{"error": "E_NAME_RESERVED", "detail"}` with status 422 from
//!   `POST /bundles`. `<host>/key/k<24 hex>/f` of the mesh's own key, and any `<host>/key/foo`, are reserved
//!   even from a registered key. `import-vectors` and bind lines are not tested separately (they share the
//!   name check). A bare `<host>/series` (no second segment) is not tested either way.
//! * H2's "relay_of reads at most 8 receipts" and H4's "reindex once per DATUMESH_VALUE_COUNTS_SECS" are not
//!   observable from outside and have no test.
//! * Existing tests that expect `tests.derived` (tests/function_page.rs `the_json_form_is_the_same_data`, and
//!   any that look for derived cases on the page) contradict H1 and must be changed by the implementer.

#[test]
fn choices_are_written_above() {}
