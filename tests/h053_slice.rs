//! http 0.5.3, CONTRACT-protocol-part1 H2 (D6): CSV and Arrow slices obey `limit` and page with a `Link` header.
//! Written first; FAIL until then.
//!
//! * `?format=csv&limit=N` returns at most N rows (Arrow likewise);
//! * when more follow, the response carries `Link: <the same URL with after=<cursor>>; rel="next"`, the cursor being
//!   the JSON `next`'s; following the Links walks every member once in key order; the last page has no Link;
//! * without `limit` the answer is the whole slice up to `slice_max`, as in 0.5.1.
//!
//! Fixture: `A/obs(i)` for i in 0..12, value `i.5`, key order numeric (so 2 comes before 10).

mod common;
#[path = "common/h053.rs"]
mod h053;

use arrow_array::{Array, StringArray};
use h053::*;

const SIG: &str = r#"{"args":{"i":{"type":"number","unit":"EUR","valid":false}},"order":["i"],"returns":{"type":"number","unit":"EUR"}}"#;
const N: usize = 12;

async fn world(slice_max: Option<usize>) -> Node {
    let a = Node::start(A, move |c| {
        c.rate.enabled = false;
        if let Some(m) = slice_max {
            c.slice_max = m;
        }
    })
    .await;
    a.stored(&format!("{A}/obs"), SIG);
    let node = &a;
    tokio::task::block_in_place(|| load_obs(node, "obs", N, 3, T1));
    a
}

fn csv_rows(r: &Resp) -> (Vec<String>, Vec<String>) {
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(r.header("content-type").unwrap_or_default().starts_with("text/csv"), "{:?}", r.header("content-type"));
    let t = r.text();
    let mut lines: Vec<String> = t.split("\r\n").filter(|l| !l.is_empty()).map(str::to_string).collect();
    assert_eq!(lines[0], "i,value,tx", "{t}");
    lines.remove(0);
    let keys = lines.iter().map(|l| l.split(',').next().unwrap().to_string()).collect();
    (keys, lines)
}

fn arrow_keys(r: &Resp) -> Vec<String> {
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.header("content-type").as_deref(), Some("application/vnd.apache.arrow.stream"));
    let reader = arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(&r.body), None).unwrap();
    let mut out = Vec::new();
    for b in reader {
        let b = b.unwrap();
        let i = b.column_by_name("i").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        for k in 0..i.len() {
            out.push(i.value(k).to_string());
        }
    }
    out
}

/// The target of the `rel="next"` link of a response, as a path on the test node.
fn next_link(r: &Resp) -> Option<String> {
    let l = r.header("link")?;
    let (target, rest) = l.strip_prefix('<')?.split_once('>')?;
    assert!(rest.contains("rel=\"next\""), "Link: {l}");
    Some(target.replacen(&format!("https://{A}"), "", 1))
}

fn after_of(url: &str) -> String {
    url.split("after=").nth(1).unwrap_or_else(|| panic!("no after in {url}")).split('&').next().unwrap().to_string()
}

fn want(from: usize, to: usize) -> Vec<String> {
    (from..to).map(|i| i.to_string()).collect()
}

