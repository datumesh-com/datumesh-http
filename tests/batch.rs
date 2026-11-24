//! Test 8 of the brief, as the user decided it for http 0.5.4 (CONTRACT-next-patch item 17, option b): **a batch costs
//! one round trip and one name lookup for any number of cells, and on loopback 1,000 cells take less than 50 single
//! requests.**
//!
//! * The main proof is counted, so no load can move it (`a_batch_saves_round_trips_and_binding_reads`): one HTTP round
//!   trip in place of 1,000, and one lookup of the name's binding in the store in place of 1,000 (by a read probe in
//!   the database, as `common/h0410.rs` counts reads), and no more cell rows read than the 1,000 requests read.
//! * The time (`thousand_cells`, release builds): the median batch of 1,000 cells is under 50 times the median single
//!   request. Both medians come from the same seven interleaved rounds (1,000 single requests, then batches), so that
//!   a burst of load on a shared machine falls on both sides, not on one.

mod common;
#[path = "common/h0410.rs"]
mod h0410;

use std::time::{Duration, Instant};

use common::*;
use datumesh_core::json::Value;
use datumesh_core::{Cell, ReceiptBuilder, SigningKey};
use datumesh_store::pg_unstable::postgres::{Config as PgConfig, NoTls};
use datumesh_store::Store;
use h0410::{config, held, Fx, Group};

const A: &str = "mesh-a.example";
const SIG: &str = r#"{"args":{"i":{"type":"number","unit":"EUR","valid":false}},"order":["i"],"returns":{"type":"number","unit":"EUR"}}"#;

/// The rounds of the timing: each is 1,000 single requests one after another, then [`BATCHES_PER_ROUND`] batches.
const ROUNDS: usize = 7;
const BATCHES_PER_ROUND: usize = 3;

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    pct(&v, 0.5)
}

