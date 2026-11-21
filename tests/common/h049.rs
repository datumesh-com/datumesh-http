//! Helpers of the 0.4.9 tests (CONTRACT-0.4.9 H1, H6): a stored function `level(site)`, its tests written
//! straight into the store (so that a test can set up thousands of rows, tamper with a stored result, or hold
//! a row that never ran), and reads of the function page.
//!
//! Needs `#[path = "common/calculus.rs"] mod calculus; mod common; #[path = "common/publish4.rs"] mod publish4;`
//! at the crate root, like the tests that use `calculus`.
#![allow(dead_code)]

use datumesh_store::{FunctionTest, Store};
use serde_json::Value;

use crate::calculus::{case_id, num, reading_sig, site_cell, test_line};
use crate::common::lite::*;
use crate::common::readpath::*;

pub const OLD_RUN: &str = "2026-01-01T00:00:00.000000Z";

pub fn sig() -> String {
    reading_sig()
}

pub fn call(f: &str, site: &str) -> String {
    format!("{f}(site: \"{site}\")")
}

/// A test line: `f(site: "<site>")` is the number `want`.
pub fn tline(f: &str, site: &str, want: &str) -> String {
    test_line(f, &call(f, site), None, None, num(want))
}

pub fn cell(f: &str, site: &str, value: &str) -> String {
    site_cell(f, site, value)
}

/// The hash `name` is bound to in `lite`'s store.
pub fn hash_in(lite: &Lite, name: &str) -> String {
    lite.store.resolve_binding(name, END).unwrap().unwrap_or_else(|| panic!("{name} is unbound")).fn_hash
}

pub fn jcs(line: &str) -> Vec<u8> {
    datumesh_core::json::parse(line.as_bytes()).unwrap().to_jcs()
}

/// Writes the test `line` of the function with hash `hash` straight into the store, as one that last ran at
/// `run` = (instant, result) or never; returns its case. The row is not named.
pub fn put_raw(lite: &Lite, hash: &str, line: &str, run: Option<(&str, &str)>) -> String {
    let case = case_id(line);
    let mut t = FunctionTest::new(hash, &case, &jcs(line), HOST);
    if let Some((at, result)) = run {
        t.last_run_at = Some(at.to_string());
        t.last_result = Some(result.to_string());
        t.last_detail = Some(String::new());
    }
    lite.store.put_function_test(&t).unwrap();
    case
}

/// `GET path`, 200, JSON.
pub async fn page_json(m: &Served, path: &str) -> Value {
    let r = crate::publish4::get(m, path, &[("accept", "application/json")]).await;
    assert_eq!(r.status, 200, "{path}: {}", r.text());
    r.json()
}

pub fn tests_of(page: &Value) -> &Value {
    &page["tests"]
}
