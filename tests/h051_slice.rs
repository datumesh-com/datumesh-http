//! http 0.5.1, CONTRACT-0.5.1 H1: `/slice`, `/x/interval` and the series period pages read one page at a time.
//! Written first; FAIL until then.
//!
//! The fixture is the contract's: a two-argument slice of 10,000 cells (100 currencies by 100 days). The row probe of
//! `common/h0410.rs` counts the rows of `cells`, `cell_args`, `versions` and `computed_versions` a request visits. A page
//! of `limit` members read through an index visits at most `limit + 1` members' rows; a member is a handful of rows
//! (`ROWS_PER_MEMBER` is a generous 20). Today every request sorts the whole slice (see EXPLAIN-A17-slice.txt) and visits
//! 10,000 members' rows and more.
//!
//! * `/slice`: the first page, a later page and a slice with one fixed argument read at most `limit + 1` members; the pages
//!   across `next` are complete (every member once) and in slice key order; `next` is still the last member's key (hex of the
//!   JCS bytes of the typed key scalars, SPEC 5.6).
//! * `/x/interval` and a series period page, with `DATUMESH_SLICE_MAX=100` (their page size is the cap): at most 101
//!   members read; the answer holds at most 100 and says it is cut. Which 100 members of a cut interval it shows is not
//!   pinned (the contract does not say); an interval inside the cap is whole and ordered by date then key.
//!
//! Not pinned here (the contract names a time, not a test): "cold first page under 200 ms on the restored copy" is the
//! rehearsal's measure. The dropped member-list cache is not testable from outside either.

mod common;
#[path = "common/h0410.rs"]
mod h0410;
#[path = "common/h051.rs"]
mod h051;

use h051::*;
use serde_json::Value;

const SLICE_ALL: &str = "(currency:%20*,%20date:%20*)";

fn key_of(m: &Value) -> (String, String) {
    let a = &m["receipt"]["cell"]["args"];
    (a["currency"]["value"].as_str().unwrap().to_string(), a["date"]["value"].as_str().unwrap().to_string())
}

/// A page: its members' keys, and `next`, as a path on the test server.
struct Page {
    keys: Vec<(String, String)>,
    next: Option<String>,
    next_after: Option<String>,
}

async fn page(w: &h0410::W, path: &str) -> Page {
    let r = w.get(path).await;
    assert_eq!(r.status, 200, "{path}: {}", r.text());
    let v = r.json();
    assert_eq!(v["outcome"], "ok", "a page envelope (5.6): {v}");
    let keys = v["members"].as_array().unwrap_or_else(|| panic!("no members: {v}")).iter().map(key_of).collect();
    let next = v["next"].as_str().map(|n| n.replacen(&format!("https://{}", common::lite::HOST), "", 1));
    let next_after = v["next"].as_str().and_then(|n| n.split("after=").nth(1)).map(|a| a.split('&').next().unwrap().to_string());
    Page { keys, next, next_after }
}

