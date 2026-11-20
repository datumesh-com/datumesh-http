//! Build 2 Part 6, G5 follow-up (http 0.4.8): the sitemap and the series index never compute on a request.
//! Found on production: a cold sitemap took 44.6 s. These tests are written first and FAIL until the
//! overview is built in the background.
//!
//! # What these tests fix (the implementer follows)
//!
//! * The overview of every listed series (its periods and latest as-of, what the index and the sitemap
//!   need) is built on a thread of its own, at start and then every `DATUMESH_SERIES_REFRESH_SECS`
//!   (default 300), like the find index thread. A request only reads the last built overview.
//! * Before the first build has completed, `/series/sitemap.xml` (and its parts) and a series index answer
//!   `503` with `Retry-After` (whole seconds, 1 to 300) and `Cache-Control: no-store`: nothing is computed on
//!   the request. Month and year pages, and CSV, do not wait for the overview: they answer as they do from
//!   the store (their previous and next links may be missing until the first build is done).
//! * `GET /metrics` exposes `datumesh_series_overview_builds_total{by="background"}` and `{by="request"}`.
//!   The request counter is 0 for ever (a build on a request path would count there); a line that is absent
//!   counts as 0.
//! * `DATUMESH_SERIES_FIRST_BUILD_DELAY_SECS` (a test knob, default 0) holds the first background build back
//!   for that many seconds, so that "before the first build" can be observed without a race.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use std::time::{Duration, Instant};

use common::lite::*;
use common::pub031::{function_line, run, write};
use common::readpath::*;
use datumesh_http::Config;
use publish4::{get, Reply4};
use serde_json::{json, Value};

const BANK: &str = "mesh-test.datumesh.com/relay/bankofengland.co.uk/bank_rate";
const BOUND_CLOCK: &str = "2026-01-01T00:00:00.000000Z";
const SITEMAP: &str = "/series/sitemap.xml";

fn index() -> String {
    format!("/series/{BANK}/")
}

fn bank_sig() -> String {
    r#"{"args":{"date":{"type":"date","valid":true}},"order":["date"],"returns":{"type":"number","unit":"1"}}"#
        .to_string()
}

fn bank_cell(date: &str, value: &str, tx: &str) -> String {
    json!({"cell": {"fn": BANK, "args": {"date": {"type": "date", "value": date}}}, "licence": "OGL-UK-3.0",
        "summary": "Synthetic test data.", "value": {"type": "number", "value": value},
        "tx": tx, "asof": tx, "issued": tx,
        "source": {"name": "bankofengland.co.uk", "url": "https://bankofengland.co.uk/data", "retrieved": BOUND_CLOCK,
            "takeover": format!("https://{HOST}/takeover/bankofengland.co.uk")}})
    .to_string()
}

/// `days` daily cells from 2025-01-01 on, then the series file; `vars` are environment variables.
async fn serve(label: &str, days: usize, vars: &[(&str, &str)]) -> Served {
    let lite = Lite::new(label);
    let f0 = write(&lite, "functions.jsonl", &[function_line(BANK, &bank_sig(), None)]);
    run(&lite, &f0, BOUND_CLOCK).unwrap_or_else(|e| panic!("function refused: {e}"));
    let mut lines = Vec::new();
    let mut d = chrono_free_date(2024, 1, 1);
    for i in 0..days {
        lines.push(bank_cell(&d, &format!("{}.25", 1 + i % 7), "2026-03-01T12:00:00.000000Z"));
        d = next_day(&d);
    }
    lines.push(common::readpath::catalog_line(
        BANK,
        "function",
        "Bank rate",
        "The bank rate.",
        json!({"date": "the day"}),
        json!({}),
    ));
    let f = write(&lite, "fixture.jsonl", &lines);
    run(&lite, &f, NOW).unwrap_or_else(|e| panic!("fixture refused: {e}"));
    let p = lite.path("series.json");
    std::fs::write(
        &p,
        json!([{"function": BANK, "granularity": "month", "date_arg": "date", "unit": "percent", "kind": "relayed"}])
            .to_string(),
    )
    .unwrap();
    let mut env: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    env.push(("DATUMESH_SERIES_FILE".into(), p.to_string_lossy().into_owned()));
    // these tests poll: the budgets of the rate limiter are out of their way
    for (k, v) in [
        ("DATUMESH_RATE_PER_IP", "1000000"),
        ("DATUMESH_RATE_IP_BURST", "1000000"),
        ("DATUMESH_RATE_SERIES_PER_IP", "1000000"),
    ] {
        env.push((k.into(), v.into()));
    }
    let cfg = Config::from_vars(move |k| {
        if k == "DATUMESH_HOST" {
            return Some(HOST.to_string());
        }
        env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    })
    .unwrap();
    build_served(lite, Some(cfg), |_| {}).await
}

fn chrono_free_date(y: u32, m: u32, d: u32) -> String {
    format!("{y:04}-{m:02}-{d:02}")
}

