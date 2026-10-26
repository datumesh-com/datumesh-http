//! Test 9 of the brief: a 100,000-member slice streams as Arrow in under 2 s warm; the JSON
//! form is paginated by key.

mod common;

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use arrow_array::{Array, StringArray};
use common::*;
use datumesh_core::json::Value;

const A: &str = "mesh-a.example";
const SIG: &str = r#"{"args":{"i":{"type":"number","unit":"EUR","valid":false}},"order":["i"],"returns":{"type":"number","unit":"EUR"}}"#;
const N: usize = 100_000;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hundred_thousand_members() {
    let a = Node::start(A, |c| c.rate.enabled = false).await;
    fast_commits(&a);
    a.stored(&format!("{A}/obs"), SIG);
    let t = Instant::now();
    let node = &a;
    tokio::task::block_in_place(|| load_obs(node, "obs", N, 12, T1));
    let loaded = t.elapsed();
    let at = T2;
    let path = format!("/slice/{A}/obs?i=*&asof={at}");
    let arrow = [("accept", "application/vnd.apache.arrow.stream")];

    let t = Instant::now();
    let cold = a.get_with(&path, &arrow).await;
    let cold_t = t.elapsed();
    assert_eq!(cold.status, 200, "{}", cold.text());
    assert_eq!(cold.header("content-type").as_deref(), Some("application/vnd.apache.arrow.stream"));
    let mut warm = Vec::new();
    let mut body = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let r = a.get_with(&path, &arrow).await;
        warm.push(t.elapsed());
        body = r.body;
    }
    warm.sort();
    assert_eq!(body, cold.body, "the same stream");
    // The stream: 100,000 rows, keys ascending, values as stored.
    let reader = arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(&body), None).unwrap();
    let schema = reader.schema();
    assert_eq!(schema.metadata().get("asof").map(String::as_str), Some(at));
    let mut rows = 0;
    let mut last: Option<i64> = None;
    for batch in reader {
        let batch = batch.unwrap();
        let i = batch.column_by_name("i").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let v = batch.column_by_name("value").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        for k in 0..batch.num_rows() {
            let x: i64 = i.value(k).parse().unwrap();
            assert!(last.is_none_or(|l| x > l), "key order");
            assert_eq!(v.value(k), format!("{x}.5"));
            last = Some(x);
        }
        rows += i.len();
    }
    assert_eq!(rows, N);
    let warm_p50 = pct(&warm, 0.5);
    // Timings hold for an optimised build (`cargo test --release`).
    if !cfg!(debug_assertions) {
        assert!(warm[warm.len() - 1] < Duration::from_secs(2), "warm Arrow stream took {:?}", warm);
    }

    // JSON: pages of 1,000 by key, every member once, in order, the asof fixed by the first.
    let t = Instant::now();
    let mut next = Some(format!("{}/slice/{A}/obs?i=*&limit=1000", a.base));
    let (mut pages, mut seen, mut prev) = (0, BTreeSet::new(), -1i64);
    let mut asof = None;
    while let Some(url) = next {
        let r = a.http.get(&url).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 200);
        let v = datumesh_core::json::parse(&r.bytes().await.unwrap()).unwrap();
        let page_asof = v.get("asof").and_then(Value::as_str).unwrap().to_string();
        assert_eq!(asof.get_or_insert(page_asof.clone()), &page_asof);
        let Some(Value::Array(ms)) = v.get("members") else { panic!("members") };
        for m in ms {
            let x: i64 = m
                .get("args")
                .and_then(|a| a.get("i"))
                .and_then(|i| i.get("value"))
                .and_then(Value::as_str)
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(x, prev + 1, "contiguous by key");
            prev = x;
            seen.insert(x);
        }
        pages += 1;
        next = v.get("next").and_then(Value::as_str).map(|n| n.replacen(&format!("https://{A}"), &a.base, 1));
    }
    let json_t = t.elapsed();
    assert_eq!((pages, seen.len()), (100, N));
    let out = format!(
        "slice: {N} members loaded in {loaded:?}; Arrow stream {} bytes: cold {cold_t:?}, warm p50 {warm_p50:?} (max {:?}); JSON: {pages} pages of 1,000 in {json_t:?}\n",
        body.len(),
        warm[warm.len() - 1]
    );
    std::fs::write(out_dir().join("slice.txt"), &out).unwrap();
    eprint!("{out}");
}
