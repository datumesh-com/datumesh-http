//! Test 6 of the brief: immutable responses carry `Cache-Control: public,
//! max-age=31536000, immutable`; answers about now a short max-age and a `Link:
//! rel=canonical` to the as-of form; receipts are gzipped on request; and a cache in front
//! (nginx in Docker, `tools/cache_test.sh`) serves repeat gets without touching the origin,
//! asserted by the origin's request counter.

mod common;

use std::time::Duration;

use common::*;
use datumesh_core::json::Value;

const A: &str = "mesh-a.example";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn headers() {
    let a = Node::start(A, |c| policy(c, ALICE)).await;
    seed(&a, 10);
    let r = a.get(&format!("/cell/{A}/rate?ccy=USD&date=2026-03-01&asof={T2}")).await;
    assert_eq!(r.header("cache-control").as_deref(), Some(IMMUTABLE));
    assert!(r.header("link").is_none());
    let r = a.get(&format!("/cell/{A}/rate?ccy=USD&date=2026-03-01")).await;
    assert_eq!(r.header("cache-control").as_deref(), Some("public, max-age=10"));
    let link = r.header("link").unwrap();
    let target = link.strip_prefix('<').and_then(|l| l.split_once(">; rel=\"canonical\"")).unwrap().0.to_string();
    assert!(target.starts_with(&format!("https://{A}/cell/{A}/rate?ccy=USD&date=2026-03-01&asof=")), "{link}");
    let canon = a.get(target.strip_prefix(&format!("https://{A}")).unwrap()).await;
    assert_eq!(canon.json().get("receipt"), r.json().get("receipt"), "the as-of form gives the same receipt");
    assert_eq!(canon.header("cache-control").as_deref(), Some(IMMUTABLE));
    let hash =
        datumesh_core::Receipt::from_json(&r.json().get("receipt").unwrap().to_jcs()).unwrap().hash().to_string();
    for path in [
        format!("/receipt/{hash}"),
        format!("/descriptor/{}", bl(|| a.store.resolve_name(&format!("{A}/rate"), T1).unwrap().unwrap())),
    ] {
        let r = a.get(&path).await;
        assert_eq!(r.header("cache-control").as_deref(), Some(IMMUTABLE), "{path}");
    }
    // gzip when asked (the receipt endpoints are gzipped).
    let raw = a.http.get(format!("{}/receipt/{hash}", a.base)).header("accept-encoding", "gzip").send().await.unwrap();
    assert_eq!(raw.headers().get("content-encoding").and_then(|v| v.to_str().ok()), Some("gzip"));
    // Never cached: errors, jobs, refusals.
    let r = a.get(&format!("/cell/{A}/secret?ccy=USD")).await;
    assert_eq!(r.header("cache-control").as_deref(), Some("private, no-store"));
    let r = a.get("/jobs/0123456789abcdef0123456789abcdef").await;
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
}

/// With `DATUMESH_HTTP_CACHE_URL` (the cache) and `DATUMESH_HTTP_ORIGIN_LISTEN` (where the
/// cache expects the origin), as `tools/cache_test.sh` sets them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cache_in_front() {
    let (Ok(cache), Ok(listen)) =
        (std::env::var("DATUMESH_HTTP_CACHE_URL"), std::env::var("DATUMESH_HTTP_ORIGIN_LISTEN"))
    else {
        eprintln!("cache_in_front: skipped (run tools/cache_test.sh)");
        return;
    };
    let a = Node::start_with(Setup { listen, ..Setup::new(A) }, |c| policy(c, ALICE), |b| b).await;
    seed(&a, 10);
    let http = client();
    let get = |path: String| {
        let http = http.clone();
        let url = format!("{cache}{path}");
        async move {
            let r = http.get(&url).send().await.unwrap();
            let status = r.status().as_u16();
            let x = r.headers().get("x-cache-status").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            let body = r.bytes().await.unwrap().to_vec();
            (status, x, body)
        }
    };
    let mut lines = String::new();
    for path in [
        format!("/cell/{A}/rate?ccy=USD&date=2026-03-01&asof={T2}"),
        format!("/cell/{A}/double?ccy=USD&date=2026-03-01&asof={T2}"),
        format!("/cell/{A}/rate?ccy=GBP&date=2026-03-01"),
    ] {
        let before = a.mesh.requests_of("/cell/{cell}");
        let (s0, x0, b0) = get(path.clone()).await;
        assert_eq!(s0, 200);
        let mut statuses = vec![x0];
        for _ in 0..9 {
            let (s, x, b) = get(path.clone()).await;
            assert_eq!((s, &b), (200, &b0));
            statuses.push(x);
        }
        let origin = a.mesh.requests_of("/cell/{cell}") - before;
        assert_eq!(origin, 1, "{path}: the origin saw {origin} of 10 requests ({statuses:?})");
        assert!(statuses[1..].iter().all(|x| x == "HIT"), "{statuses:?}");
        lines.push_str(&format!("cache: {path}: 10 gets, {origin} at the origin, {statuses:?}\n"));
    }
    let rec =
        datumesh_core::json::parse(&get(format!("/cell/{A}/rate?ccy=USD&date=2026-03-01&asof={T2}")).await.2).unwrap();
    let hash = datumesh_core::Receipt::from_json(&rec.get("receipt").unwrap().to_jcs()).unwrap().hash().to_string();
    let before = a.mesh.requests_of("/receipt/{hash}");
    for _ in 0..5 {
        assert_eq!(get(format!("/receipt/{hash}")).await.0, 200);
    }
    assert_eq!(a.mesh.requests_of("/receipt/{hash}") - before, 1);
    // A refusal is not cached: every request reaches the origin.
    let before = a.mesh.requests_of("/cell/{cell}");
    for _ in 0..3 {
        let (s, _, b) = get(format!("/cell/{A}/secret?ccy=USD")).await;
        assert_eq!(s, 403);
        assert_eq!(datumesh_core::json::parse(&b).unwrap().get("outcome").and_then(Value::as_str), Some("unlicensed"));
    }
    assert_eq!(a.mesh.requests_of("/cell/{cell}") - before, 3);
    std::fs::write(out_dir().join("cache.txt"), &lines).unwrap();
    eprint!("{lines}");
    let _ = Duration::ZERO;
}