const PIN: &str = "asof=2026-06-01T00:00:00.000000Z";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h2_csv_limit_three_gives_three_rows_and_a_link_with_the_json_cursor() {
    let a = world(None).await;
    let r = a.get(&format!("/slice/{A}/obs?i=*&{PIN}&format=csv&limit=3")).await;
    let (keys, _) = csv_rows(&r);
    assert_eq!(keys, want(0, 3), "at most `limit` rows, in key order");
    let link = next_link(&r).unwrap_or_else(|| panic!("no Link header: {:?}", r.headers));
    assert!(link.contains("format=csv") && link.contains("limit=3"), "the same URL: {link}");
    // the cursor is the JSON next's
    let j = jv(&a.get(&format!("/slice/{A}/obs?i=*&{PIN}&limit=3")).await);
    let json_next = j["next"].as_str().unwrap();
    assert_eq!(after_of(&link), after_of(json_next));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h2_following_the_links_walks_every_member_once_in_key_order_and_the_last_page_has_none() {
    let a = world(None).await;
    for limit in [3usize, 5, 1] {
        let mut path = format!("/slice/{A}/obs?i=*&{PIN}&format=csv&limit={limit}");
        let (mut all, mut pages) = (Vec::new(), 0);
        loop {
            let r = a.get(&path).await;
            let (keys, _) = csv_rows(&r);
            assert!(!keys.is_empty() && keys.len() <= limit, "limit {limit}, page {pages}: {keys:?}");
            let link = next_link(&r);
            if all.len() + keys.len() < N {
                assert!(link.is_some(), "limit {limit}: page {pages} of {} rows has more after it but no Link", keys.len());
            } else {
                assert!(link.is_none(), "limit {limit}: the last page carries a Link");
            }
            all.extend(keys);
            pages += 1;
            match link {
                Some(l) => path = l,
                None => break,
            }
        }
        assert_eq!(all, want(0, N), "limit {limit}: every member once, in key order");
        assert_eq!(pages, N.div_ceil(limit));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h2_a_limit_that_holds_the_whole_slice_has_no_link() {
    let a = world(None).await;
    let r = a.get(&format!("/slice/{A}/obs?i=*&{PIN}&format=csv&limit={N}")).await;
    assert_eq!(csv_rows(&r).0, want(0, N));
    assert!(r.header("link").is_none(), "{:?}", r.header("link"));
    let r = a.get(&format!("/slice/{A}/obs?i=*&{PIN}&format=csv&limit={}", N - 1)).await;
    assert_eq!(csv_rows(&r).0, want(0, N - 1));
    let r = a.get(&next_link(&r).expect("a Link: one member follows")).await;
    assert_eq!(csv_rows(&r).0, want(N - 1, N));
    assert!(r.header("link").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h2_without_a_limit_the_csv_is_the_whole_slice_up_to_slice_max() {
    let a = world(None).await;
    let r = a.get(&format!("/slice/{A}/obs?i=*&{PIN}&format=csv")).await;
    assert_eq!(csv_rows(&r).0, want(0, N));
    assert!(r.header("link").is_none());
    let b = world(Some(5)).await;
    let r = b.get(&format!("/slice/{A}/obs?i=*&{PIN}&format=csv")).await;
    assert_eq!(csv_rows(&r).0, want(0, 5), "cut at slice_max, as in 0.5.1");
    // a limit above slice_max does not lift the bound
    let r = b.get(&format!("/slice/{A}/obs?i=*&{PIN}&format=csv&limit=10")).await;
    assert!(csv_rows(&r).0.len() <= 5);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h2_arrow_limit_and_links_as_csv() {
    let a = world(None).await;
    let arrow = [("accept", "application/vnd.apache.arrow.stream")];
    let r = a.get_with(&format!("/slice/{A}/obs?i=*&{PIN}&limit=3"), &arrow).await;
    assert_eq!(arrow_keys(&r), want(0, 3));
    let link = next_link(&r).unwrap_or_else(|| panic!("no Link header on the Arrow page: {:?}", r.headers));
    let j = jv(&a.get(&format!("/slice/{A}/obs?i=*&{PIN}&limit=3")).await);
    assert_eq!(after_of(&link), after_of(j["next"].as_str().unwrap()), "the cursor of the JSON next");

    // by `format=arrow` too, walked to the end
    let mut path = format!("/slice/{A}/obs?i=*&{PIN}&format=arrow&limit=5");
    let (mut all, mut pages) = (Vec::new(), 0);
    loop {
        let r = a.get(&path).await;
        let keys = arrow_keys(&r);
        assert!(keys.len() <= 5);
        all.extend(keys);
        pages += 1;
        match next_link(&r) {
            Some(l) => path = l,
            None => break,
        }
    }
    assert_eq!((all, pages), (want(0, N), 3), "every member once, in key order; no Link on the last page");

    // without a limit: the whole slice, no Link
    let r = a.get_with(&format!("/slice/{A}/obs?i=*&{PIN}"), &arrow).await;
    assert_eq!(arrow_keys(&r), want(0, N));
    assert!(r.header("link").is_none());
}
