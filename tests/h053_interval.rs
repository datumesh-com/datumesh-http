//! http 0.5.3, CONTRACT-protocol-part1 H3 on `/x/interval`: the query `asof` takes the forms of an instant.
//! Written first; FAIL until then.
//!
//! The contract says "the query `asof` on every read endpoint". `/x/interval` has no `asof` parameter in 0.5.2
//! (`E_PARSE_URI`; it reads as of the mesh's now), so this test also asks it to take one (see the report).
//!
//! Fixture: a one-argument daily series; the cell 2026-03-05 is 4 as of 2026-03-05T12:00Z, 5 as of 2026-03-06T12:00Z
//! and 6 as of 2026-03-07T12:00Z.

mod common;
#[path = "common/h0410.rs"]
mod h0410;

use common::lite::*;
use common::pub031::function_line;
use common::readpath::catalog_line;
use h0410::*;
use serde_json::{json, Value};

const POLICY: &str = "mesh-test.datumesh.com/relay/ecb.europa.eu/policy_rate";
const CLOCK: &str = "2026-01-01T00:00:00.000000Z";

fn daily(args: Value, value: &str, day: &str) -> String {
    let tx = format!("{day}T12:00:00.000000Z");
    let source = json!({"name": "ecb.europa.eu", "url": "https://ecb.europa.eu/data", "retrieved": CLOCK,
        "takeover": format!("https://{HOST}/takeover/ecb.europa.eu")});
    json!({"cell": {"fn": POLICY, "args": args}, "licence": "CC0-1.0", "summary": "Synthetic test data.",
        "value": {"type": "number", "value": value}, "source": source, "tx": tx, "asof": tx, "issued": tx})
    .to_string()
}

/// (form, canonical instant, value)
const FORMS: &[(&str, &str, &str)] = &[
    ("2026-03-06T12:00:00Z", "2026-03-06T12:00:00.000000Z", "5"),
    ("2026-03-06T11:59:59.9Z", "2026-03-06T11:59:59.900000Z", "4"),
    ("2026-03-06T12:00:00.5Z", "2026-03-06T12:00:00.500000Z", "5"),
    ("2026-03-06T12:00:00.000001Z", "2026-03-06T12:00:00.000001Z", "5"),
    ("2026-03-05", "2026-03-05T23:59:59.999999Z", "4"),
    ("2026-03-06", "2026-03-06T23:59:59.999999Z", "5"),
];

async fn world(label: &str) -> W {
    let fx = Fx::lite(label);
    let sig = r#"{"args":{"date":{"type":"date","valid":true}},"order":["date"],"returns":{"type":"number","unit":"1"}}"#;
    fx.publish(&[function_line(POLICY, sig, None)], CLOCK);
    let d = json!({"date": {"type": "date", "value": "2026-03-05"}});
    let mut v = vec![daily(d.clone(), "4", "2026-03-05"), daily(d.clone(), "5", "2026-03-06"), daily(d, "6", "2026-03-07")];
    v.push(catalog_line(POLICY, "function", "Policy rate", "The rate.", json!({"date": "the day"}), json!({})));
    fx.publish(&v, NOW);
    let p = fx.dir.join("series.json");
    std::fs::write(
        &p,
        json!([{"function": POLICY, "granularity": "month", "date_arg": "date", "unit": "percent", "kind": "relayed"}]).to_string(),
    )
    .unwrap();
    let path = p.to_string_lossy().into_owned();
    fx.serve(config(&[("DATUMESH_SERIES_FILE", &path)], &[])).await
}

async fn interval(w: &W, asof: Option<&str>) -> Rep {
    let q = asof.map(|a| format!("&asof={a}")).unwrap_or_default();
    let path = format!("/x/interval/{POLICY}?from=2026-03-05&to=2026-03-05{q}");
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let r = w.get(&path).await;
        if r.status != 503 || std::time::Instant::now() > end {
            return r;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h3_interval_in_every_form_answers_as_the_canonical_form_does() {
    let w = world("h053-interval").await;
    for (form, canon, want) in FORMS {
        let r = interval(&w, Some(form)).await;
        assert_eq!(r.status, 200, "{form}: {}", r.text());
        let v = r.json();
        assert_eq!(v["members"][0]["value"], *want, "{form}: {v}");
        assert_eq!(v["asof"], *canon, "{form}: the answer's asof is the canonical instant");
        let c = interval(&w, Some(canon)).await.json();
        assert_eq!(v["members"], c["members"], "{form}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h3_interval_refuses_an_offset() {
    let w = world("h053-interval-offset").await;
    for bad in ["2026-03-06T12:00:00%2B01:00", "2026-03-06T12:00:00.5-05:00"] {
        let r = interval(&w, Some(bad)).await;
        let v = r.json();
        assert_eq!(
            (r.status, v["outcome"].as_str(), v["error"].as_str()),
            (400, Some("bad_request"), Some("E_CANON_INSTANT")),
            "{bad}: {}",
            r.text()
        );
    }
}
