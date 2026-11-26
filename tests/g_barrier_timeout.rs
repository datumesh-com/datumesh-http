//! The guard patch, second review (27 Nov), B3: the barrier waits for writers in flight at most
//! `PoolOptions::barrier_timeout` (default 5 s); past it a request fails 503 `E_INPUT_UNAVAILABLE`.
//! A file of its own: it needs the store's barrier_timeout, which the rest of the guard tests do not.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use datumesh_store::pg_unstable::postgres::{Config as PgConfig, NoTls};
use datumesh_store::{PgStore, PoolOptions};

const HOST: &str = "mesh-test.datumesh.com";
const LOCK_KEY: i64 = 7_412_030_007;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_held_past_the_barrier_timeout_is_e_input_unavailable() {
    let db = Arc::new(TestDb::migrated());
    let url = db.url();
    let store = Arc::new(bl(|| {
        PgStore::connect_with(&url, PoolOptions::new().max_size(40).barrier_timeout(Duration::from_millis(300)))
            .unwrap()
    }));
    let mut setup = Setup::new(HOST);
    setup.db = Some(db.clone());
    let n = Node::start_on_store(setup, store, |c| c.rate.enabled = false, |b| b).await;
    let site =
        r#"{"args":{"site":{"type":"string","valid":false}},"order":["site"],"returns":{"type":"number","unit":"1"}}"#;
    let reading = format!("{HOST}/reading");
    n.stored(&reading, site);
    n.put_value(&reading, &args(&[("site", s("a"))]), common::n("1"), T1);
    let empty = r#"{"args":{},"order":[],"returns":{"type":"number","unit":"1"}}"#;
    n.formula(&format!("{HOST}/all"), &format!("sum({reading}(site: *))"), empty);
    // Another session holds the version lock, as a writer in flight does.
    let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        let cfg: PgConfig = url.parse().unwrap();
        let mut c = cfg.connect(NoTls).unwrap();
        let mut t = c.transaction().unwrap();
        t.execute("SELECT pg_advisory_xact_lock($1)", &[&LOCK_KEY]).unwrap();
        held_tx.send(()).unwrap();
        let _ = release_rx.recv();
        t.commit().unwrap();
    });
    held_rx.recv().unwrap();
    let r = n.get(&format!("/cell/{HOST}/all")).await;
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    assert_eq!(r.status, 503, "{}", r.text());
    assert!(r.text().contains("E_INPUT_UNAVAILABLE"), "{}", r.text());
    // With the writer gone the request is answered.
    assert_eq!(n.get(&format!("/cell/{HOST}/all")).await.outcome(), "ok");
}
