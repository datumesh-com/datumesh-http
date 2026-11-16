//! Helpers of the 0.3.1 tests (CONTRACT-1.7a H1-H4): SQLite stores on a fixed clock, the lines of
//! `publish-stored` written as JSON, and a snapshot of a store's file to prove that a refused run
//! wrote nothing.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use datumesh_core::json::{self, Value};
use datumesh_core::{FunctionDescriptor, Keys};
use datumesh_http::publish::{publish_stored_with, PublishError, PublishOptions, Published};
use datumesh_store::Store;

use super::lite::{FixedClock, Lite, HOST, K_FROM, NOW};
use super::world::{class_json, fx_rate_sig, NONE_KEYS_PAIR};

/// One second after [`NOW`].
pub const T1: &str = "2026-11-15T12:00:01.000000Z";
/// Two seconds after [`NOW`].
pub const T2: &str = "2026-11-15T12:00:02.000000Z";
/// One microsecond before [`NOW`].
pub const BEFORE: &str = "2026-11-15T11:59:59.999999Z";
/// Under the mesh's host, lowercase snake_case segments (W3).
pub const NS: &str = "mesh-test.datumesh.com/scratch_1_7a";

pub fn st(v: &str) -> Value {
    Value::String(v.to_string())
}

pub fn parse(v: &str) -> Value {
    json::parse(v.as_bytes()).unwrap()
}

pub fn jcs(members: Vec<(&str, Value)>) -> String {
    String::from_utf8(Value::Object(members.into_iter().map(|(k, v)| (k.to_string(), v)).collect()).to_jcs()).unwrap()
}

