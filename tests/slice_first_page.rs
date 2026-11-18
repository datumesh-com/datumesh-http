//! CONTRACT-perf S-T2..4 (H4): the first page of a latest slice reuses the members of its
//! series when nothing changed. Runs on PostgreSQL through the shared `Node` harness, with a
//! settable mesh clock.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use datumesh_core::json::Value;
use datumesh_core::Receipt;
use datumesh_engine::Clock;

const A: &str = "mesh-a.example";
const SIG: &str = r#"{"args":{"i":{"type":"number","unit":"EUR","valid":false}},"order":["i"],"returns":{"type":"number","unit":"EUR"}}"#;
const T1: &str = "2026-01-01T00:00:00.000000Z";
const C0: &str = "2026-02-01T00:00:00.000000Z";
const H: i64 = 3_600_000_000;
const MIN: i64 = 60_000_000;

/// A mesh clock the test moves.
struct Settable(Mutex<String>);

impl Clock for Settable {
    fn now(&self) -> String {
        self.0.lock().unwrap().clone()
    }
}

fn set(c: &Settable, t: &str) {
    *c.0.lock().unwrap() = t.to_string();
}

/// A mesh on a settable clock with `obs(i)` for i in 0..count, value `<i>.5`, stored at T1.
async fn world(count: usize) -> (Node, Arc<Settable>) {
    let clock = Arc::new(Settable(Mutex::new(C0.to_string())));
    let c = clock.clone();
    let node = Node::start_with(Setup::new(A), |c| c.rate.enabled = false, move |b| b.clock(c)).await;
    node.stored(&format!("{A}/obs"), SIG);
    let nd = &node;
    tokio::task::block_in_place(|| load_obs(nd, "obs", count, 4, T1));
    (node, clock)
}

/// A page: (key, value, receipt hash) per member, and the `next` link.
struct Page {
    members: Vec<(i64, String, String)>,
    next: Option<String>,
}

impl Page {
    fn asof(&self) -> Option<String> {
        self.next.as_ref().map(|n| n.split("asof=").nth(1).unwrap().split('&').next().unwrap().to_string())
    }
}

async fn page(node: &Node, url: &str) -> Page {
    let r = node.http.get(url).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let v = datumesh_core::json::parse(&r.bytes().await.unwrap()).unwrap();
    assert_eq!(v.get("outcome").and_then(Value::as_str), Some("ok"));
    let Some(Value::Array(ms)) = v.get("members") else { panic!("members") };
    let members = ms
        .iter()
        .map(|m| {
            let r = Receipt::from_json(&m.get("receipt").unwrap().to_jcs()).unwrap();
            let k: i64 = r.cell().args["i"].scalar().unwrap().lexical().parse().unwrap();
            (k, r.value().lexical().to_string(), r.hash().to_string())
        })
        .collect();
    let next = v.get("next").and_then(Value::as_str).map(|n| n.replacen(&format!("https://{A}"), &node.base, 1));
    Page { members, next }
}

/// Every page from `first` on, following `next`.
async fn rest(node: &Node, first: &Page) -> Vec<(i64, String, String)> {
    let mut out = Vec::new();
    let mut next = first.next.clone();
    while let Some(u) = next {
        let p = page(node, &u).await;
        out.extend(p.members);
        next = p.next;
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_repeated_latest_first_page_reuses_the_members_and_pins_the_same_asof() {
    let (node, clock) = world(25).await;
    let url = format!("{}/slice/{A}/obs?i=*&limit=10", node.base);
    let p1 = page(&node, &url).await;
    assert_eq!(p1.members.len(), 10);
    assert_eq!(p1.asof().as_deref(), Some(C0), "the first page is answered as of now");
    set(&clock, &plus(C0, H));
    let p2 = page(&node, &url).await;
    assert_eq!(p2.members, p1.members);
    let (r1, r2) = (rest(&node, &p1).await, rest(&node, &p2).await);
    assert_eq!(r1, r2);
    // Every member once, in key order.
    let all: Vec<i64> = p1.members.iter().chain(&r1).map(|m| m.0).collect();
    assert_eq!(all, (0..25).collect::<Vec<_>>());
    // H4: the second first page is answered as of the first one's `at` (the clock has moved).
    assert_eq!(p2.asof(), p1.asof(), "the second first page pins the first one's asof");
    assert_eq!(p2.next, p1.next, "the next links are equal");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_write_after_a_first_page_shows_in_the_next_latest_first_page() {
    let (node, clock) = world(25).await;
    let url = format!("{}/slice/{A}/obs?i=*&limit=10", node.base);
    let name = format!("{A}/obs");
    let p1 = page(&node, &url).await;
    assert_eq!(p1.members[3], (3, "3.5".into(), p1.members[3].2.clone()));

    // A new version of member 3 (later tx and asof, another value).
    set(&clock, &plus(C0, H));
    let h = node.put_value(&name, &args(&[("i", n("3"))]), n("99.5"), &plus(C0, 30 * MIN));
    let p2 = page(&node, &url).await;
    assert_eq!(p2.members[3], (3, "99.5".into(), h));
    assert_eq!(p2.members.len(), 10);
    assert_eq!(p2.members[4], p1.members[4]);

    // A new cell matching the slice.
    set(&clock, &plus(C0, 2 * H));
    let h100 = node.put_value(&name, &args(&[("i", n("100"))]), n("100.5"), &plus(C0, 90 * MIN));
    let all = format!("{}/slice/{A}/obs?i=*&limit=1000", node.base);
    let p3 = page(&node, &all).await;
    assert_eq!(p3.next, None);
    assert_eq!(p3.members.len(), 26);
    assert_eq!(p3.members.last().unwrap(), &(100, "100.5".into(), h100));
    assert_eq!(p3.members[3].1, "99.5");
    // The paged form reaches it too.
    let q = page(&node, &url).await;
    assert_eq!(q.members[3].1, "99.5");
    let tail = rest(&node, &q).await;
    assert_eq!(tail.last().map(|m| m.0), Some(100));
    assert_eq!(q.members.len() + tail.len(), 26);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_first_page_with_an_explicit_asof_is_cached_by_that_asof() {
    let (node, clock) = world(25).await;
    let at = "2026-01-15T00:00:00.000000Z";
    let url = format!("{}/slice/{A}/obs?i=*&limit=10&asof={at}", node.base);
    let p1 = page(&node, &url).await;
    assert_eq!(p1.members.len(), 10);
    assert_eq!(p1.asof().as_deref(), Some(at));
    set(&clock, &plus(C0, H));
    let p2 = page(&node, &url).await;
    assert_eq!(p2.members, p1.members);
    assert_eq!(p2.next, p1.next);
    assert_eq!(p2.asof().as_deref(), Some(at), "next pins the asof asked for");
    let (r1, r2) = (rest(&node, &p1).await, rest(&node, &p2).await);
    assert_eq!(r1, r2);
    assert_eq!(p1.members.len() + r1.len(), 25);
    // A write after `at` does not change what was asked as of `at`.
    node.put_value(&format!("{A}/obs"), &args(&[("i", n("3"))]), n("99.5"), &plus(C0, 30 * MIN));
    let p3 = page(&node, &url).await;
    assert_eq!(p3.members, p1.members);
    assert_eq!(p3.next, p1.next);
}
