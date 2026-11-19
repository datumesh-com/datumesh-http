//! Fixtures of the Build 2 Part 3 tests (CONTRACT-build2-part3-calculus E1, E3, E5 at the mesh).
//!
//! Everything is SYNTHETIC and goes in through `publish-stored` lines (stored functions with cells,
//! `formula` lines, and the new `test` lines), on an SQLite store with a fixed clock, and is served
//! by the real router on loopback. Answers are read as `serde_json::Value`; scenario envelopes are
//! also checked with datumesh-core 0.3.9's `check_answer` (the dev dependency `core039`), which
//! knows overrides and `solve`.
#![allow(dead_code)]

use datumesh_http::publish::{publish_stored_checked, PublishError, Published};
use datumesh_store::Store;
use serde_json::{json, Value};

use crate::common::lite::*;
use crate::common::pub031::{function_line, opts, write};
use crate::common::readpath::*;

pub const PFX: &str = "mesh-test.datumesh.com/scratch_b3/calc";

/// The names of the fixture under a prefix.
#[derive(Clone)]
pub struct Calc {
    pub reading: String,
    pub log: String,
    pub per_unit: String,
    pub avg_log: String,
    pub quarter: String,
    pub band: String,
    pub double: String,
    pub inverse: String,
    pub square: String,
    pub all_sum: String,
    pub scaled: String,
    pub outer: String,
}

impl Calc {
    pub fn new(prefix: &str) -> Calc {
        let n = |x: &str| format!("{prefix}/{x}");
        Calc {
            reading: n("reading"),
            log: n("log"),
            per_unit: n("per_unit"),
            avg_log: n("avg_log"),
            quarter: n("quarter"),
            band: n("band"),
            double: n("double"),
            inverse: n("inverse"),
            square: n("square"),
            all_sum: n("all_sum"),
            scaled: n("scaled"),
            outer: n("outer"),
        }
    }
}

pub fn calc() -> Calc {
    Calc::new(PFX)
}

const STR: &str = r#"{"type":"string","valid":false}"#;
const NUM: &str = r#"{"type":"number","unit":"1","valid":false}"#;
const RET: &str = r#"{"type":"number","unit":"1"}"#;