/// A stored function's descriptor, as `{"descriptor": ...}` carries it.
pub fn stored_descriptor(signature: &str) -> String {
    format!(r#"{{"code":{{"body":"","lang":"stored"}},"inputs":[],"signature":{signature}}}"#)
}

/// The hash of that descriptor: SHA-256 of its canonical bytes, the core's own.
pub fn descriptor_hash(descriptor: &str) -> String {
    FunctionDescriptor::from_json(descriptor.as_bytes()).unwrap().hash()
}

/// The canonical bytes of that descriptor.
pub fn descriptor_canonical(descriptor: &str) -> Vec<u8> {
    FunctionDescriptor::from_json(descriptor.as_bytes()).unwrap().canonical()
}

/// `fx_pair/rate` of the other tests.
pub fn rate_sig() -> String {
    fx_rate_sig()
}

/// Another function for the same class slot: the same arguments, a different return, so a
/// different hash.
pub fn rate_sig_plain() -> String {
    r#"{"args":{"base":{"type":"string","valid":false},"date":{"type":"date","valid":true},"quote":{"type":"string","valid":false}},"order":["base","quote","date"],"returns":{"type":"number","unit":"1"}}"#.to_string()
}

/// A second method of a class: a conversion of an amount.
pub fn convert_sig() -> String {
    r#"{"args":{"amount":{"type":"number","unit":{"arg":"base"},"valid":false},"base":{"type":"string","valid":false},"quote":{"type":"string","valid":false}},"order":["base","quote","amount"],"returns":{"type":"number","unit":{"arg":"quote"}}}"#.to_string()
}

/// `{"function": ..., "signature": ..., "bound"?: ...}`
pub fn function_line(name: &str, signature: &str, bound: Option<&str>) -> String {
    let mut m = vec![("function", st(name)), ("signature", parse(signature))];
    if let Some(b) = bound {
        m.push(("bound", st(b)));
    }
    jcs(m)
}

/// `{"descriptor": ...}`
pub fn descriptor_line(descriptor: &str) -> String {
    jcs(vec![("descriptor", parse(descriptor))])
}

/// A class descriptor over the `base`, `quote` keys with the given `(method, function hash)`.
pub fn pair_class(methods: &[(&str, &str)]) -> String {
    class_json(NONE_KEYS_PAIR, &["base", "quote"], methods, HOST, "")
}

/// `{"class": ..., "descriptor": ..., "bound"?: ...}`
pub fn class_line(name: &str, descriptor: &str, bound: Option<&str>) -> String {
    let mut m = vec![("class", st(name)), ("descriptor", parse(descriptor))];
    if let Some(b) = bound {
        m.push(("bound", st(b)));
    }
    jcs(m)
}

/// A cell of an `fx_rate`-shaped function: `base`, `quote`, `date` (valid), a number value.
/// `tx`, `asof` and `issued` are left out when `None`.
pub fn rate_cell(function: &str, value: &str, tx: Option<&str>, asof: Option<&str>, issued: Option<&str>) -> String {
    let args = parse(
        r#"{"base":{"type":"string","value":"EUR"},"date":{"type":"date","value":"2026-03-02"},"quote":{"type":"string","value":"USD"}}"#,
    );
    let cell = Value::Object(vec![("args".into(), args), ("fn".into(), st(function))]);
    let mut m = vec![
        ("cell", cell),
        ("licence", st("CC0-1.0")),
        ("summary", st("Synthetic test data.")),
        ("value", parse(&format!(r#"{{"type":"number","value":"{value}"}}"#))),
    ];
    for (k, v) in [("tx", tx), ("asof", asof), ("issued", issued)] {
        if let Some(v) = v {
            m.push((k, st(v)));
        }
    }
    jcs(m)
}

/// A cell of an `fx_rate`-shaped function whose value is a blob read from `file`.
pub fn blob_cell(function: &str, file: &str, tx: &str) -> String {
    let args = parse(
        r#"{"base":{"type":"string","value":"EUR"},"date":{"type":"date","value":"2026-03-02"},"quote":{"type":"string","value":"USD"}}"#,
    );
    let cell = Value::Object(vec![("args".into(), args), ("fn".into(), st(function))]);
    jcs(vec![
        ("asof", st(tx)),
        ("cell", cell),
        ("licence", st("CC0-1.0")),
        ("summary", st("Synthetic test data.")),
        ("tx", st(tx)),
        ("value", parse(&format!(r#"{{"blob":"{file}","media_type":"text/plain"}}"#))),
    ])
}

pub fn write(lite: &Lite, name: &str, lines: &[String]) -> PathBuf {
    let p = lite.path(name);
    std::fs::write(&p, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    p
}

pub fn opts(now: &'static str) -> PublishOptions {
    PublishOptions { clock: Arc::new(FixedClock(now)), backfill: None }
}

pub fn run(lite: &Lite, path: &Path, now: &'static str) -> Result<Published, PublishError> {
    let owned = vec![HOST.to_string()];
    publish_stored_with(&*lite.store, &lite.key, HOST, &owned, path, &opts(now))
}

/// `Published::descriptors` (0.3.1, H1) read from the `Debug` form, so that the tests compile
/// against 0.3.0 too and fail at runtime there: 0 is not an answer, a missing field panics.
pub fn descriptors_of(p: &Published) -> u64 {
    let d = format!("{p:?}");
    let rest = d.split("descriptors: ").nth(1).unwrap_or_else(|| panic!("Published has no `descriptors` field: {d}"));
    rest.split(|c: char| !c.is_ascii_digit()).next().unwrap().parse().unwrap()
}

/// The keys of the test mesh, for `datumesh_core::verify`.
pub fn keys(lite: &Lite) -> Keys {
    let mut k = Keys::new();
    k.insert_response(HOST, 200, Some(&datumesh_http::keydoc::with_key(HOST, None, &lite.key, K_FROM).unwrap()));
    k
}

/// What a store holds, as one fingerprint of its database file and its write-ahead log: any
/// binding, descriptor, class, blob, receipt or version written changes it, and reads do not.
pub fn snapshot(lite: &Lite) -> String {
    let db = lite.path("store.db");
    let mut all = Vec::new();
    for p in [db.clone(), db.with_file_name("store.db-wal")] {
        let bytes = std::fs::read(&p).unwrap_or_default();
        all.extend_from_slice(format!("{}:{}:", p.file_name().unwrap().to_string_lossy(), bytes.len()).as_bytes());
        all.extend_from_slice(datumesh_core::sha256_hex(&bytes).as_bytes());
    }
    datumesh_core::sha256_hex(&all)
}

/// The cell id of an `fx_rate`-shaped cell of `function` (hash `fn_hash`), as `rate_cell` writes it.
pub fn rate_cell_id(function: &str, fn_hash: &str) -> String {
    use datumesh_core::{Arg, Args, Cell, Scalar, Type};
    let (ns, f) = function.rsplit_once('/').unwrap();
    let mut args = Args::new();
    for (k, t, v) in [("base", Type::String, "EUR"), ("date", Type::Date, "2026-03-02"), ("quote", Type::String, "USD")]
    {
        args.insert(k.to_string(), Arg::Value(Scalar::from_lexical(t, v).unwrap()));
    }
    Cell { ns: ns.to_string(), fn_name: f.to_string(), fn_hash: fn_hash.to_string(), args }.id()
}

/// Asserts that running `lines` as a file refuses at `line` and writes nothing: the snapshot of
/// the store is the same before and after.
pub fn refused_whole(lite: &Lite, name: &str, lines: &[String], line: usize, now: &'static str) -> PublishError {
    let f = write(lite, name, lines);
    let before = snapshot(lite);
    let e = run(lite, &f, now).expect_err("the file must be refused");
    assert_eq!(e.line, line, "refused at the wrong line: {e}");
    assert_eq!(snapshot(lite), before, "a refused file wrote something: {e}");
    e
}

pub fn unbound(lite: &Lite, name: &str) -> bool {
    lite.store.resolve_binding(name, "9999-12-31T23:59:59.999999Z").unwrap().is_none()
}

pub fn now_clock() -> &'static str {
    NOW
}
