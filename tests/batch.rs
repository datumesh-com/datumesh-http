//! Test 8 of the brief: 1,000 cells in one `POST /cells` complete in under 10 times one
//! cell's latency, on a warm store.

mod common;

use std::time::{Duration, Instant};

use common::*;
use datumesh_core::json::Value;

const A: &str = "mesh-a.example";
const SIG: &str = r#"{"args":{"i":{"type":"number","unit":"EUR","valid":false}},"order":["i"],"returns":{"type":"number","unit":"EUR"}}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thousand_cells() {
    let a = Node::start(A, |c| c.rate.enabled = false).await;
    let name = format!("{A}/obs");
    a.stored(&name, SIG);
    let store = a.store.clone();
    let node_key = a.key.clone();
    let fh = bl(|| a.store.resolve_name(&name, T1).unwrap().unwrap());
    // 1,000 values, signed and stored from several threads.
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let (store, key, fh) = (store.clone(), node_key.clone(), fh.clone());
            std::thread::spawn(move || {
                for i in (t..1000).step_by(8) {
                    put_obs(&store, &key, A, "obs", &fh, i, T1);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let body: Vec<String> = (0..1000).map(|i| format!("\"{A}/obs?i={i}\"")).collect();
    let body = format!("[{}]", body.join(",")).into_bytes();
    // Warm: every cell answered once, by the batch itself.
    let r = a.post("/cells", body.clone()).await;
    assert_eq!(r.status, 200);

    let mut single = Vec::new();
    for k in 0..200 {
        let t = Instant::now();
        let r = a.get(&format!("/cell/{A}/obs?i={}", k % 1000)).await;
        single.push(t.elapsed());
        assert_eq!(r.status, 200);
    }
    single.sort();
    let one = pct(&single, 0.5);
    let mut batches = Vec::new();
    let mut server = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        let r = a.post("/cells", body.clone()).await;
        batches.push(t.elapsed());
        server.push(r.header("server-timing").unwrap_or_default().trim_start_matches("app;dur=").to_string());
        let Value::Array(items) = r.json() else { panic!("array") };
        assert_eq!(items.len(), 1000);
        assert!(items
            .iter()
            .all(|e| e.get("envelope").and_then(|x| x.get("outcome")).and_then(Value::as_str) == Some("ok")));
    }
    batches.sort();
    let batch = pct(&batches, 0.5);
    eprintln!("server-side ms: {server:?}");
    let out = format!(
        "batch: one cell p50 {one:?} (p99 {:?}); 1,000 cells in one POST /cells p50 {batch:?} (min {:?}, max {:?}): {:.1} times one cell\n",
        pct(&single, 0.99),
        batches[0],
        batches[batches.len() - 1],
        batch.as_secs_f64() / one.as_secs_f64()
    );
    std::fs::write(out_dir().join("batch.txt"), &out).unwrap();
    eprint!("{out}");
    // Timings hold for an optimised build (`cargo test --release`, as CI runs this test).
    if !cfg!(debug_assertions) {
        assert!(batch < one * 10, "{out}");
    }
    let _ = Duration::ZERO;
}

/// A batch answered in several chunks (one per core, at least 64 requests each) is, byte for
/// byte, the envelopes `/cell` gives for its requests, wrapped as 5.7 says, in request order,
/// and the metrics count its outcomes: whatever each chunk writes on its own, the batch is
/// the same.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chunks_write_the_batch() {
    let a = Node::start(A, |c| c.rate.enabled = false).await;
    let name = format!("{A}/obs");
    a.stored(&name, SIG);
    let fh = bl(|| a.store.resolve_name(&name, T1).unwrap().unwrap());
    bl(|| {
        for i in 0..200 {
            put_obs(&a.store, &a.key, A, "obs", &fh, i, T1);
        }
    });
    // Stored values (ok), values not stored (not_covered), an unbound name (not_covered with
    // E_NAME_UNBOUND) and malformed arguments (bad_request), mixed through every chunk.
    let reqs: Vec<String> = (0..400)
        .map(|k| match k % 10 {
            3 => format!("{A}/nobody?i={k}"),
            7 => format!("{A}/obs?i=0{k}"),
            9 => format!("{A}/obs?i={}", 1000 + k),
            _ => format!("{A}/obs?i={}", k % 200),
        })
        .collect();
    let body: Vec<String> = reqs.iter().map(|r| format!("\"{r}\"")).collect();
    let r = a.post("/cells", format!("[{}]", body.join(",")).into_bytes()).await;
    assert_eq!(r.status, 200, "{}", r.text());

    let mut want = b"[".to_vec();
    let mut outcomes = std::collections::BTreeMap::<String, u64>::new();
    for (k, req) in reqs.iter().enumerate() {
        let one = a.get(&format!("/cell/{req}")).await;
        if k > 0 {
            want.push(b',');
        }
        want.extend_from_slice(b"{\"envelope\":");
        want.extend_from_slice(&one.body);
        want.extend_from_slice(format!(",\"status\":{}}}", one.status).as_bytes());
        *outcomes.entry(one.outcome()).or_default() += 1;
    }
    want.push(b']');
    assert_eq!(r.text(), String::from_utf8(want).unwrap());
    assert_eq!(outcomes.len(), 3, "{outcomes:?}");

    let m = a.get("/metrics").await.text();
    for (o, n) in &outcomes {
        let line = format!("datumesh_envelopes_total{{route=\"/cells\",outcome=\"{o}\"}} {n}");
        assert!(m.lines().any(|l| l == line), "{line} in\n{m}");
    }
}