pub fn site_number_sig(second: &str) -> String {
    format!(r#"{{"args":{{"site":{STR},"{second}":{NUM}}},"order":["site","{second}"],"returns":{RET}}}"#)
}

/// `reading(site) -> number`, stored.
pub fn reading_sig() -> String {
    format!(r#"{{"args":{{"site":{STR}}},"order":["site"],"returns":{RET}}}"#)
}

/// `log(site, day) -> number`, stored.
pub fn log_sig() -> String {
    format!(
        r#"{{"args":{{"day":{{"type":"date","valid":true}},"site":{STR}}},"order":["site","day"],"returns":{RET}}}"#
    )
}

/// `f(d: date in 2026) -> number`.
pub fn quarter_sig() -> String {
    format!(
        r#"{{"args":{{"d":{{"type":"date","valid":true,"extent":{{"lo":"2026-01-01","hi":"2026-12-31"}}}}}},"order":["d"],"returns":{RET}}}"#
    )
}

pub fn x_sig() -> String {
    format!(r#"{{"args":{{"x":{NUM}}},"order":["x"],"returns":{RET}}}"#)
}

pub fn formula_line(name: &str, source: &str, signature: &str, bound: Option<&str>) -> String {
    let mut v =
        json!({"formula": name, "source": source, "signature": serde_json::from_str::<Value>(signature).unwrap()});
    if let Some(b) = bound {
        v["bound"] = json!(b);
    }
    v.to_string()
}

/// A cell line of a stored function (tx and asof the run's clock).
pub fn cell_line(function: &str, args: Value, value: &str) -> String {
    json!({"cell": {"fn": function, "args": args}, "licence": "CC0-1.0", "summary": "Synthetic test data.",
        "value": {"type": "number", "value": value}})
    .to_string()
}

pub fn site_cell(function: &str, site: &str, value: &str) -> String {
    cell_line(function, json!({"site": {"type": "string", "value": site}}), value)
}

pub fn log_cell(function: &str, site: &str, day: &str, value: &str) -> String {
    cell_line(
        function,
        json!({"site": {"type": "string", "value": site}, "day": {"type": "date", "value": day}}),
        value,
    )
}

// ---- the functions under test -----------------------------------------------------------------

/// `per_unit(site, units)`: one divisor, one input cell, one branch point (two arms).
pub const PER_UNIT_BODY: &str = "if units > 0 then {reading}(site: site) / units else 0";
/// `avg_log(site, scale)`: a slice input and a number argument.
pub const AVG_LOG_BODY: &str = "mean({log}(site: site, day: *)) * scale";

pub fn per_unit_line(c: &Calc, bound: Option<&str>) -> String {
    formula_line(&c.per_unit, &PER_UNIT_BODY.replace("{reading}", &c.reading), &site_number_sig("units"), bound)
}

/// A `per_unit` with another body (the rebind of the rerun tests): three times the reading.
pub fn per_unit_v2_line(c: &Calc, bound: Option<&str>) -> String {
    let body = format!("if units > 0 then {}(site: site) * 3 / units else 0", c.reading);
    formula_line(&c.per_unit, &body, &site_number_sig("units"), bound)
}

pub fn avg_log_line(c: &Calc) -> String {
    formula_line(&c.avg_log, &AVG_LOG_BODY.replace("{log}", &c.log), &site_number_sig("scale"), None)
}

pub fn quarter_line(c: &Calc) -> String {
    formula_line(&c.quarter, "month(d)", &quarter_sig(), None)
}

pub fn band_line(c: &Calc) -> String {
    formula_line(&c.band, "if x > 10 then 2 else (if x > 0 then 1 else 0)", &x_sig(), None)
}

/// `scaled(site) = per_unit(site, 2) + 1` and `outer(site) = scaled(site) * 10`.
pub fn scaled_line(c: &Calc) -> String {
    let sig = format!(r#"{{"args":{{"site":{STR}}},"order":["site"],"returns":{RET}}}"#);
    formula_line(&c.scaled, &format!("{}(site: site, units: 2) + 1", c.per_unit), &sig, None)
}

pub fn outer_line(c: &Calc) -> String {
    let sig = format!(r#"{{"args":{{"site":{STR}}},"order":["site"],"returns":{RET}}}"#);
    formula_line(&c.outer, &format!("{}(site: site) * 10", c.scaled), &sig, None)
}

/// The data and the stored functions: `reading` (N = 10, S = 30) and `log` (N: 2026-03-01 = 2, 2026-03-02 = 4).
pub fn data_lines(c: &Calc) -> Vec<String> {
    vec![
        function_line(&c.reading, &reading_sig(), None),
        site_cell(&c.reading, "N", "10"),
        site_cell(&c.reading, "S", "30"),
        function_line(&c.log, &log_sig(), None),
        log_cell(&c.log, "N", "2026-03-01", "2"),
        log_cell(&c.log, "N", "2026-03-02", "4"),
    ]
}

/// The whatif fixture: the data and four computed functions (no tests: published without the rule).
pub fn whatif_lines(c: &Calc) -> Vec<String> {
    let site = format!(r#"{{"args":{{"site":{STR}}},"order":["site"],"returns":{RET}}}"#);
    let mut v = data_lines(c);
    v.push(formula_line(&c.double, &format!("{}(site: site) * 2", c.reading), &site, None));
    v.push(formula_line(&c.inverse, &format!("1 / {}(site: site)", c.reading), &site, None));
    v.push(formula_line(&c.square, &format!("{}(site: site) ^ 2", c.reading), &site, None));
    v.push(formula_line(
        &c.all_sum,
        &format!("sum({}(site: *)) * scale", c.reading),
        &format!(r#"{{"args":{{"scale":{NUM}}},"order":["scale"],"returns":{RET}}}"#),
        None,
    ));
    v
}

// ---- test lines -------------------------------------------------------------------------------

pub fn num(v: &str) -> Value {
    json!({"type": "number", "value": v})
}

pub fn fault(code: &str) -> Value {
    json!({"fault": code})
}

/// `{"test": <function>, "call": <call>, "asof"?, "inputs"?, "expect"}`, as one JSON line.
pub fn test_line(function: &str, call: &str, inputs: Option<Value>, asof: Option<&str>, expect: Value) -> String {
    let mut v = json!({"test": function, "call": call, "expect": expect});
    if let Some(i) = inputs {
        v["inputs"] = i;
    }
    if let Some(a) = asof {
        v["asof"] = json!(a);
    }
    v.to_string()
}

/// The case id of a test line: the first 16 hex digits of the SHA-256 of its JCS (E5).
pub fn case_id(line: &str) -> String {
    let v = datumesh_core::json::parse(line.as_bytes()).unwrap();
    datumesh_core::sha256_hex(&v.to_jcs())[..16].to_string()
}

pub fn pu_call(c: &Calc, site: &str, units: &str) -> String {
    format!("{}(site: \"{site}\", units: {units})", c.per_unit)
}

/// The six tests of `per_unit` that satisfy the public rule: every derived case (zero divisor,
/// absent input, negative number) and both arms. `t[0]` is the then arm, `t[1]` the zero divisor,
/// `t[2]` the negative, `t[3]` the absent input, `t[4]` supplied inputs, `t[5]` a pinned asof.
pub fn per_unit_tests(c: &Calc) -> Vec<String> {
    let f = &c.per_unit;
    let input = json!({ format!("{}(site: \"N\")", c.reading): num("40") });
    vec![
        test_line(f, &pu_call(c, "N", "2"), None, None, num("5")),
        test_line(f, &pu_call(c, "N", "0"), None, None, num("0")),
        test_line(f, &pu_call(c, "N", "-3"), None, None, num("0")),
        test_line(f, &pu_call(c, "ZZ", "2"), None, None, fault("E_FORMULA_ABSENT")),
        test_line(f, &pu_call(c, "N", "2"), Some(input), None, num("20")),
        test_line(f, &pu_call(c, "N", "2"), None, Some(NOW), num("5")),
    ]
}

pub fn avg_call(c: &Calc, site: &str, scale: &str) -> String {
    format!("{}(scale: {scale}, site: \"{site}\")", c.avg_log)
}

/// `avg_log`: positive, negative scale, and the empty slice (a site with no log).
pub fn avg_log_tests(c: &Calc) -> Vec<String> {
    let f = &c.avg_log;
    vec![
        test_line(f, &avg_call(c, "N", "2"), None, None, num("6")),
        test_line(f, &avg_call(c, "N", "-1"), None, None, num("-3")),
        test_line(f, &avg_call(c, "ZZ", "1"), None, None, fault("E_FORMULA_EMPTY")),
    ]
}

pub fn quarter_tests(c: &Calc) -> Vec<String> {
    let f = &c.quarter;
    vec![
        test_line(f, &format!("{f}(d: 2026-01-01)"), None, None, num("1")),
        test_line(f, &format!("{f}(d: 2026-12-31)"), None, None, num("12")),
    ]
}

pub fn band_tests(c: &Calc) -> Vec<String> {
    let f = &c.band;
    ["-1", "5", "20"]
        .iter()
        .zip(["0", "1", "2"])
        .map(|(x, want)| test_line(f, &format!("{f}(x: {x})"), None, None, num(want)))
        .collect()
}

pub fn scaled_tests(c: &Calc) -> Vec<String> {
    let f = &c.scaled;
    vec![
        test_line(f, &format!("{f}(site: \"N\")"), None, None, num("6")),
        test_line(f, &format!("{f}(site: \"ZZ\")"), None, None, fault("E_FORMULA_ABSENT")),
    ]
}

pub fn outer_tests(c: &Calc) -> Vec<String> {
    let f = &c.outer;
    vec![
        test_line(f, &format!("{f}(site: \"N\")"), None, None, num("60")),
        test_line(f, &format!("{f}(site: \"ZZ\")"), None, None, fault("E_FORMULA_ABSENT")),
    ]
}

// ---- publishing -------------------------------------------------------------------------------

/// The publish with the public rule (D8: what datumesh-http does), at `now`.
pub fn run_checked(lite: &Lite, path: &std::path::Path, now: &'static str) -> Result<Published, PublishError> {
    let owned = vec![HOST.to_string()];
    publish_stored_checked(&*lite.store, &lite.key, HOST, &owned, path, &opts(now))
}

/// Lines, as a file, run with the public rule on a fresh store; the error, or the store.
pub fn checked(label: &str, lines: &[String]) -> (Lite, Result<Published, PublishError>) {
    let lite = Lite::new(label);
    let f = write(&lite, "f.jsonl", lines);
    let r = run_checked(&lite, &f, NOW);
    (lite, r)
}

/// Asserts that the public rule refuses `lines` at `line` with `code`, with each of `contains` in
/// the detail, and writes nothing.
pub fn refused(label: &str, lines: &[String], line: usize, code: &str, contains: &[&str]) -> PublishError {
    let lite = Lite::new(label);
    let f = write(&lite, "f.jsonl", lines);
    let before = crate::common::pub031::snapshot(&lite);
    let e = run_checked(&lite, &f, NOW).expect_err("the file must be refused");
    assert_eq!(e.line, line, "refused at the wrong line: {e}");
    assert!(e.detail.contains(code), "the detail names {code}: {e}");
    let kind = format!("{:?}", e.kind);
    let variant = if code == "E_TESTS_MISSING" { "TestsMissing" } else { "TestsFailing" };
    assert!(kind.contains(variant), "the kind is {variant}: {kind}");
    for c in contains {
        assert!(e.detail.contains(c), "the detail lists {c:?}: {e}");
    }
    assert_eq!(crate::common::pub031::snapshot(&lite), before, "a refused file wrote something: {e}");
    e
}

// ---- HTTP helpers -----------------------------------------------------------------------------

/// The function hash `name` is bound to (as of the end of time).
pub fn fn_hash(m: &Served, name: &str) -> String {
    m.lite.store.resolve_binding(name, END).unwrap().unwrap_or_else(|| panic!("{name} is unbound")).fn_hash
}

/// A cell as 4.4 writes it: `args` are `(name, type, lexical)`.
pub fn cell_json(m: &Served, name: &str, args: &[(&str, &str, &str)]) -> Value {
    let (ns, f) = name.rsplit_once('/').unwrap();
    let a: serde_json::Map<String, Value> =
        args.iter().map(|(k, t, v)| (k.to_string(), json!({"type": t, "value": v}))).collect();
    json!({"ns": ns, "fn": f, "fn_hash": fn_hash(m, name), "args": a})
}

/// A slice object of 4.4: the arguments in `open` are written `*`.
pub fn slice_json(m: &Served, name: &str, args: &[(&str, &str, &str)], open: &[&str]) -> Value {
    let mut v = cell_json(m, name, args);
    for o in open {
        v["args"][o] = json!("*");
    }
    v
}

pub fn core_id(v: &Value) -> String {
    let c = datumesh_core::json::parse(&serde_json::to_vec(v).unwrap()).unwrap();
    datumesh_core::Cell::from_value(&c).unwrap().id()
}

pub fn core_slice_id(v: &Value) -> String {
    let c = datumesh_core::json::parse(&serde_json::to_vec(v).unwrap()).unwrap();
    datumesh_core::Slice::from_value(&c).unwrap().id().to_string()
}

/// The canonical request override array (5.9): value overrides `(cell, value)`, sorted by id.
pub fn canonical_overrides(values: &[(Value, &str)]) -> Value {
    let mut items: Vec<(String, Value)> = values
        .iter()
        .map(|(cell, v)| (core_id(cell), json!({"cell": cell, "value": {"type": "number", "value": v}})))
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    Value::Array(items.into_iter().map(|(_, v)| v).collect())
}

/// One slice override in canonical form: `add` and `remove` sorted by cell id, both present.
pub fn canonical_slice_override(slice: &Value, add: &[(Value, &str)], remove: &[Value]) -> Value {
    let mut a: Vec<(String, Value)> = add
        .iter()
        .map(|(cell, v)| (core_id(cell), json!({"cell": cell, "value": {"type": "number", "value": v}})))
        .collect();
    a.sort_by(|x, y| x.0.cmp(&y.0));
    let mut r: Vec<(String, Value)> = remove.iter().map(|c| (core_id(c), c.clone())).collect();
    r.sort_by(|x, y| x.0.cmp(&y.0));
    json!({"slice": slice,
        "add": a.into_iter().map(|x| x.1).collect::<Vec<_>>(),
        "remove": r.into_iter().map(|x| x.1).collect::<Vec<_>>()})
}

pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub json: Value,
}

impl Served {
    /// `POST /whatif` with `body` (JSON), as the mesh document's endpoint says.
    pub async fn whatif(&self, body: &Value) -> Reply {
        self.post_raw("/whatif", serde_json::to_vec(body).unwrap()).await
    }

    pub async fn post_raw(&self, path: &str, body: Vec<u8>) -> Reply {
        let r = self
            .http
            .post(format!("{}{path}", self.base))
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
        let status = r.status().as_u16();
        let body = r.bytes().await.unwrap().to_vec();
        let json = serde_json::from_slice(&body).unwrap_or(Value::Null);
        Reply { status, body, json }
    }

    /// `GET path` of any status.
    pub async fn get_any(&self, path: &str) -> Reply {
        let (status, body) = self.raw(path).await;
        let json = serde_json::from_slice(&body).unwrap_or(Value::Null);
        Reply { status, body, json }
    }
}

/// Checks a whatif answer with core 0.3.9's `check_answer` against the request description built
/// from `cell` (a cell object), `asof`, the canonical `overrides` and `solve` (the vary cell's id);
/// returns the outcome's name.
pub fn check_scenario(
    r: &Reply,
    cell: &Value,
    asof: Option<&str>,
    overrides: &Value,
    solve: Option<&str>,
) -> datumesh_core::Answer {
    let req = datumesh_core::Request::new(&core_id(cell), asof)
        .unwrap()
        .with_whatif(&datumesh_core::json::parse(&serde_json::to_vec(overrides).unwrap()).unwrap(), solve)
        .unwrap();
    datumesh_core::check_answer(r.status, &r.body, &datumesh_core::Keys::new(), &req)
        .unwrap_or_else(|e| panic!("check_answer: {e}: {}", String::from_utf8_lossy(&r.body)))
}

/// The scenario's value as text.
pub fn scenario_value(r: &Reply) -> String {
    assert_eq!(r.json["outcome"], "scenario", "{}", String::from_utf8_lossy(&r.body));
    r.json["scenario"]["value"].as_str().unwrap().to_string()
}

/// A mesh with the test cells switched on (mesh-test, E5), through the environment route.
pub fn test_cells_config() -> datumesh_http::Config {
    datumesh_http::Config::from_vars(|k| match k {
        "DATUMESH_HOST" => Some(HOST.to_string()),
        "DATUMESH_TEST_CELLS" => Some("1".to_string()),
        _ => None,
    })
    .unwrap()
}

/// The argument name of the test cell for the function's hash: the contract writes
/// `test_case(fn_hash: string, case: string)`, but 5.1 reserves `fn_hash` (E_CANON_ARG_NAME). The
/// implementer settles the name with the contract's owner; this constant is the one place that
/// follows.
pub const HASH_ARG: &str = "function_hash";

/// The call of the test cell of `case` of the function with hash `fn_hash`.
pub fn test_case_call(fn_hash: &str, case: &str) -> String {
    format!("{HOST}/test_case(case: \"{case}\", {HASH_ARG}: \"{fn_hash}\")")
}

/// Publishes the fixture runs `(clock, lines)` in order on one store and serves it.
pub async fn serve_runs(
    label: &str,
    runs: &[(&'static str, Vec<String>)],
    base: Option<datumesh_http::Config>,
) -> Served {
    let lite = Lite::new(label);
    for (i, (clock, lines)) in runs.iter().enumerate() {
        let f = write(&lite, &format!("run{i}.jsonl"), lines);
        crate::common::pub031::run(&lite, &f, clock).unwrap_or_else(|e| panic!("run {i} refused: {e}"));
    }
    build_served(lite, base, |_| {}).await
}