fn all_ok(r: &Resp) {
    assert_eq!(r.status, 200);
    let Value::Array(items) = r.json() else { panic!("array") };
    assert_eq!(items.len(), 1000);
    assert!(items
        .iter()
        .all(|e| e.get("envelope").and_then(|x| x.get("outcome")).and_then(Value::as_str) == Some("ok")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thousand_cells() {
    let a = Node::start(A, |c| c.rate.enabled = false).await;
    let name = format!("{A}/obs");
    a.stored(&name, SIG);
    let store = a.store.clone();
    let node_key = a.key.clone();
    let fh = bl(|| a.store.resolve_name(&name, T1).unwrap().unwrap());
    // 1,000 values, signed and stored from several threads.
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let (store, key, fh) = (store.clone(), node_key.clone(), fh.clone());
            std::thread::spawn(move || {
                for i in (t..1000).step_by(8) {
                    put_obs(&store, &key, A, "obs", &fh, i, T1);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let body: Vec<String> = (0..1000).map(|i| format!("\"{A}/obs?i={i}\"")).collect();
    let body = format!("[{}]", body.join(",")).into_bytes();
    // Warm: every cell answered once by the batch and once on its own.
    assert_eq!(a.post("/cells", body.clone()).await.status, 200);
    for i in 0..1000 {
        assert_eq!(a.get(&format!("/cell/{A}/obs?i={i}")).await.status, 200);
    }

    // Interleaved rounds: the 1,000 single requests the batch replaces, then the batch.
    let mut thousands = Vec::new();
    let mut single = Vec::new();
    let mut batches = Vec::new();
    let mut server = Vec::new();
    for _ in 0..ROUNDS {
        let all = Instant::now();
        for i in 0..1000 {
            let t = Instant::now();
            let r = a.get(&format!("/cell/{A}/obs?i={i}")).await;
            single.push(t.elapsed());
            assert_eq!(r.status, 200);
        }
        thousands.push(all.elapsed());
        for _ in 0..BATCHES_PER_ROUND {
            let t = Instant::now();
            let r = a.post("/cells", body.clone()).await;
            batches.push(t.elapsed());
            server.push(r.header("server-timing").unwrap_or_default().trim_start_matches("app;dur=").to_string());
            all_ok(&r);
        }
    }
    let thousand = median(thousands.clone());
    let batch = median(batches.clone());
    single.sort();
    let one = pct(&single, 0.5);
    batches.sort();
    eprintln!("server-side ms: {server:?}");
    let out = format!(
        "batch: 1,000 single requests one after another p50 {thousand:?} (of {ROUNDS} rounds: {thousands:?}); \
         1,000 cells in one POST /cells p50 {batch:?} (min {:?}, max {:?}): {:.1} times cheaper; \
         one cell p50 {one:?} (p99 {:?}), the batch {:.1} times one cell\n",
        batches[0],
        batches[batches.len() - 1],
        thousand.as_secs_f64() / batch.as_secs_f64(),
        pct(&single, 0.99),
        batch.as_secs_f64() / one.as_secs_f64()
    );
    std::fs::write(out_dir().join("batch.txt"), &out).unwrap();
    eprint!("{out}");
    // Timings hold for an optimised build (`cargo test --release`, as CI runs this test): on loopback, 1,000 cells take
    // less than 50 single requests (the user's decision on item 17), on medians of the interleaved rounds.
    if !cfg!(debug_assertions) {
        assert!(batch < one * 50, "{out}");
    }
}

/// Stores the value `<i>.5` of `<host>/<f>(i: i)` at `tx`, signed by `key`, in any store.
fn put_obs_in(store: &dyn Store, key: &SigningKey, host: &str, f: &str, fh: &str, i: usize, tx: &str) {
    let cell = Cell { ns: host.into(), fn_name: f.into(), fn_hash: fh.into(), args: args(&[("i", n(&i.to_string()))]) };
    let r = ReceiptBuilder::new()
        .cell(cell.clone())
        .value(n(&format!("{i}.5")))
        .valid(Default::default())
        .asof(tx)
        .tx(tx)
        .issued(tx)
        .publisher(host)
        .signer_host(host)
        .licence("CC0-1.0")
        .summary("An observation.")
        .sign(key)
        .unwrap();
    let h = store.put_receipt(&receipt_json(&r)).unwrap();
    store.put_version(&cell.id(), tx, &h).unwrap();
}

/// The read probe of `common/h0410.rs` on one more table, `name_bindings`: every row of it a statement visits draws one
/// number from `probe_bindings` (the probe's role does not own the table, so row-level security applies to it).
struct BindingProbe {
    db_url: String,
}

impl BindingProbe {
    fn arm(fx: &Fx) -> BindingProbe {
        let p = fx.probe();
        let (url, role) = (p.db_url.clone(), p.role.clone());
        let u = url.clone();
        on_thread(move || {
            let mut c = u.parse::<PgConfig>().unwrap().connect(NoTls).unwrap();
            c.batch_execute(&format!(
                "CREATE SEQUENCE IF NOT EXISTS probe_bindings;\
                 GRANT USAGE, SELECT, UPDATE ON SEQUENCE probe_bindings TO \"{role}\";\
                 ALTER TABLE name_bindings ENABLE ROW LEVEL SECURITY;\
                 CREATE POLICY probe_sel ON name_bindings FOR SELECT USING (nextval('probe_bindings') > 0);\
                 CREATE POLICY probe_ins ON name_bindings FOR INSERT WITH CHECK (true);\
                 CREATE POLICY probe_upd ON name_bindings FOR UPDATE USING (true) WITH CHECK (true);\
                 CREATE POLICY probe_del ON name_bindings FOR DELETE USING (true)"
            ))
            .unwrap_or_else(|e| panic!("arming name_bindings: {e}"));
        });
        BindingProbe { db_url: url }
    }

    /// Rows of `name_bindings` visited since [`BindingProbe::arm`].
    fn reads(&self) -> u64 {
        let url = self.db_url.clone();
        on_thread(move || {
            let mut c = url.parse::<PgConfig>().unwrap().connect(NoTls).unwrap();
            let row =
                c.query_one("SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM probe_bindings", &[]).unwrap();
            row.get::<_, i64>(0) as u64
        })
    }
}

/// What a batch of 1,000 saves, counted, not timed: one HTTP round trip in place of 1,000 (the server's own request
/// counter), and one store lookup of the name's binding in place of 1,000 (rows of `name_bindings` visited, by the
/// read probe); and it reads no more cell rows than the 1,000 single requests do. Every count is exact, so the test
/// does not depend on the load of the machine.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_batch_saves_round_trips_and_binding_reads() {
    let fx = Fx::pg("batch-saves");
    let host = common::lite::HOST;
    let name = format!("{host}/obs");
    let fh = {
        let store = fx.store.clone();
        let name = name.clone();
        on_thread(move || {
            let d = format!(r#"{{"code":{{"body":"","lang":"stored"}},"inputs":[],"signature":{SIG}}}"#);
            let h = store.put_descriptor(d.as_bytes()).unwrap();
            store.bind_name(&name, &h, "2026-02-01T00:00:00.000000Z").unwrap();
            h
        })
    };
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let (store, key, fh) = (fx.store.clone(), fx.key.clone(), fh.clone());
            std::thread::spawn(move || {
                for i in (t..1000).step_by(8) {
                    put_obs_in(&*store, &key, host, "obs", &fh, i, T1);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    fx.probe().arm();
    let bindings = BindingProbe::arm(&fx);
    // Nothing but the requests under test reads the store: the background builds are held back; and no request
    // turns into a job (a budget of a minute), so every read is the request's own.
    let mut vars = held(&[]);
    vars.push(("DATUMESH_BUDGET_MS", "60000"));
    let w = fx.serve(config(&vars, &[])).await;
    let reqs: Vec<String> = (0..1000).map(|i| format!("{name}?i={i}")).collect();
    let body = serde_json::Value::Array(reqs.iter().map(|r| serde_json::json!(r)).collect());
    let batch_ok = |r: &h0410::Rep| {
        assert_eq!(r.status, 200, "{}", r.text());
        let items = r.json();
        let items = items.as_array().unwrap();
        assert_eq!(items.len(), 1000);
        assert!(items.iter().all(|e| e["envelope"]["outcome"] == "ok"), "{}", r.text());
    };
    // Warm, both ways.
    batch_ok(&w.post("/cells", &body).await);
    for r in &reqs {
        assert_eq!(w.get(&format!("/cell/{r}")).await.status, 200);
    }

    const BATCH_REQS: &str = "datumesh_http_requests_total{route=\"/cells\"";
    const SINGLE_REQS: &str = "datumesh_http_requests_total{route=\"/cell/{cell}\"";
    const BATCH_OK: &str = "datumesh_envelopes_total{route=\"/cells\",outcome=\"ok\"}";
    let count = |v: Option<f64>| v.unwrap_or(0.0) as u64;

    // The batch.
    let (reqs0, ok0) = (count(w.metric(BATCH_REQS).await), count(w.metric(BATCH_OK).await));
    let (bind0, cells0) = (bindings.reads(), w.fx.probe().reads(Group::Cells));
    batch_ok(&w.post("/cells", &body).await);
    let (bind1, cells1) = (bindings.reads(), w.fx.probe().reads(Group::Cells));
    let (reqs1, ok1) = (count(w.metric(BATCH_REQS).await), count(w.metric(BATCH_OK).await));
    let (batch_bind, batch_cells) = (bind1 - bind0, cells1 - cells0);

    // The 1,000 single requests it replaces.
    let singles0 = count(w.metric(SINGLE_REQS).await);
    let (bind2, cells2) = (bindings.reads(), w.fx.probe().reads(Group::Cells));
    for r in &reqs {
        let one = w.get(&format!("/cell/{r}")).await;
        assert_eq!(one.status, 200, "{}", one.text());
    }
    let (bind3, cells3) = (bindings.reads(), w.fx.probe().reads(Group::Cells));
    let singles1 = count(w.metric(SINGLE_REQS).await);
    let (single_bind, single_cells) = (bind3 - bind2, cells3 - cells2);

    let out = format!(
        "batch saves: round trips 1 (server counted {}) for 1,000 envelopes (counted {}) against {} single requests; \
         name_bindings rows read {batch_bind} against {single_bind}; cell rows read {batch_cells} against {single_cells}\n",
        reqs1 - reqs0,
        ok1 - ok0,
        singles1 - singles0
    );
    std::fs::write(out_dir().join("batch-saves.txt"), &out).unwrap();
    eprint!("{out}");
    // One round trip in place of 1,000.
    assert_eq!((reqs1 - reqs0, ok1 - ok0), (1, 1000), "{out}");
    assert_eq!(singles1 - singles0, 1000, "{out}");
    // One binding lookup in place of 1,000: every single request looks the name up in the store, the batch once.
    assert!(single_bind >= 1000, "each single request looks the binding up: {out}");
    assert!(batch_bind >= 1 && batch_bind * 1000 <= single_bind, "the batch looks the binding up once: {out}");
    // And it reads no more of the cells than the requests it replaces.
    assert!(batch_cells <= single_cells, "{out}");
}

/// A batch answered in several chunks (one per core, at least 64 requests each) is, byte for
/// byte, the envelopes `/cell` gives for its requests, wrapped as 5.7 says, in request order,
/// and the metrics count its outcomes: whatever each chunk writes on its own, the batch is
/// the same.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chunks_write_the_batch() {
    let a = Node::start(A, |c| c.rate.enabled = false).await;
    let name = format!("{A}/obs");
    a.stored(&name, SIG);
    let fh = bl(|| a.store.resolve_name(&name, T1).unwrap().unwrap());
    bl(|| {
        for i in 0..200 {
            put_obs(&a.store, &a.key, A, "obs", &fh, i, T1);
        }
    });
    // Stored values (ok), values not stored (not_covered), an unbound name (not_covered with
    // E_NAME_UNBOUND) and malformed arguments (bad_request), mixed through every chunk.
    let reqs: Vec<String> = (0..400)
        .map(|k| match k % 10 {
            3 => format!("{A}/nobody?i={k}"),
            7 => format!("{A}/obs?i=0{k}"),
            9 => format!("{A}/obs?i={}", 1000 + k),
            _ => format!("{A}/obs?i={}", k % 200),
        })
        .collect();
    let body: Vec<String> = reqs.iter().map(|r| format!("\"{r}\"")).collect();
    let r = a.post("/cells", format!("[{}]", body.join(",")).into_bytes()).await;
    assert_eq!(r.status, 200, "{}", r.text());

    let mut want = b"[".to_vec();
    let mut outcomes = std::collections::BTreeMap::<String, u64>::new();
    for (k, req) in reqs.iter().enumerate() {
        let one = a.get(&format!("/cell/{req}")).await;
        if k > 0 {
            want.push(b',');
        }
        want.extend_from_slice(b"{\"envelope\":");
        want.extend_from_slice(&one.body);
        want.extend_from_slice(format!(",\"status\":{}}}", one.status).as_bytes());
        *outcomes.entry(one.outcome()).or_default() += 1;
    }
    want.push(b']');
    assert_eq!(r.text(), String::from_utf8(want).unwrap());
    assert_eq!(outcomes.len(), 3, "{outcomes:?}");

    let m = a.get("/metrics").await.text();
    for (o, n) in &outcomes {
        let line = format!("datumesh_envelopes_total{{route=\"/cells\",outcome=\"{o}\"}} {n}");
        assert!(m.lines().any(|l| l == line), "{line} in\n{m}");
    }
}
