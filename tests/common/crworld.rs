//! Fixtures of the classes-round tests (CONTRACT-classes-round.md K5, K7 to K13): worlds published through `publish-stored`
//! lines on the 1.8b SQLite store (fixed clock `NOW`), served by the real router with the mesh clock `MESH_NOW`, and asked
//! over HTTP. Everything is synthetic.
#![allow(dead_code)]

use datumesh_core::json::Value as CoreValue;
use datumesh_http::publish::{PublishError, Published};
use datumesh_store::Store;
use serde_json::{json, Value};

use crate::common::lite::*;
use crate::common::pub031::{class_line, descriptor_line, function_line, run, write, BEFORE};
use crate::common::readpath::*;
use crate::common::world::{call_path, class_json};

pub const STR: &str = r#"{"type":"string","valid":false}"#;
pub const NUM1: &str = r#"{"type":"number","unit":"1"}"#;
pub const DATE_V: &str = r#"{"type":"date","valid":true}"#;
pub const NONE_STR: &str = r#"{"key_normalisation":"none","type":"string"}"#;
/// The day of the rates cells.
pub const DAY: &str = "2026-03-02";
/// The second run's clock (two seconds after `NOW`), and an instant between the two runs.
pub const T2: &str = "2026-11-15T12:00:02.000000Z";
pub const BETWEEN: &str = "2026-11-15T12:00:01.000000Z";

/// The hash `name` is bound to (as of the end of time).
pub fn fn_hash(lite: &Lite, name: &str) -> String {
    lite.store.resolve_binding(name, END).unwrap().unwrap_or_else(|| panic!("{name} is unbound")).fn_hash
}

/// The class hash `name` is bound to (as of the end of time).
pub fn class_hash(lite: &Lite, name: &str) -> String {
    lite.store.resolve_class(name, END).unwrap().unwrap_or_else(|| panic!("class {name} is unbound"))
}

/// Publishes `lines` as one file at `clock`.
pub fn publish(lite: &Lite, file: &str, lines: &[String], clock: &'static str) -> Result<Published, PublishError> {
    let f = write(lite, file, lines);
    run(lite, &f, clock)
}

pub fn formula_line(name: &str, source: &str, signature: &str) -> String {
    json!({"formula": name, "source": source, "signature": serde_json::from_str::<Value>(signature).unwrap()}).to_string()
}

/// A cell line of a stored function: `args` are `(name, type, lexical)`, the value is `(type, lexical)`.
pub fn cell_line(function: &str, args: &[(&str, &str, &str)], value: (&str, &str)) -> String {
    let a: serde_json::Map<String, Value> = args.iter().map(|(k, t, v)| (k.to_string(), json!({"type": t, "value": v}))).collect();
    json!({"cell": {"fn": function, "args": a}, "licence": "CC0-1.0", "summary": "Synthetic test data.",
        "value": {"type": value.0, "value": value.1}})
    .to_string()
}

/// A cell line whose value is the JSON `true` or `false` (a boolean as a cell line writes it).
pub fn bool_cell_line(function: &str, args: &[(&str, &str, &str)], value: bool) -> String {
    let a: serde_json::Map<String, Value> = args.iter().map(|(k, t, v)| (k.to_string(), json!({"type": t, "value": v}))).collect();
    json!({"cell": {"fn": function, "args": a}, "licence": "CC0-1.0", "summary": "Synthetic test data.",
        "value": {"type": "boolean", "value": value}})
    .to_string()
}

/// A catalogue line of a function.
pub fn catalog_line_of(name: &str, kind: &str, extra: Value) -> String {
    let mut v = json!({"catalog": name, "kind": kind, "label": "A synthetic name", "description": "A synthetic description.",
        "examples": [], "dictionary": {}});
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    v.to_string()
}

/// The path of a call, percent-encoded, with its `?asof=`.
pub fn path(call: &str, asof: Option<&str>) -> String {
    let mut p = format!("/cell/{}", call_path(call));
    if let Some(a) = asof {
        p.push_str(&format!("?asof={a}"));
    }
    p
}

/// What `/cell` or `/describe` answered.
pub struct Ans {
    pub status: u16,
    pub v: Value,
    pub text: String,
}

pub async fn ask(m: &Served, path: &str) -> Ans {
    let (status, body) = m.raw(path).await;
    let text = String::from_utf8_lossy(&body).into_owned();
    let v = serde_json::from_slice(&body).unwrap_or(Value::Null);
    Ans { status, v, text }
}

