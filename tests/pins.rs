//! spec-0.4.2: pinned requests (`name@<fn_hash>(args)`, `fn_hash=` in the URI, 5.1 and 5.8),
//! the echo of the pin, and a function its name is no longer bound to.

mod common;

use std::time::Duration;

use common::*;
use datumesh_core::json::Value;

const A: &str = "mesh-a.example";
const SIG2: &str = r#"{"args":{"ccy":{"type":"string","valid":false},"date":{"type":"date","valid":true}},"order":["date","ccy"],"returns":{"type":"number","unit":"EUR"}}"#;

fn enc(call: &str) -> String {
    let mut out = String::new();
    for b in call.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:@*".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pinned_requests() {
    let a = Node::start(A, |c| {
        c.budget = Duration::from_millis(500);
        c.aliases.insert("fx".into(), format!("{A}/rate"));
    })
    .await;
    seed(&a, 1500);
    let keys = a.keys();
    let schemas = schemas();
    let rate = format!("{A}/rate");
    let old = bl(|| a.store.resolve_name(&rate, T1).unwrap().unwrap());
    let usd = args(&[("ccy", s("USD")), ("date", d("2026-03-01"))]);
    let old_id = a.cell_id(&rate, &usd);

    // The name bound again, to another function: unpinned, the name now means the new one.
    let new = bl(|| {
        let d = format!(r#"{{"code":{{"body":"","lang":"stored"}},"inputs":[],"signature":{SIG2}}}"#);
        let h = a.store.put_descriptor(d.as_bytes()).unwrap();
        a.store.bind_name(&rate, &h, "2026-06-01T00:00:00.000000Z").unwrap();
        h
    });
    assert_ne!(old, new);
    let r = a.get(&format!("/cell/{rate}?ccy=USD&date=2026-03-01")).await;
    assert_eq!(r.outcome(), "not_covered", "the new function has no value: {}", r.text());
    assert_eq!(r.json().get("fn_hash").and_then(Value::as_str), Some(new.as_str()), "the hash the name resolved to");

    // Pinned to the old function: by URI, by call, by alias; one answer, the pin echoed.
    let by_uri = a.get(&format!("/cell/{rate}?ccy=USD&date=2026-03-01&fn_hash={old}")).await;
    assert_eq!(by_uri.outcome(), "ok", "{}", by_uri.text());
    schemas.assert_envelope(&by_uri.body);
    assert_eq!(by_uri.json().get("fn_hash").and_then(Value::as_str), Some(old.as_str()));
    assert_eq!(check_pinned(&by_uri, &keys, &old_id, None, None, Some(&old)), Ok("ok".into()));
    for c in [
        enc(&format!("{rate}@{old}(\"USD\", 2026-03-01)")),
        enc(&format!("{rate}@{old}(date: 2026-03-01, ccy: \"USD\")")),
        enc(&format!("@fx@{old}(\"USD\", 2026-03-01)")),
    ] {
        let r = a.get(&format!("/cell/{c}")).await;
        assert_eq!(r.body, by_uri.body, "{c}: {}", r.text());
    }
    // With an asof: the pin before asof in the URI.
    let r = a.get(&format!("/cell/{rate}?ccy=USD&date=2026-03-01&fn_hash={old}&asof={T2}")).await;
    assert_eq!(r.outcome(), "ok");
    assert_eq!(check_pinned(&r, &keys, &old_id, Some(T2), None, Some(&old)), Ok("ok".into()));
    assert!(r.header("cache-control").unwrap().contains("immutable"));
    // A pin no descriptor has: not_covered (the mesh does not serve that function).
    let r = a.get(&format!("/cell/{rate}?ccy=USD&date=2026-03-01&fn_hash={}", "0".repeat(64))).await;
    assert_eq!(r.outcome(), "not_covered");
    assert!(r.json().get("error").is_none());
    // Malformed pins.
    let r = a.get(&format!("/cell/{rate}?ccy=USD&date=2026-03-01&fn_hash=xyz")).await;
    assert_eq!(r.json().get("error").and_then(Value::as_str), Some("E_CANON_HEX"));
    let r = a.get(&format!("/cell/{rate}?fn_hash={old}&ccy=USD&date=2026-03-01")).await;
    assert_eq!(r.outcome(), "bad_request", "the pin comes after the arguments: {}", r.text());
    // An unbound name in a call: not_covered with E_NAME_UNBOUND.
    let r = a.get(&format!("/cell/{}", enc(&format!("{A}/nope(\"USD\")")))).await;
    assert_eq!(
        (r.outcome(), r.json().get("error").and_then(Value::as_str)),
        ("not_covered".to_string(), Some("E_NAME_UNBOUND"))
    );

    // pending echoes the pin in its request, and the job's answer carries it.
    let slow = format!("{A}/slow");
    let sh = bl(|| a.store.resolve_name(&slow, T1).unwrap().unwrap());
    let slow_id = a.cell_id(&slow, &args(&[("tag", s("p"))]));
    let p = a.get(&format!("/cell/{slow}?tag=p&fn_hash={sh}")).await;
    assert_eq!(p.status, 202, "{}", p.text());
    schemas.assert_envelope(&p.body);
    assert_eq!(p.json().get("request").and_then(|r| r.get("fn_hash")).and_then(Value::as_str), Some(sh.as_str()));
    assert_eq!(check_pinned(&p, &keys, &slow_id, None, None, Some(&sh)), Ok("pending".into()));
    let job = p.json().get("job").and_then(Value::as_str).unwrap().to_string();
    let done = loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let r = a.get(&format!("/jobs/{job}")).await;
        if r.status != 202 {
            break r;
        }
    };
    assert_eq!(check_pinned(&done, &keys, &slow_id, None, Some(&job), Some(&sh)), Ok("ok".into()));
    assert_eq!(done.json().get("trust").and_then(Value::as_str), Some("primary"));

    // The versions endpoint, pinned.
    let r = a.get(&format!("/versions/{rate}?ccy=USD&date=2026-03-01&fn_hash={old}")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json().get("cell").and_then(|c| c.get("fn_hash")).and_then(Value::as_str), Some(old.as_str()));
}
