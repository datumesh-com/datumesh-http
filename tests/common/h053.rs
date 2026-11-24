//! Helpers of the 0.5.3 tests (CONTRACT-protocol-part1, datumesh-http 0.5.3: H1-H3), on the shared `Node` harness.
#![allow(dead_code)]

pub use crate::common::*;

pub const A: &str = "mesh-a.example";

/// `A/px(ccy, date)`, stored, with the cell (USD, 2026-05-01) written three times and (USD, 2026-05-02) once:
/// 1 at V1, 2 at V2 (half a second later), 3 at V3 (the next day); (USD, 2026-05-02) is 10 at V1.
pub const V1: &str = "2026-05-10T12:00:00.000000Z";
pub const V2: &str = "2026-05-10T12:00:00.500000Z";
pub const V3: &str = "2026-05-11T12:00:00.000000Z";

pub async fn px_world() -> Node {
    let node = Node::start(A, |c| c.rate.enabled = false).await;
    let px = format!("{A}/px");
    node.stored(&px, RATE_SIG);
    let k1 = args(&[("ccy", s("USD")), ("date", d("2026-05-01"))]);
    let k2 = args(&[("ccy", s("USD")), ("date", d("2026-05-02"))]);
    node.put_value(&px, &k1, n("1"), V1);
    node.put_value(&px, &k1, n("2"), V2);
    node.put_value(&px, &k1, n("3"), V3);
    node.put_value(&px, &k2, n("10"), V1);
    // a formula over it, and an input to override in a what-if
    node.stored(&format!("{A}/off"), r#"{"args":{"k":{"type":"string","valid":false}},"order":["k"],"returns":{"type":"number","unit":"EUR"}}"#);
    node.put_value(&format!("{A}/off"), &args(&[("k", s("x"))]), n("100"), V1);
    node.formula(
        &format!("{A}/plus_off"),
        &format!("{A}/px(ccy: ccy, date: date) + {A}/off(k: \"x\")"),
        RATE_SIG,
    );
    node
}

/// The forms of an instant a request may carry, the canonical instant each means, and the value of
/// `px(USD, 2026-05-01)` as of it.
pub const FORMS: &[(&str, &str, &str)] = &[
    ("2026-05-10T12:00:00Z", "2026-05-10T12:00:00.000000Z", "1"),
    ("2026-05-10T12:00:00.4Z", "2026-05-10T12:00:00.400000Z", "1"),
    ("2026-05-10T12:00:00.49999Z", "2026-05-10T12:00:00.499990Z", "1"),
    ("2026-05-10T12:00:00.5Z", "2026-05-10T12:00:00.500000Z", "2"),
    ("2026-05-10T12:00:00.500000Z", "2026-05-10T12:00:00.500000Z", "2"),
    ("2026-05-10T12:00:00.50Z", "2026-05-10T12:00:00.500000Z", "2"),
    // a date: the end of that day, UTC (D7.1)
    ("2026-05-10", "2026-05-10T23:59:59.999999Z", "2"),
    ("2026-05-11", "2026-05-11T23:59:59.999999Z", "3"),
];

pub fn jv(r: &Resp) -> serde_json::Value {
    serde_json::from_slice(&r.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", r.text()))
}

pub fn jv_bytes(body: &[u8]) -> serde_json::Value {
    serde_json::from_slice(body).unwrap()
}