impl Ans {
    pub fn outcome(&self) -> &str {
        self.v["outcome"].as_str().unwrap_or("")
    }
    pub fn error(&self) -> Option<&str> {
        self.v["error"].as_str()
    }
    pub fn fault(&self) -> Option<&str> {
        self.v["fault"].as_str()
    }
    /// The receipt's value text.
    pub fn value(&self) -> Option<&str> {
        self.v["receipt"]["value"].as_str()
    }
    pub fn kind(&self) -> Option<&str> {
        self.v["kind"].as_str()
    }
    /// The id of the cell of the receipt.
    pub fn cell_id(&self) -> String {
        let bytes = serde_json::to_vec(&self.v["receipt"]).unwrap();
        datumesh_core::Receipt::from_json(&bytes).unwrap_or_else(|e| panic!("{e}: {}", self.text)).cell().id()
    }
    /// The receipt's hash: what another receipt lists as its input (SPEC 3 and 8.1: receipt inputs are receipt hashes).
    pub fn receipt_hash(&self) -> String {
        let bytes = serde_json::to_vec(&self.v["receipt"]).unwrap();
        datumesh_core::Receipt::from_json(&bytes).unwrap_or_else(|e| panic!("{e}: {}", self.text)).hash().to_string()
    }
    /// The receipt hashes the receipt lists as its inputs.
    pub fn inputs(&self) -> Vec<String> {
        self.v["receipt"]["inputs"].as_array().map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect()).unwrap_or_default()
    }
    /// `resolved.resolutions` as `(from, kind, to)`.
    pub fn resolutions(&self) -> Vec<(String, String, String)> {
        let rs = self.v["resolved"]["resolutions"].as_array().unwrap_or_else(|| panic!("no resolved: {}", self.text));
        rs.iter()
            .map(|x| {
                let f = |k: &str| x[k].as_str().unwrap_or("").to_string();
                (f("from"), f("kind"), f("to"))
            })
            .collect()
    }
    pub fn resolution_kinds(&self) -> Vec<String> {
        self.resolutions().into_iter().map(|r| r.1).collect()
    }
}

