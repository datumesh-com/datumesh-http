//! CONTRACT-perf R-T2 (H7): for a slice in Arrow, the schema metadata `asof` is the instant the
//! members were computed for: the reused `at` of a reused first page (H4), the same instant
//! the JSON `next` pins. Runs on PostgreSQL through the shared `Node` harness, with a settable
//! mesh clock.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use datumesh_core::json::Value;
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

async fn world(count: usize) -> (Node, Arc<Settable>) {
    let clock = Arc::new(Settable(Mutex::new(C0.to_string())));
    let c = clock.clone();
    let node = Node::start_with(Setup::new(A), |c| c.rate.enabled = false, move |b| b.clock(c)).await;
    node.stored(&format!("{A}/obs"), SIG);
    let nd = &node;
    tokio::task::block_in_place(|| load_obs(nd, "obs", count, 4, T1));
    (node, clock)
}

/// The `asof` a JSON first page pins in its `next` link.
async fn json_next_asof(node: &Node, url: &str) -> String {
    let r = node.http.get(url).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let v = datumesh_core::json::parse(&r.bytes().await.unwrap()).unwrap();
    let next = v.get("next").and_then(Value::as_str).expect("a next link");
    next.split("asof=").nth(1).unwrap().split('&').next().unwrap().to_string()
}

/// The schema metadata `asof` of an Arrow response, and its row count.
async fn arrow_asof(node: &Node, path: &str) -> (String, usize) {
    let r = node.get_with(path, &[("accept", "application/vnd.apache.arrow.stream")]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.header("content-type").as_deref(), Some("application/vnd.apache.arrow.stream"));
    let reader = arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(&r.body), None).unwrap();
    let asof = reader.schema().metadata().get("asof").cloned().expect("asof metadata");
    let rows = reader.map(|b| b.unwrap().num_rows()).sum();
    (asof, rows)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_arrow_first_page_carries_the_instant_its_members_were_computed_for() {
    let (node, clock) = world(25).await;
    let json = format!("{}/slice/{A}/obs?i=*&limit=10", node.base);
    let path = format!("/slice/{A}/obs?i=*");

    // A latest first page in JSON, answered as of now (C0): its `next` pins C0.
    let pinned = json_next_asof(&node, &json).await;
    assert_eq!(pinned, C0);

    // An hour later, nothing written: the Arrow first page reuses those members, so its
    // metadata is the instant of the JSON page's `next`, not the new now.
    let later = plus(C0, H);
    set(&clock, &later);
    let (asof, rows) = arrow_asof(&node, &path).await;
    assert_eq!(rows, 25);
    assert_eq!(asof, pinned, "the Arrow asof is the instant the JSON next pins");
    assert_ne!(asof, later, "and not the new now");

    // After a write to the series the members are recomputed, so the next Arrow first page
    // carries the new now.
    node.put_value(&format!("{A}/obs"), &args(&[("i", n("3"))]), n("99.5"), &plus(C0, 30 * MIN));
    let (asof, rows) = arrow_asof(&node, &path).await;
    assert_eq!(rows, 25);
    assert_eq!(asof, later, "a changed series is answered as of now");
    let later2 = plus(C0, 2 * H);
    set(&clock, &later2);
    node.put_value(&format!("{A}/obs"), &args(&[("i", n("4"))]), n("98.5"), &plus(C0, 90 * MIN));
    let (asof, _) = arrow_asof(&node, &path).await;
    assert_eq!(asof, later2);
}