fn next_day(d: &str) -> String {
    let (y, m, day): (u32, u32, u32) = (d[..4].parse().unwrap(), d[5..7].parse().unwrap(), d[8..].parse().unwrap());
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let len = match m {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    match (day < len, m < 12) {
        (true, _) => chrono_free_date(y, m, day + 1),
        (false, true) => chrono_free_date(y, m + 1, 1),
        (false, false) => chrono_free_date(y + 1, 1, 1),
    }
}

/// The count of `datumesh_series_overview_builds_total{by="<by>"}`; an absent line is 0.
async fn builds(m: &Served, by: &str) -> u64 {
    let t = get(m, "/metrics", &[]).await.text();
    let want = format!("datumesh_series_overview_builds_total{{by=\"{by}\"}}");
    t.lines().find_map(|l| l.strip_prefix(&want).map(|r| r.trim().parse::<u64>().unwrap())).unwrap_or(0)
}

/// Waits until `path` answers something but 503 (the first build is done); returns the answer.
async fn until_built(m: &Served, path: &str) -> Reply4 {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let r = get(m, path, &[]).await;
        if r.status != 503 || Instant::now() > end {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn timed(m: &Served, path: &str) -> (Reply4, Duration) {
    let t = Instant::now();
    let r = get(m, path, &[]).await;
    (r, t.elapsed())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn before_the_first_build_the_sitemap_and_the_index_answer_503() {
    let m = serve("sb-503", 120, &[("DATUMESH_SERIES_FIRST_BUILD_DELAY_SECS", "3600")]).await;
    for path in [SITEMAP.to_string(), index(), "/series/sitemap-1.xml".to_string()] {
        let (r, took) = timed(&m, &path).await;
        assert_eq!(r.status, 503, "{path}: {}", r.text());
        let ra: u64 = r.header("retry-after").unwrap_or_else(|| panic!("{path}: no Retry-After")).parse().unwrap();
        assert!((1..=300).contains(&ra), "{path}: {ra}");
        assert_eq!(r.header("cache-control").as_deref(), Some("no-store"), "{path}");
        assert!(took < Duration::from_secs(1), "{path}: answered at once, took {took:?}");
    }
    // nothing was built on those requests, and the delayed first build has not run
    assert_eq!(builds(&m, "request").await, 0);
    assert_eq!(builds(&m, "background").await, 0);
    // a page of the series does not wait for the overview, and builds nothing either
    let page = get(&m, &format!("/series/{BANK}/2024-03"), &[]).await;
    assert_eq!(page.status, 200, "{}", page.text());
    let csv = get(&m, &format!("/series/{BANK}/2024-03.csv"), &[]).await;
    assert_eq!(csv.status, 200, "{}", csv.text());
    assert_eq!(get(&m, &format!("/series/{BANK}/2023-03"), &[]).await.status, 404);
    assert_eq!(builds(&m, "request").await, 0);
    assert_eq!(builds(&m, "background").await, 0);
    // still 503 after all that
    assert_eq!(get(&m, SITEMAP, &[]).await.status, 503);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_the_build_requests_read_the_last_overview_and_compute_nothing() {
    // two years of daily cells: 730 stored cells
    let m = serve("sb-fast", 730, &[]).await;
    let sm = until_built(&m, SITEMAP).await;
    assert_eq!(sm.status, 200, "{}", sm.text());
    let x = sm.text();
    assert!(x.contains("http://www.sitemaps.org/schemas/sitemap/0.9"), "{x}");
    assert!(x.contains(&format!("/series/{BANK}/2024-01")) && x.contains(&format!("/series/{BANK}/2025-12")), "{x}");
    let ix = get(&m, &index(), &[]).await;
    assert_eq!(ix.status, 200, "{}", ix.text());
    assert!(ix.text().contains("2024-06"), "{}", ix.text());
    let built = builds(&m, "background").await;
    assert!(built >= 1, "the background build is counted: {built}");
    // every request after the build is quick and builds nothing
    for _ in 0..10 {
        for path in [SITEMAP.to_string(), index()] {
            let (r, took) = timed(&m, &path).await;
            assert_eq!(r.status, 200, "{path}: {}", r.text());
            assert!(took < Duration::from_secs(1), "{path}: {took:?}");
        }
    }
    assert_eq!(builds(&m, "request").await, 0, "no overview was built on a request");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_overview_is_rebuilt_on_a_schedule_never_on_a_request() {
    let m = serve("sb-refresh", 60, &[("DATUMESH_SERIES_REFRESH_SECS", "1")]).await;
    let sm = until_built(&m, SITEMAP).await;
    assert_eq!(sm.status, 200, "{}", sm.text());
    assert!(!sm.text().contains("2026-09"), "{}", sm.text());
    // a month with data appears in the store after the first build
    let f = write(&m.lite, "later.jsonl", &[bank_cell("2026-09-10", "9.5", "2026-09-10T12:00:00.000000Z")]);
    run(&m.lite, &f, NOW).unwrap_or_else(|e| panic!("later cell refused: {e}"));
    // a request right away reads the last overview: the new month is not in it yet (the schedule is 1 s,
    // so allow that it may already be) and, whichever it is, it builds nothing
    let _ = get(&m, SITEMAP, &[]).await;
    assert_eq!(builds(&m, "request").await, 0);
    // within a few seconds the scheduled build has picked it up
    let end = Instant::now() + Duration::from_secs(20);
    let mut seen = false;
    while Instant::now() < end {
        let r = get(&m, SITEMAP, &[]).await;
        assert_eq!(r.status, 200, "{}", r.text());
        if r.text().contains(&format!("/series/{BANK}/2026-09")) {
            seen = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(seen, "the scheduled build did not add 2026-09 to the sitemap in 20 s");
    let ix: Value = Value::String(get(&m, &index(), &[]).await.text());
    assert!(ix.as_str().unwrap().contains("2026-09"), "{ix}");
    assert!(builds(&m, "background").await >= 2, "built at start and again on the schedule");
    assert_eq!(builds(&m, "request").await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_a_series_file_there_is_nothing_to_build() {
    let lite = Lite::new("sb-none");
    let m = build_served(lite, None, |_| {}).await;
    assert_eq!(get(&m, SITEMAP, &[]).await.status, 404);
    assert_eq!(builds(&m, "request").await, 0);
}