/// The key of an `Value` as core's.
pub fn core_json(v: &Value) -> CoreValue {
    datumesh_core::json::parse(&serde_json::to_vec(v).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------------------------------------------------
// The rates world (K5): a parent class and a subclass that adds a key, a method, and overrides one.

pub fn rates_p() -> String {
    format!("{HOST}/rates/fx_pair")
}
pub fn rates_s() -> String {
    format!("{HOST}/rates/fx_ndf")
}

fn rate_sig() -> String {
    format!(
        r#"{{"args":{{"base":{STR},"date":{DATE_V},"quote":{STR}}},"order":["base","quote","date"],"returns":{NUM1}}}"#
    )
}

fn fixed_sig() -> String {
    format!(
        r#"{{"args":{{"base":{STR},"date":{DATE_V},"fixing":{STR},"quote":{STR}}},"order":["base","quote","fixing","date"],"returns":{NUM1}}}"#
    )
}

fn via_pair_sig(class: &str) -> String {
    format!(
        r#"{{"args":{{"date":{DATE_V},"pair":{{"classes":["{class}"],"type":"string","valid":false}}}},"order":["pair","date"],"returns":{NUM1}}}"#
    )
}

fn date_only_sig() -> String {
    format!(r#"{{"args":{{"date":{DATE_V}}},"order":["date"],"returns":{NUM1}}}"#)
}

pub fn via_pair() -> String {
    format!("{HOST}/rates/via_pair")
}
pub fn via_object() -> String {
    format!("{HOST}/rates/via_object")
}

/// A rates cell: `P/rate(base, quote, date)` is `value`.
pub fn rates_cell(base: &str, quote: &str, value: &str) -> String {
    cell_line(
        &format!("{}/rate", rates_p()),
        &[("base", "string", base), ("date", "date", DAY), ("quote", "string", quote)],
        ("number", value),
    )
}

/// The first file of the rates world: the parent's methods (`rate` stored; `cross` and `inverse` computed), the subclass's own
/// (`fixed` stored, and an override of `cross`), and the rate cells. Run at `BEFORE`: a class binds its methods' names, which
/// must be bound earlier than the class.
pub fn rates_functions() -> Vec<String> {
    let (p, s) = (rates_p(), rates_s());
    let cross_sig = format!(
        r#"{{"args":{{"base":{STR},"date":{DATE_V},"quote":{STR}}},"order":["base","quote","date"],"returns":{NUM1}}}"#
    );
    vec![
        function_line(&format!("{p}/rate"), &rate_sig(), None),
        formula_line(
            &format!("{p}/cross"),
            &format!(r#"{p}/rate(base: "EUR", quote: quote, date: date) / {p}/rate(base: "EUR", quote: base, date: date)"#),
            &cross_sig,
        ),
        formula_line(&format!("{p}/inverse"), &format!("1 / {p}/rate(base: base, quote: quote, date: date)"), &rate_sig()),
        function_line(&format!("{s}/fixed"), &fixed_sig(), None),
        formula_line(
            &format!("{s}/cross"),
            &format!("{p}/cross(base: base, quote: quote, date: date) * 2"),
            &cross_sig,
        ),
        rates_cell("EUR", "USD", "1.2"),
        rates_cell("EUR", "GBP", "0.8"),
    ]
}

/// The second file: the two classes (`fx_ndf` has the parent `fx_pair`, adds the key `fixing`, the method `fixed`, and
/// overrides `cross`). Run after [`rates_functions`], at the same clock.
pub fn rates_classes(lite: &Lite) -> Vec<String> {
    let (p, s) = (rates_p(), rates_s());
    let h = |n: &str| fn_hash(lite, n);
    let parent = class_json(
        crate::common::world::NONE_KEYS_PAIR,
        &["base", "quote"],
        &[("cross", &h(&format!("{p}/cross"))), ("inverse", &h(&format!("{p}/inverse"))), ("rate", &h(&format!("{p}/rate")))],
        HOST,
        "",
    );
    let child_keys = format!(r#"{{"fixing":{NONE_STR}}}"#);
    let parent_hash = datumesh_core::ClassDescriptor::from_json(parent.as_bytes()).unwrap().hash();
    let child = class_json(
        &child_keys,
        &["fixing"],
        &[("cross", &h(&format!("{s}/cross"))), ("fixed", &h(&format!("{s}/fixed")))],
        HOST,
        &format!(r#""parent":"{parent_hash}","#),
    );
    vec![class_line(&p, &parent, None), class_line(&s, &child, None)]
}

/// The third file: formulas over the subclass (a dispatch over `[fx_ndf]` and an object with literal keys).
pub fn rates_formulas() -> Vec<String> {
    let s = rates_s();
    vec![
        formula_line(&via_pair(), "pair.rate(date: date)", &via_pair_sig(&s)),
        formula_line(&via_object(), &format!(r#"{s}("EUR", "USD", "F1").rate(date: date)"#), &date_only_sig()),
    ]
}

/// The rates world, served. With `rebind`, a second run at `T2` binds the parent class's name to another descriptor (it gains
/// a method `extra`), so that no class name is bound to the hash the subclass's `parent` names from `T2` on.
pub async fn rates(label: &str, rebind: bool) -> Served {
    rates_with(label, rebind, false).await
}

/// [`rates`], with the two formulas over the subclass (`via_pair`, `via_object`) when `formulas`: they are a separate file
/// because they cannot be defined before K5 (the formula resolves `fx_ndf/rate` and wants the subclass's three keys first),
/// and the tests that do not need them must not fail on that.
pub async fn rates_with(label: &str, rebind: bool, formulas: bool) -> Served {
    let lite = Lite::new(label);
    publish(&lite, "functions.jsonl", &rates_functions(), BEFORE).unwrap_or_else(|e| panic!("functions: {e}"));
    let classes = rates_classes(&lite);
    publish(&lite, "classes.jsonl", &classes, NOW).unwrap_or_else(|e| panic!("classes: {e}"));
    if formulas {
        publish(&lite, "formulas.jsonl", &rates_formulas(), T2).unwrap_or_else(|e| panic!("formulas: {e}"));
    }
    if rebind {
        let p = rates_p();
        let extra = crate::common::pub031::stored_descriptor(&rate_sig());
        let extra_hash = crate::common::pub031::descriptor_hash(&extra);
        let h = |n: &str| fn_hash(&lite, n);
        let parent2 = class_json(
            crate::common::world::NONE_KEYS_PAIR,
            &["base", "quote"],
            &[
                ("cross", &h(&format!("{p}/cross"))),
                ("extra", &extra_hash),
                ("inverse", &h(&format!("{p}/inverse"))),
                ("rate", &h(&format!("{p}/rate"))),
            ],
            HOST,
            "",
        );
        publish(&lite, "rebind.jsonl", &[descriptor_line(&extra), class_line(&p, &parent2, None)], T2)
            .unwrap_or_else(|e| panic!("rebind: {e}"));
    }
    build_served(lite, None, |_| {}).await
}

// ---------------------------------------------------------------------------------------------------------------------
// The shop world (K8 labels, K10 chains, K9 extensions): the class `shop/product` (key `sku`, `upper_ascii`; methods `price`,
// `name`, `successor` returning a key of the class, and the lookup `from_label` whose `label` is under `label_fold`) and the
// class `shop/crate` (key `id`).

pub fn product() -> String {
    format!("{HOST}/shop/product")
}
pub fn crate_class() -> String {
    format!("{HOST}/shop/crate")
}

pub const SKU_ARG: &str = r#"{"key_normalisation":"upper_ascii","type":"string","valid":false}"#;
pub const LABEL_ARG: &str = r#"{"key_normalisation":"label_fold","type":"string","valid":false}"#;

fn price_sig() -> String {
    format!(r#"{{"args":{{"date":{DATE_V},"sku":{SKU_ARG}}},"order":["sku","date"],"returns":{NUM1}}}"#)
}
fn name_sig() -> String {
    format!(r#"{{"args":{{"sku":{SKU_ARG}}},"order":["sku"],"returns":"string"}}"#)
}
fn successor_sig() -> String {
    format!(r#"{{"args":{{"sku":{SKU_ARG}}},"order":["sku"],"returns":{{"classes":["{}"],"type":"string"}}}}"#, product())
}
fn from_label_sig() -> String {
    format!(r#"{{"args":{{"label":{LABEL_ARG}}},"order":["label"],"returns":{{"classes":["{}"],"type":"string"}}}}"#, product())
}
fn weight_sig() -> String {
    format!(r#"{{"args":{{"id":{STR}}},"order":["id"],"returns":{NUM1}}}"#)
}

/// The key encoding of a product (16.5).
pub fn pk(sku: &str) -> String {
    format!(r#"{}(sku:"{sku}")"#, product())
}

pub fn shop_functions() -> Vec<String> {
    let p = product();
    vec![
        function_line(&format!("{p}/price"), &price_sig(), None),
        function_line(&format!("{p}/name"), &name_sig(), None),
        function_line(&format!("{p}/successor"), &successor_sig(), None),
        function_line(&format!("{p}/from_label"), &from_label_sig(), None),
        function_line(&format!("{}/weight", crate_class()), &weight_sig(), None),
    ]
}

pub fn shop_classes(lite: &Lite) -> Vec<String> {
    let (p, c) = (product(), crate_class());
    let h = |n: &str| fn_hash(lite, n);
    let pc = class_json(
        &format!(r#"{{"sku":{{"key_normalisation":"upper_ascii","type":"string"}}}}"#),
        &["sku"],
        &[
            ("from_label", &h(&format!("{p}/from_label"))),
            ("name", &h(&format!("{p}/name"))),
            ("price", &h(&format!("{p}/price"))),
            ("successor", &h(&format!("{p}/successor"))),
        ],
        HOST,
        r#""lookup":"from_label","#,
    );
    let cc = class_json(&format!(r#"{{"id":{NONE_STR}}}"#), &["id"], &[("weight", &h(&format!("{c}/weight")))], HOST, "");
    vec![class_line(&p, &pc, None), class_line(&c, &cc, None)]
}

pub fn label_cell(label: &str, to: &str) -> String {
    cell_line(&format!("{}/from_label", product()), &[("label", "string", label)], ("string", to))
}

pub fn price_cell_of(sku: &str, day: &str, v: &str) -> String {
    cell_line(&format!("{}/price", product()), &[("date", "date", day), ("sku", "string", sku)], ("number", v))
}

pub fn successor_cell(sku: &str, to: &str) -> String {
    cell_line(&format!("{}/successor", product()), &[("sku", "string", sku)], ("string", to))
}

/// The day of the shop's prices.
pub const SHOP_DAY: &str = "2026-03-16";

/// The cells of the shop: labels (`apple`, `green apple`, `strasse`, and `old`, to be withdrawn), prices, successors
/// (`A1` -> `P1` -> `Q1`; `A2` -> a key of another class; `K1` has none), a name.
pub fn shop_cells() -> Vec<String> {
    vec![
        label_cell("apple", &pk("A1")),
        label_cell("green apple", &pk("A2")),
        label_cell("strasse", &pk("S1")),
        label_cell("old", &pk("A3")),
        price_cell_of("A1", SHOP_DAY, "1.5"),
        price_cell_of("A2", SHOP_DAY, "2.5"),
        price_cell_of("S1", SHOP_DAY, "9"),
        price_cell_of("A3", SHOP_DAY, "7"),
        price_cell_of("P1", SHOP_DAY, "3"),
        price_cell_of("Q1", SHOP_DAY, "4"),
        price_cell_of("K1", SHOP_DAY, "6"),
        successor_cell("A1", &pk("P1")),
        successor_cell("P1", &pk("Q1")),
        successor_cell("A2", &format!(r#"{}(id:"c1")"#, crate_class())),
        cell_line(&format!("{}/name", product()), &[("sku", "string", "A1")], ("string", "Apple")),
    ]
}

/// The shop, served. With `withdrawn`, a second run at `T2` withdraws the label `old` (a new version of its lookup cell whose
/// value is the empty string, K8).
pub async fn shop(label: &str, withdrawn: bool) -> Served {
    let lite = Lite::new(label);
    publish(&lite, "functions.jsonl", &shop_functions(), BEFORE).unwrap_or_else(|e| panic!("functions: {e}"));
    let mut v = shop_classes(&lite);
    v.extend(shop_cells());
    publish(&lite, "classes.jsonl", &v, NOW).unwrap_or_else(|e| panic!("classes and cells: {e}"));
    if withdrawn {
        publish(&lite, "withdraw.jsonl", &[label_cell("old", "")], T2).unwrap_or_else(|e| panic!("withdrawal: {e}"));
    }
    build_served(lite, None, |_| {}).await
}

// ---------------------------------------------------------------------------------------------------------------------
// The filing world (K11): a class keyed by a CIK (a string of digits), a method keyed by an accession (a string of digits and
// hyphens), and a plain function with a number position, to say what the same runs are there.

pub fn company() -> String {
    format!("{HOST}/sec/company")
}
pub fn reading() -> String {
    format!("{HOST}/sec/reading")
}
pub const CIK: &str = "0000320193";
pub const ACCN: &str = "0000320193-24-000123";

fn filed_sig() -> String {
    format!(r#"{{"args":{{"accession":{STR},"cik":{NONE_STR_ARG}}},"order":["cik","accession"],"returns":"date"}}"#)
}
const NONE_STR_ARG: &str = STR;
fn reading_sig() -> String {
    format!(
        r#"{{"args":{{"accession":{STR},"cik":{STR},"n":{{"type":"number","unit":"1","valid":false}}}},"order":["cik","accession","n"],"returns":{NUM1}}}"#
    )
}

pub fn filing_functions() -> Vec<String> {
    vec![
        function_line(&format!("{}/filed", company()), &filed_sig(), None),
        function_line(&reading(), &reading_sig(), None),
    ]
}

pub fn filing_classes(lite: &Lite) -> Vec<String> {
    let c = company();
    let d = class_json(
        &format!(r#"{{"cik":{NONE_STR}}}"#),
        &["cik"],
        &[("filed", &fn_hash(lite, &format!("{c}/filed")))],
        HOST,
        "",
    );
    vec![
        class_line(&c, &d, None),
        cell_line(&format!("{c}/filed"), &[("accession", "string", ACCN), ("cik", "string", CIK)], ("date", "2024-11-01")),
        cell_line(&reading(), &[("accession", "string", ACCN), ("cik", "string", CIK), ("n", "number", "5")], ("number", "42")),
    ]
}

pub async fn filing(label: &str) -> Served {
    let lite = Lite::new(label);
    publish(&lite, "functions.jsonl", &filing_functions(), BEFORE).unwrap_or_else(|e| panic!("functions: {e}"));
    publish(&lite, "classes.jsonl", &filing_classes(&lite), NOW).unwrap_or_else(|e| panic!("classes: {e}"));
    build_served(lite, None, |_| {}).await
}
