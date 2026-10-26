//! Test 5 of the brief: rate limiting refuses, never delays. A client under its limit has
//! the same p99 latency with and without over-limit traffic from another client, and the
//! over-limit requests are refused at once (`rate_limited`, `Retry-After`).

mod common;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;

const A: &str = "mesh-a.example";
const GOOD: &str = "dm_1111111111111111111111111111111111111111111111111111111111111111";

async fn good_client(a: &Node, n: usize) -> Vec<Duration> {
    let mut lat = Vec::new();
    let auth = format!("Bearer {GOOD}");
    for k in 0..n {
        let t = Instant::now();
        let r = a
            .get_with(&format!("/cell/{A}/rate?ccy=USD&date=2026-03-0{}", 1 + k % 2), &[("authorization", &auth)])
            .await;
        lat.push(t.elapsed());
        assert_eq!(r.status, 200, "{}", r.text());
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    lat.sort();
    lat
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refuse_never_delay() {
    let a = Node::start(A, |c| {
        c.trust_proxy = true;
        c.api_keys.insert_hash(
            &datumesh_core::sha256_hex(GOOD.as_bytes()),
            datumesh_http::apikeys::Principal { id: "good".into(), per_minute: Some(1_000_000) },
        );
    })
    .await;
    seed(&a, 10);
    let _ = good_client(&a, 50).await; // warm
    let base = good_client(&a, 400).await;

    // Over-limit traffic: one IP far above its 60 a minute (burst 120), from 8 tasks pacing
    // about 1,000 requests a second together, for as long as the good client runs.
    let stop = Arc::new(AtomicBool::new(false));
    let (sent, refused, slow) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let refusals = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut flood = Vec::new();
    for _ in 0..8 {
        let (http, url, stop, sent, refused, slow, refusals) = (
            a.http.clone(),
            format!("{}/cell/{A}/rate?ccy=GBP&date=2026-03-01", a.base),
            stop.clone(),
            sent.clone(),
            refused.clone(),
            slow.clone(),
            refusals.clone(),
        );
        flood.push(tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(8));
            while !stop.load(Ordering::Relaxed) {
                tick.tick().await;
                let t = Instant::now();
                let r = http.get(&url).header("x-forwarded-for", "198.51.100.7").send().await.unwrap();
                let d = t.elapsed();
                sent.fetch_add(1, Ordering::Relaxed);
                if r.status().as_u16() == 429 {
                    assert!(r.headers().get("retry-after").is_some());
                    refused.fetch_add(1, Ordering::Relaxed);
                    refusals.lock().unwrap().push(d);
                    if d > Duration::from_millis(100) {
                        slow.fetch_add(1, Ordering::Relaxed);
                    }
                }
                let _ = r.bytes().await;
            }
        }));
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let under = good_client(&a, 400).await;
    stop.store(true, Ordering::Relaxed);
    for f in flood {
        f.await.unwrap();
    }
    let mut rf = refusals.lock().unwrap().clone();
    rf.sort();
    let (b99, u99) = (pct(&base, 0.99), pct(&under, 0.99));
    let out = format!(
        "rate limits: the client under its limit p50/p99 {:?}/{b99:?} alone, {:?}/{u99:?} with over-limit traffic; that traffic: {} requests, {} refused (429) with refusal p50/p99 {:?}/{:?}, {} slower than 100 ms\n",
        pct(&base, 0.5),
        pct(&under, 0.5),
        sent.load(Ordering::Relaxed),
        refused.load(Ordering::Relaxed),
        pct(&rf, 0.5),
        pct(&rf, 0.99),
        slow.load(Ordering::Relaxed)
    );
    std::fs::write(out_dir().join("rate-limit.txt"), &out).unwrap();
    eprint!("{out}");
    assert!(refused.load(Ordering::Relaxed) > sent.load(Ordering::Relaxed) / 2, "{out}");
    assert!(u99 <= b99 * 2 + Duration::from_millis(5), "p99 changed under over-limit traffic: {out}");
    assert!(pct(&rf, 0.99) < Duration::from_millis(50), "refusals are immediate: {out}");
}
