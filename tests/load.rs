//! Test 11 of the brief: 1,000 concurrent warm `get`s: p50, p99, error rate and pool
//! saturation; no request blocks longer than the budget.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;

const A: &str = "mesh-a.example";
const SIG: &str = r#"{"args":{"i":{"type":"number","unit":"EUR","valid":false}},"order":["i"],"returns":{"type":"number","unit":"EUR"}}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn thousand_concurrent_gets() {
    let a = Node::start(A, |c| {
        c.rate.enabled = false;
        c.budget = Duration::from_secs(1);
    })
    .await;
    fast_commits(&a);
    a.stored(&format!("{A}/obs"), SIG);
    a.formula(&format!("{A}/twice"), &format!("{A}/obs(i: i) * 2"), SIG);
    let node = &a;
    tokio::task::block_in_place(|| load_obs(node, "obs", 200, 8, T1));
    // Warm: each cell (100 stored, 100 computed) once.
    let paths: Arc<Vec<String>> = Arc::new(
        (0..200)
            .map(|i| if i % 2 == 0 { format!("/cell/{A}/obs?i={i}") } else { format!("/cell/{A}/twice?i={i}") })
            .collect(),
    );
    for p in paths.iter() {
        assert_eq!(a.get(p).await.status, 200, "{p}");
    }
    let budget = a.mesh.config().budget;
    let mut report = String::new();
    for round in 0..3 {
        let before = a.get("/metrics").await.text();
        let start = Arc::new(tokio::sync::Barrier::new(1000));
        let mut tasks = Vec::new();
        for k in 0..1000usize {
            let (http, base, paths, start) = (a.http.clone(), a.base.clone(), paths.clone(), start.clone());
            tasks.push(tokio::spawn(async move {
                start.wait().await;
                let t = Instant::now();
                let r = http.get(format!("{base}{}", paths[k % paths.len()])).send().await;
                let status = match r {
                    Ok(r) => {
                        let s = r.status().as_u16();
                        let _ = r.bytes().await;
                        s
                    }
                    Err(_) => 0,
                };
                (t.elapsed(), status)
            }));
        }
        let mut lat = Vec::new();
        let mut errors = 0;
        for t in tasks {
            let (d, s) = t.await.unwrap();
            if s != 200 {
                errors += 1;
            }
            lat.push(d);
        }
        lat.sort();
        let after = a.get("/metrics").await.text();
        let waited = metric(&after, "datumesh_pool_waited_total") - metric(&before, "datumesh_pool_waited_total");
        let runs = metric(&after, "datumesh_pool_runs_total") - metric(&before, "datumesh_pool_runs_total");
        let line = format!(
            "load round {round}: 1000 concurrent warm gets: p50 {:?}, p99 {:?}, max {:?}, errors {errors} ({:.1}%), pool {} threads, peak in use {}, {waited} of {runs} pool tasks waited for a thread\n",
            pct(&lat, 0.5),
            pct(&lat, 0.99),
            lat[lat.len() - 1],
            errors as f64 / 10.0,
            metric(&after, "datumesh_pool_size"),
            metric(&after, "datumesh_pool_peak"),
        );
        eprint!("{line}");
        report.push_str(&line);
        assert_eq!(errors, 0, "{line}");
        assert!(
            lat[lat.len() - 1] < budget + Duration::from_millis(500),
            "a request blocked longer than the budget: {line}"
        );
    }
    std::fs::write(out_dir().join("load.txt"), &report).unwrap();
}