fn unhex(h: &str) -> String {
    let b: Vec<u8> = (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect();
    String::from_utf8(b).unwrap()
}

fn expected_order() -> Vec<(String, String)> {
    // the slice key order of SPEC 5.6 and 15.13: the open arguments in ascending name order (currency, date), each by its type
    let mut v = Vec::new();
    for c in 0..CURRENCIES {
        for d in 0..DAYS {
            v.push((currency(c), day(d)));
        }
    }
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h1_the_pages_of_a_slice_of_ten_thousand_cells_are_complete_and_in_order() {
    let fx = h0410::Fx::pg("h051-slice-walk");
    publish_fx_world(&fx);
    let w = fx.serve(cfg(&[], &[])).await;
    let mut all = Vec::new();
    let mut path = format!("/slice/{FXR}{SLICE_ALL}?limit=1000");
    let mut pages = 0;
    loop {
        let p = page(&w, &path).await;
        assert!(!p.keys.is_empty() && p.keys.len() <= 1000, "page {pages}: {} members", p.keys.len());
        // `next` is the last member's key: hex of the JCS of its typed scalars, open arguments in name order
        if let (Some(after), Some(last)) = (&p.next_after, p.keys.last()) {
            let key: Value = serde_json::from_str(&unhex(after)).unwrap_or_else(|e| panic!("after is not hex of JSON: {e}"));
            let got: Vec<(&str, &str)> = key
                .as_array()
                .unwrap_or_else(|| panic!("{key}"))
                .iter()
                .map(|s| (s["type"].as_str().unwrap(), s["value"].as_str().unwrap()))
                .collect();
            assert_eq!(got, vec![("string", last.0.as_str()), ("date", last.1.as_str())], "next is the last member's key");
        }
        all.extend(p.keys);
        pages += 1;
        match p.next {
            Some(n) => path = n,
            None => break,
        }
    }
    assert_eq!(pages, 10, "10,000 members in pages of 1,000");
    assert_eq!(all.len(), CURRENCIES * DAYS);
    assert!(all == expected_order(), "every member once, in slice key order");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h1_a_page_reads_at_most_limit_plus_one_members_whichever_page_it_is() {
    let fx = h0410::Fx::pg("h051-slice-rows");
    publish_fx_world(&fx);
    fx.probe().arm();
    let w = fx.serve(cfg(&[], &[])).await;
    let probe = w.fx.probe();
    let limit = 100u64;
    let bound = ROWS_PER_MEMBER * (limit + 1);

    // the first page, cold
    let b0 = probe.reads(h0410::Group::Cells);
    let first = page(&w, &format!("/slice/{FXR}{SLICE_ALL}?limit={limit}")).await;
    let b1 = probe.reads(h0410::Group::Cells);
    assert_eq!(first.keys.len() as u64, limit);
    assert!(b1 - b0 > 0, "the probe counted nothing: it is not armed");
    assert!(b1 - b0 <= bound, "the first page of {limit} visited {} rows of cells; at most {bound} (limit + 1 members)", b1 - b0);

    // a page in the middle, by `next`
    let mid = page(&w, first.next.as_deref().expect("a next link")).await;
    let b2 = probe.reads(h0410::Group::Cells);
    assert_eq!(mid.keys.len() as u64, limit);
    assert_eq!(mid.keys[0], expected_order()[limit as usize]);
    assert!(b2 - b1 <= bound, "the second page visited {} rows; at most {bound}", b2 - b1);

    // a page near the end: the cursor of the 9,900th member
    let tail = page(&w, &format!("/slice/{FXR}{SLICE_ALL}?limit={limit}&after={}", cursor_hex(&currency(98), &day(99)))).await;
    let b3 = probe.reads(h0410::Group::Cells);
    assert_eq!(tail.keys.len() as u64, limit, "C099's 100 days follow C098's last");
    assert_eq!(tail.keys[0], (currency(99), day(0)));
    assert!(tail.next.is_none(), "the last page has no next");
    assert!(b3 - b2 <= bound, "the last page visited {} rows; at most {bound}", b3 - b2);

    // the default page is 1,000 members
    let dflt = page(&w, &format!("/slice/{FXR}{SLICE_ALL}")).await;
    let b4 = probe.reads(h0410::Group::Cells);
    assert_eq!(dflt.keys.len(), 1000);
    assert!(b4 - b3 <= ROWS_PER_MEMBER * 1001, "the default page visited {} rows; at most {}", b4 - b3, ROWS_PER_MEMBER * 1001);
}

fn cursor_hex(cur: &str, d: &str) -> String {
    let j = serde_json::json!([{"type": "string", "value": cur}, {"type": "date", "value": d}]).to_string();
    j.bytes().map(|b| format!("{b:02x}")).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h1_a_slice_with_one_fixed_argument_is_one_complete_page_of_its_members() {
    let fx = h0410::Fx::pg("h051-slice-fixed");
    publish_fx_world(&fx);
    fx.probe().arm();
    let w = fx.serve(cfg(&[], &[])).await;
    let b0 = w.fx.probe().reads(h0410::Group::Cells);
    let p = page(&w, &format!("/slice/{FXR}(currency:%20%22C042%22,%20date:%20*)")).await;
    let b1 = w.fx.probe().reads(h0410::Group::Cells);
    let want: Vec<(String, String)> = (0..DAYS).map(|d| (currency(42), day(d))).collect();
    assert!(p.keys == want, "the 100 days of C042, in date order: {} members", p.keys.len());
    assert!(p.next.is_none());
    assert!(
        b1 - b0 <= ROWS_PER_MEMBER * 101,
        "a slice of 100 members in a function of 10,000 cells visited {} rows; at most {}",
        b1 - b0,
        ROWS_PER_MEMBER * 101
    );
    // a page of it
    let p = page(&w, &format!("/slice/{FXR}(currency:%20%22C042%22,%20date:%20*)?limit=30")).await;
    assert_eq!(p.keys.len(), 30);
    let rest = page(&w, p.next.as_deref().unwrap()).await;
    assert_eq!(rest.keys[0], (currency(42), day(30)));
}

// ---- /x/interval and the period pages --------------------------------------------------------------------------------

async fn series_ready(w: &h0410::W) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let r = w.get(&format!("/series/{FXR}/")).await;
        if r.status != 503 || std::time::Instant::now() > end {
            assert_eq!(r.status, 200, "the series overview: {}", r.text());
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h1_an_interval_reads_at_most_the_cap_plus_one_members() {
    let fx = h0410::Fx::pg("h051-interval");
    publish_fx_world(&fx);
    let path = series_file(&fx);
    fx.probe().arm();
    let w = fx.serve(cfg(&["series"], &[("DATUMESH_SERIES_FILE", &path), ("DATUMESH_SLICE_MAX", "100")])).await;
    series_ready(&w).await;
    let probe = w.fx.probe();
    let bound = ROWS_PER_MEMBER * 101;

    // every key, the whole of the data: cut at the cap of 100
    let b0 = probe.reads(h0410::Group::Cells);
    let r = w.get(&format!("/x/interval/{FXR}?currency=*&from=2026-01-01&to=2026-04-10")).await;
    let b1 = probe.reads(h0410::Group::Cells);
    assert_eq!(r.status, 200, "{}", r.text());
    let v = r.json();
    assert_eq!(v["members"].as_array().unwrap().len(), 100, "cut at DATUMESH_SLICE_MAX: {}", v["count"]);
    assert_eq!(v["truncated"], true, "the answer says it is cut");
    assert!(b1 - b0 > 0, "the probe counted nothing");
    assert!(b1 - b0 <= bound, "the interval of 10,000 cells visited {} rows; at most {bound} (cap + 1 members)", b1 - b0);

    // one key, a month: inside the cap, whole, by date
    let r = w.get(&format!("/x/interval/{FXR}?currency=C007&from=2026-03-01&to=2026-03-31")).await;
    let b2 = probe.reads(h0410::Group::Cells);
    assert_eq!(r.status, 200, "{}", r.text());
    let v = r.json();
    let dates: Vec<&str> = v["members"].as_array().unwrap().iter().map(|m| m["keys"]["date"].as_str().unwrap()).collect();
    let want: Vec<String> = (59..90).map(day).collect();
    assert_eq!(dates, want.iter().map(String::as_str).collect::<Vec<_>>(), "the 31 days of March, whole and in order");
    assert!(v.get("truncated").is_none_or(|t| t != &Value::Bool(true)), "{v}");
    assert!(b2 - b1 <= bound, "a month of one currency visited {} rows; at most {bound}", b2 - b1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h1_a_period_page_reads_at_most_the_cap_plus_one_members() {
    let fx = h0410::Fx::pg("h051-period");
    publish_fx_world(&fx);
    let path = series_file(&fx);
    fx.probe().arm();
    let w = fx.serve(cfg(&["series"], &[("DATUMESH_SERIES_FILE", &path), ("DATUMESH_SLICE_MAX", "100")])).await;
    series_ready(&w).await;
    let probe = w.fx.probe();
    let b0 = probe.reads(h0410::Group::Cells);
    // March: 100 currencies by 31 days is 3,100 members, the cap 100
    let r = w.get(&format!("/series/{FXR}/2026-03.csv")).await;
    let b1 = probe.reads(h0410::Group::Cells);
    assert_eq!(r.status, 200, "{}", r.text());
    let rows = r.text().lines().filter(|l| !l.trim().is_empty()).count().saturating_sub(1);
    assert!((1..=100).contains(&rows), "the page shows at most the cap's worth of members: {rows} date rows");
    assert!(b1 - b0 > 0, "the probe counted nothing");
    assert!(
        b1 - b0 <= ROWS_PER_MEMBER * 101,
        "the period page of a month of 3,100 members visited {} rows; at most {}",
        b1 - b0,
        ROWS_PER_MEMBER * 101
    );
    // the HTML page likewise
    let h = w.get_h(&format!("/series/{FXR}/2026-03"), &[("accept", "text/html")]).await;
    let b2 = probe.reads(h0410::Group::Cells);
    assert_eq!(h.status, 200);
    assert!(b2 - b1 <= ROWS_PER_MEMBER * 101, "the HTML page visited {} rows", b2 - b1);
}
