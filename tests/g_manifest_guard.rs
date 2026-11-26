//! The guard patch, Part G (CONTRACT-guard-patch.md), at the mesh: G4 (the switch), G5 (the members
//! rebuilt on demand at `GET /manifest/<slice_id>/<members_hash>`), G6 (the other readers of a
//! manifest) and G7 (nothing stored changes), on PostgreSQL nodes with a fixed mesh clock. Failing
//! tests: the switch, the rebuild and the paged form are not written yet.
//!
//! API and behaviour assumed (the contract leaves these open; the test writer's choices):
//!
//! * `Config::manifest_store_max: usize`, default 0, read from `DATUMESH_MANIFEST_STORE_MAX` by
//!   `Config::from_vars` (a malformed value is a `ConfigError`). 0 runs the engine with
//!   `ManifestPolicy::MemberSetOnly`; above 0 a member list of at most that many members is stored
//!   as a manifest (as today) and a longer one is not (its member-set row is, as always).
//! * The full form of `/manifest` answers the canonical manifest bytes, held or rebuilt, `200` with
//!   `Cache-Control: public, max-age=31536000, immutable`, with no member limit.
//! * A rebuild whose hash differs, or with a member that cannot be listed (a computed member with no
//!   computed version as of the row's asof), answers `503` with a JSON body naming
//!   `E_INPUT_UNAVAILABLE` and no immutable caching, logs a line, counts
//!   `datumesh_manifest_rebuild_failures_total` in `/metrics`, and stores nothing. An entry with
//!   neither a manifest nor a row is `404` as today.
//! * The paged form: `?after=<member hash>&limit=<n>` (either one makes it paged; `after` is
//!   exclusive). `limit` defaults to 1,000 and may be 1..=10,000; **a limit over 10,000 (or 0) is
//!   refused with 400** (the test writer's pick between refused and clamped), as is an `after` that
//!   is not 64 lowercase hex digits. A page is `200` JSON with at least `"members"`: the member
//!   hashes after `after`, ascending, at most `limit`. A page followed by more members has a `Link`
//!   header `<URL>; rel="next"` whose URL (absolute or a path) is this route with `after=<the page's
//!   last member>` and the same `limit`; the last page has none. The hash is checked over the whole
//!   set before the first page: a mismatch is `503` as for the full form.
//!
//! The paths that evaluate on request (GET /cell of a computed cell, POST /cells, GET /explain,
//! POST /verify at level recomputed) are exercised; the chain and lookup links (class calls) run on
//! the same engine and are not exercised here (they need the classes world of cr_chain.rs).
//!
//! The coordinator's flood timeline (26 Nov) and the paths that store cells (publish-stored with the
//! run's tx and with a tx of its own, `import-signed` / `POST /bundles` through `bundle::import_signed`)
//! are at the end. Cross-mesh imports cannot add a slice member (slices of foreign functions are
//! refused, foreign.rs), and the catalogue job stores catalogue cells only.

mod common;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use common::lite::FixedClock;
use common::*;
use datumesh_core::json::Value;
use datumesh_core::{Manifest, Receipt, ReceiptBuilder};
use datumesh_http::publish::{publish_stored_with, PublishOptions};
use datumesh_http::Config;
use datumesh_store::pg_unstable::postgres::{Config as PgConfig, NoTls};

const HOST: &str = "mesh-test.datumesh.com";
const NS: &str = "mesh-test.datumesh.com/scratch_guard";
const LEVEL: &str = "mesh-test.datumesh.com/scratch_guard/level";
const FLOOD: &str = "mesh-test.datumesh.com/scratch_guard/flood";
const DOUBLE: &str = "mesh-test.datumesh.com/scratch_guard/double";
const ALLD: &str = "mesh-test.datumesh.com/scratch_guard/alldouble";
const WINDOW: &str = "mesh-test.datumesh.com/scratch_guard/window";
const DAY: &str = "2016-11-20";
const DAY2: &str = "2016-11-21";

/// The functions and formulas are bound by a run at this clock.
const R0: &str = "2026-11-15T10:00:00.000000Z";
/// The first run of values (stations a and b): T0's world.
const R1: &str = "2026-11-15T12:00:00.000000Z";
/// When flood is first computed (T0), as of.
const A0: &str = "2026-11-15T12:00:00.500000Z";
/// Between T0 and T1.
const A1: &str = "2026-11-15T12:00:01.000000Z";
/// The second run (T1): station c's readings, recorded with this tx.
const R2: &str = "2026-11-15T12:00:02.000000Z";
/// Between T1 and now.
const A2: &str = "2026-11-15T12:00:03.000000Z";
/// After the bindings, before every value: a tx "from elsewhere", older than T0's members.
const OLD: &str = "2026-11-15T11:00:00.000000Z";
/// The mesh's clock: now (T2).
const MESH_NOW: &str = "2026-11-16T00:00:00.000000Z";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

const RET: &str = r#"{"type":"number","unit":"1"}"#;

fn level_sig() -> String {
    format!(
        r#"{{"args":{{"d":{{"type":"date","valid":false}},"station":{{"type":"string","valid":false}}}},"order":["station","d"],"returns":{RET}}}"#
    )
}
fn flood_sig() -> String {
    format!(r#"{{"args":{{"d":{{"type":"date","valid":false}}}},"order":["d"],"returns":{RET}}}"#)
}
fn double_sig() -> String {
    format!(
        r#"{{"args":{{"station":{{"extent":["a","b"],"type":"string","valid":false}}}},"order":["station"],"returns":{RET}}}"#
    )
}
fn empty_sig() -> String {
    format!(r#"{{"args":{{}},"order":[],"returns":{RET}}}"#)
}

fn formula_line(name: &str, source: &str, signature: &str) -> String {
    serde_json::json!({"formula": name, "source": source, "signature": serde_json::from_str::<serde_json::Value>(signature).unwrap()})
        .to_string()
}

/// A cell line of `level(station, d)`; `tx` (and asof) the run's clock unless given.
fn level_line(station: &str, d: &str, value: &str, tx: Option<&str>) -> String {
    let mut v = serde_json::json!({
        "cell": {"fn": LEVEL, "args": {"station": {"type": "string", "value": station}, "d": {"type": "date", "value": d}}},
        "licence": "CC0-1.0", "summary": "Synthetic test data.",
        "value": {"type": "number", "value": value},
    });
    if let Some(t) = tx {
        v["tx"] = serde_json::json!(t);
        v["asof"] = serde_json::json!(t);
    }
    v.to_string()
}

fn withdrawal_line(station: &str, d: &str, reason: &str) -> String {
    serde_json::json!({
        "cell": {"fn": LEVEL, "args": {"station": {"type": "string", "value": station}, "d": {"type": "date", "value": d}}},
        "licence": "CC0-1.0", "summary": "Synthetic test data.", "withdrawn": reason,
    })
    .to_string()
}

/// The functions: level stored; flood(d) = the sum of level(*, d) (a stored-function slice);
/// double(station) over the extent [a, b] and alldouble() = the sum of double(*) (a computed-function
/// slice); window() = the sum of the versions of level(a, DAY2) in the day (a version slice).
fn functions() -> Vec<String> {
    vec![
        common::pub031::function_line(LEVEL, &level_sig(), None),
        formula_line(FLOOD, &format!("sum({LEVEL}(station: *, d: d))"), &flood_sig()),
        formula_line(DOUBLE, &format!("{LEVEL}(station: station, d: {DAY}) * 2"), &double_sig()),
        formula_line(ALLD, &format!("sum({DOUBLE}(station: *))"), &empty_sig()),
        formula_line(
            WINDOW,
            &format!(
                "sum(versions({LEVEL}(station: \"a\", d: {DAY2}), 2026-11-15T00:00:00.000000Z, 2026-11-15T23:00:00.000000Z))"
            ),
            &empty_sig(),
        ),
    ]
}

/// T0's values: stations a (100) and b (200) for DAY, a's first version for DAY2.
fn run1() -> Vec<String> {
    vec![level_line("a", DAY, "100", None), level_line("b", DAY, "200", None), level_line("a", DAY2, "1", None)]
}

/// T0's world: the functions bound at R0, then run1 at R1.
fn seed(n: &Node) {
    publish(n, &functions(), R0);
    publish(n, &run1(), R1);
}

/// T1: station c's readings for November 2016 (a new series under flood(DAY)'s pattern), and a's
/// second version for DAY2.
fn run2() -> Vec<String> {
    vec![level_line("c", DAY, "50", None), level_line("c", "2016-11-19", "40", None), level_line("a", DAY2, "2", None)]
}

static FILES: AtomicU32 = AtomicU32::new(0);

/// Publishes `lines` into the node's store with `publish-stored`, the run's clock at `clock`.
fn publish(n: &Node, lines: &[String], clock: &'static str) {
    let path = out_dir().join(format!("guard-g-{}-{}.jsonl", std::process::id(), FILES.fetch_add(1, Ordering::SeqCst)));
    std::fs::write(&path, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    let opts = PublishOptions { clock: Arc::new(FixedClock(clock)), backfill: None };
    bl(|| publish_stored_with(&*n.store, &n.key, &n.host, std::slice::from_ref(&n.host), &path, &opts))
        .unwrap_or_else(|e| panic!("publish refused: {e}"));
    let _ = std::fs::remove_file(&path);
}

/// A node for HOST at the fixed clock, its configuration changed by `cfg`; on `db` when given (a
/// fresh mesh, with no memo, on a store already filled).
async fn node_on(db: Option<Arc<TestDb>>, cfg: impl FnOnce(&mut Config)) -> Node {
    let mut setup = Setup::new(HOST);
    setup.db = db;
    Node::start_with(
        setup,
        |c| {
            c.rate.enabled = false;
            cfg(c)
        },
        |b| b.clock(Arc::new(FixedClock(MESH_NOW))),
    )
    .await
}

async fn node(cfg: impl FnOnce(&mut Config)) -> Node {
    node_on(None, cfg).await
}

/// A node at the default configuration (DATUMESH_MANIFEST_STORE_MAX unset: 0).
async fn guarded() -> Node {
    node(|_| {}).await
}

/// A node storing every manifest, as 0.5.4 does.
async fn storing() -> Node {
    node(|c| c.manifest_store_max = 1_000_000).await
}

fn count(n: &Node, table: &str) -> i64 {
    let url = n.db.url();
    let table = table.to_string();
    on_thread(move || {
        let cfg: PgConfig = url.parse().unwrap();
        let mut c = cfg.connect(NoTls).unwrap();
        c.query_one(&format!("SELECT count(*) FROM {table}"), &[]).unwrap().get(0)
    })
}

fn manifest_rows(n: &Node) -> (i64, i64) {
    (count(n, "manifests"), count(n, "manifest_members"))
}

fn flood_path(d: &str, asof: Option<&str>) -> String {
    match asof {
        Some(a) => format!("/cell/{FLOOD}?d={d}&asof={a}"),
        None => format!("/cell/{FLOOD}?d={d}"),
    }
}

/// A computed answer: its receipt and the receipt's bytes.
async fn answer(n: &Node, path: &str) -> (Receipt, Vec<u8>) {
    let r = n.get(path).await;
    assert_eq!(r.outcome(), "ok", "{path}: {}", r.text());
    let b = r.json().get("receipt").unwrap().to_jcs();
    (Receipt::from_json(&b).unwrap(), b)
}

/// The receipt hash of a stored cell's answer.
async fn stored_hash(n: &Node, station: &str, d: &str) -> String {
    answer(n, &format!("/cell/{LEVEL}?d={d}&station={station}")).await.0.hash().to_string()
}

fn manifest_path(entry: &str) -> String {
    let p: Vec<&str> = entry.split(':').collect();
    assert_eq!(p.len(), 3, "{entry}");
    format!("/manifest/{}/{}", p[1], p[2])
}

/// The full form: 200, immutable, the canonical bytes of a manifest of `entry`.
async fn full(n: &Node, entry: &str) -> Vec<u8> {
    let r = n.get(&manifest_path(entry)).await;
    assert_eq!(r.status, 200, "{entry}: {}", r.text());
    assert_eq!(r.header("cache-control").as_deref(), Some(IMMUTABLE), "{entry}: immutable, held or rebuilt");
    let m = Manifest::from_json(&r.body).unwrap_or_else(|e| panic!("{entry}: not a manifest ({e:?})"));
    assert_eq!(m.entry(), entry, "the hash is the entry's");
    assert_eq!(m.canonical(), r.body, "canonical bytes");
    r.body
}

async fn verifies_recomputed(n: &Node, bytes: &[u8], what: &str) {
    let r = n.post("/verify", bytes.to_vec()).await;
    let v = r.json();
    assert_eq!(v.get("ok"), Some(&Value::Bool(true)), "{what}: {}", r.text());
    assert_eq!(v.get("level").and_then(Value::as_str), Some("recomputed"), "{what}: {}", r.text());
}

// ---- G4: the switch --------------------------------------------------------------------------

#[test]
fn the_switch_is_read_from_the_environment() {
    assert_eq!(Config::new("mesh.example").manifest_store_max, 0, "default 0: no member list is stored");
    let with = |v: Option<&str>| {
        let v = v.map(str::to_string);
        Config::from_vars(move |k| match k {
            "DATUMESH_HOST" => Some("mesh.example".to_string()),
            "DATUMESH_MANIFEST_STORE_MAX" => v.clone(),
            _ => None,
        })
    };
    assert_eq!(with(None).unwrap().manifest_store_max, 0);
    assert_eq!(with(Some("0")).unwrap().manifest_store_max, 0);
    assert_eq!(with(Some("250")).unwrap().manifest_store_max, 250);
    assert!(with(Some("lots")).is_err(), "a malformed value is refused");
    assert!(with(Some("-1")).is_err(), "a negative value is refused");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn at_0_no_evaluation_started_by_a_request_stores_a_member_list() {
    let n = guarded().await;
    seed(&n);
    // GET /cell of computed cells over the three kinds of slices.
    let (flood, fb) = answer(&n, &flood_path(DAY, None)).await;
    assert_eq!(flood.value().lexical(), "300");
    let (alld, ab) = answer(&n, &format!("/cell/{ALLD}")).await;
    assert_eq!(alld.value().lexical(), "600");
    let (window, wb) = answer(&n, &format!("/cell/{WINDOW}")).await;
    assert_eq!(window.value().lexical(), "1");
    assert_eq!(manifest_rows(&n), (0, 0), "GET /cell");
    // POST /cells.
    let body = format!(r#"["{FLOOD}?d={DAY2}", "{FLOOD}?d=2016-11-22"]"#).into_bytes();
    let r = n.post("/cells", body).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(manifest_rows(&n), (0, 0), "POST /cells");
    // GET /explain of a cell not read before: its slice node has the count and the slice (G6).
    let r = n.get(&format!("/explain/{FLOOD}?d=2016-11-23")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(manifest_rows(&n), (0, 0), "GET /explain");
    let r = n.get(&format!("/explain/{FLOOD}?d={DAY}")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = r.json();
    let node = &array(v.get("inputs").expect("inputs"))[0];
    assert_eq!(node.get("entry").and_then(Value::as_str), Some(flood.inputs()[0].as_str()));
    assert_eq!(node.get("members").and_then(Value::as_number_text), Some("2"), "the count, from the row: {}", r.text());
    let slice = datumesh_core::Slice::from_value(node.get("slice").expect("the slice, from the row")).unwrap();
    assert_eq!(flood.inputs()[0], format!("slice:{}:{}", slice.id(), flood.inputs()[0].rsplit(':').next().unwrap()));
    // POST /verify at level recomputed, on a fresh mesh (nothing in its memo): the evaluation it
    // starts stores no member list either, and the manifests are rebuilt.
    let fresh = node_on(Some(n.db.clone()), |_| {}).await;
    for (b, what) in [(&fb, "flood"), (&ab, "alldouble"), (&wb, "window")] {
        verifies_recomputed(&fresh, b, what).await;
    }
    assert_eq!(manifest_rows(&n), (0, 0), "POST /verify");
    // A row for every member set read.
    assert!(count(&n, "member_sets") >= 5, "member_sets: {}", count(&n, "member_sets"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn above_0_lists_up_to_that_size_are_stored_and_longer_ones_are_not() {
    let n = node(|c| c.manifest_store_max = 2).await;
    seed(&n);
    let (flood, _) = answer(&n, &flood_path(DAY, None)).await;
    assert_eq!(flood.value().lexical(), "300");
    assert_eq!(manifest_rows(&n), (1, 2), "two members: stored, as today");
    assert_eq!(count(&n, "member_sets"), 1, "and the row");
    publish(&n, &run2(), R2);
    let fresh = node_on(Some(n.db.clone()), |c| c.manifest_store_max = 2).await;
    let (flood3, _) = answer(&fresh, &flood_path(DAY, None)).await;
    assert_eq!(flood3.value().lexical(), "350");
    assert_eq!(manifest_rows(&n), (1, 2), "three members: not stored");
    assert_eq!(count(&n, "member_sets"), 2, "its row is");
    // One member: stored.
    let (one, _) = answer(&fresh, &flood_path("2016-11-19", None)).await;
    assert_eq!(one.value().lexical(), "40");
    assert_eq!(manifest_rows(&n), (2, 3));
    // The rebuilt one is served like the held ones.
    full(&fresh, &flood3.inputs()[0]).await;
    full(&fresh, &flood.inputs()[0]).await;
}

// ---- G5: the members, rebuilt on demand ------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn held_manifests_are_served_as_today() {
    let n = storing().await;
    seed(&n);
    let (flood, _) = answer(&n, &flood_path(DAY, None)).await;
    let entry = &flood.inputs()[0];
    let held = bl(|| n.store.manifest(entry).expect("held").into_owned());
    assert_eq!(full(&n, entry).await, held);
    // The paged form over a held manifest: one member a page.
    let r = n.get(&format!("{}?limit=1", manifest_path(entry))).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(members_of(&r).len(), 1);
    assert!(r.header("link").is_some(), "a next page");
    assert_eq!(collect_pages(&n, entry, 1).await, Manifest::from_json(&held).unwrap().members().to_vec());
}

fn array(v: &Value) -> &[Value] {
    match v {
        Value::Array(a) => a,
        other => panic!("not an array: {other:?}"),
    }
}

/// The member hashes of a page.
fn members_of(r: &Resp) -> Vec<String> {
    let v = r.json();
    array(v.get("members").unwrap_or_else(|| panic!("a page has members: {}", r.text())))
        .iter()
        .map(|m| m.as_str().unwrap().to_string())
        .collect()
}

/// The path of a `Link: <URL>; rel="next"` header (the URL absolute or a path).
fn next_of(r: &Resp) -> Option<String> {
    let link = r.header("link")?;
    assert!(link.contains("rel=\"next\""), "{link}");
    let url = link.split('<').nth(1).and_then(|s| s.split('>').next()).unwrap_or_else(|| panic!("{link}"));
    Some(match url.find("/manifest/") {
        Some(i) => url[i..].to_string(),
        None => panic!("the next page is this route: {url}"),
    })
}

/// Every page of `entry` at `limit`, followed by its Link headers, concatenated.
async fn collect_pages(n: &Node, entry: &str, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut path = format!("{}?limit={limit}", manifest_path(entry));
    let mut pages = 0;
    loop {
        let r = n.get(&path).await;
        assert_eq!(r.status, 200, "{path}: {}", r.text());
        let ms = members_of(&r);
        assert!(ms.len() <= limit, "{path}: {} members", ms.len());
        pages += 1;
        assert!(pages < 10_000, "the pages end");
        out.extend(ms.iter().cloned());
        match next_of(&r) {
            Some(next) => {
                assert!(next.contains(&format!("after={}", ms.last().unwrap())), "{next}");
                assert!(next.contains(&format!("limit={limit}")), "{next}");
                path = next;
            }
            None => break,
        }
    }
    out
}

/// The slice object of `entry`, from its full form.
async fn slice_of(n: &Node, entry: &str) -> datumesh_core::Slice {
    Manifest::from_json(&full(n, entry).await).unwrap().slice().clone()
}

fn manifest_bytes(slice: &datumesh_core::Slice, members: &[String]) -> Vec<u8> {
    let mut m = members.to_vec();
    m.sort();
    m.dedup();
    Manifest::new(slice.clone(), m).unwrap().canonical()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rebuilt_manifests_are_the_bytes_a_stored_one_would_hold() {
    // Two meshes alike (host, key, clock, published lines): one stores manifests, one does not.
    let held = storing().await;
    let rebuilt = guarded().await;
    for n in [&held, &rebuilt] {
        seed(n);
        publish(n, &run2(), R2);
    }
    for path in [flood_path(DAY, None), flood_path(DAY, Some(A1)), format!("/cell/{ALLD}"), format!("/cell/{WINDOW}")] {
        let (h, hb) = answer(&held, &path).await;
        let (r, rb) = answer(&rebuilt, &path).await;
        assert_eq!(rb, hb, "{path}: the same receipt under both policies");
        let entry = &r.inputs()[0];
        assert_eq!(entry, &h.inputs()[0]);
        let want = bl(|| held.store.manifest(entry).expect("held by the storing mesh").into_owned());
        assert_eq!(bl(|| rebuilt.store.manifest(entry)), None, "{path}: not held");
        assert_eq!(full(&rebuilt, entry).await, want, "{path}: the canonical bytes a stored manifest holds");
        // Again, from a fresh mesh: the same bytes (nothing kept from the request).
        let fresh = node_on(Some(rebuilt.db.clone()), |_| {}).await;
        assert_eq!(full(&fresh, entry).await, want, "{path}");
        verifies_recomputed(&fresh, &rb, &path).await;
    }
    // The three kinds: members as listed.
    let (flood, _) = answer(&rebuilt, &flood_path(DAY, None)).await;
    let slice = slice_of(&rebuilt, &flood.inputs()[0]).await;
    let ms = vec![
        stored_hash(&rebuilt, "a", DAY).await,
        stored_hash(&rebuilt, "b", DAY).await,
        stored_hash(&rebuilt, "c", DAY).await,
    ];
    assert_eq!(full(&rebuilt, &flood.inputs()[0]).await, manifest_bytes(&slice, &ms));
    let (window, _) = answer(&rebuilt, &format!("/cell/{WINDOW}")).await;
    assert_eq!(window.value().lexical(), "3", "1 + 2: both versions of the day");
    let (alld, _) = answer(&rebuilt, &format!("/cell/{ALLD}")).await;
    let a = answer(&rebuilt, &format!("/cell/{DOUBLE}?station=a")).await.0.hash().to_string();
    let b = answer(&rebuilt, &format!("/cell/{DOUBLE}?station=b")).await.0.hash().to_string();
    let aslice = slice_of(&rebuilt, &alld.inputs()[0]).await;
    assert_eq!(full(&rebuilt, &alld.inputs()[0]).await, manifest_bytes(&aslice, &[a, b]), "computed members");
    assert_eq!(manifest_rows(&rebuilt), (0, 0), "the rebuild is never stored");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_different_hash_or_a_missing_member_is_input_unavailable() {
    let n = guarded().await;
    seed(&n);
    let (flood, _) = answer(&n, &flood_path(DAY, None)).await;
    let entry = flood.inputs()[0].clone();
    let row = bl(|| n.store.member_set(&entry).unwrap().expect("the row"));
    // A row whose members hash is not the hash of the members listed as of its asof.
    let mut wrong = row.clone();
    wrong.members_hash = "0".repeat(64);
    wrong.entry = format!("slice:{}:{}", row.slice_id, wrong.members_hash);
    assert!(bl(|| n.store.put_member_set(&wrong).unwrap()));
    let refused = |r: &Resp, what: &str| {
        assert_eq!(r.status, 503, "{what}: {}", r.text());
        assert!(r.text().contains("E_INPUT_UNAVAILABLE"), "{what}: {}", r.text());
        assert_ne!(r.header("cache-control").as_deref(), Some(IMMUTABLE), "{what}: never cached as immutable");
    };
    refused(&n.get(&manifest_path(&wrong.entry)).await, "a different hash");
    refused(&n.get(&format!("{}?limit=1", manifest_path(&wrong.entry))).await, "a different hash, paged");
    // A computed-function slice row whose members have no computed version as of its asof (never
    // evaluated on this mesh): a member cannot be listed.
    let other = storing().await;
    seed(&other);
    let (alld, _) = answer(&other, &format!("/cell/{ALLD}")).await;
    let arow = bl(|| other.store.member_set(&alld.inputs()[0]).unwrap().expect("the row"));
    assert!(bl(|| n.store.put_member_set(&arow).unwrap()));
    refused(&n.get(&manifest_path(&alld.inputs()[0])).await, "a missing member");
    // Nothing guessed, nothing stored; the failures counted.
    assert_eq!(manifest_rows(&n), (0, 0));
    let m = n.get("/metrics").await.text();
    assert!(metric(&m, "datumesh_manifest_rebuild_failures_total") >= 3.0, "{m}");
    // An entry with neither a manifest nor a row: not found, as today.
    let none = format!("slice:{}:{}", row.slice_id, "1".repeat(64));
    assert_eq!(n.get(&manifest_path(&none)).await.status, 404);
    // The good row still rebuilds.
    full(&n, &entry).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_paged_form_and_the_full_form_with_no_member_limit() {
    let n = node(|c| c.budget = std::time::Duration::from_secs(60)).await;
    let mut lines = functions();
    for k in 0..1005 {
        lines.push(level_line(&format!("s{k:04}"), DAY, "1", None));
    }
    publish(&n, &lines, R1);
    let (flood, _) = answer(&n, &flood_path(DAY, None)).await;
    assert_eq!(flood.value().lexical(), "1005");
    let entry = flood.inputs()[0].clone();
    // The full form: every member, one response.
    let all = Manifest::from_json(&full(&n, &entry).await).unwrap().members().to_vec();
    assert_eq!(all.len(), 1005);
    // The default page: 1,000 members, then the rest.
    let first = n.get(&format!("{}?after={}", manifest_path(&entry), "0".repeat(64))).await;
    assert_eq!(first.status, 200, "{}", first.text());
    assert_eq!(members_of(&first), all[..1000].to_vec(), "the default limit is 1,000");
    let next = next_of(&first).expect("a next page");
    assert!(next.contains(&format!("after={}", all[999])), "{next}");
    let last = n.get(&next).await;
    assert_eq!(members_of(&last), all[1000..].to_vec());
    assert!(last.header("link").is_none(), "the last page has no next");
    // Pages at other limits concatenate to the full list.
    for limit in [1, 7, 333, 1005, 10_000] {
        assert_eq!(collect_pages(&n, &entry, limit).await, all, "limit {limit}");
    }
    // After a member: the members after it.
    let r = n.get(&format!("{}?after={}&limit=3", manifest_path(&entry), all[500])).await;
    assert_eq!(members_of(&r), all[501..504].to_vec());
    // Refused: a limit over 10,000 or 0, a malformed after.
    for q in ["limit=10001", "limit=0", "limit=x", "after=xyz", &format!("after={}", "A".repeat(64))] {
        let r = n.get(&format!("{}?{q}", manifest_path(&entry))).await;
        assert_eq!(r.status, 400, "{q}: {}", r.text());
    }
    assert_eq!(manifest_rows(&n), (0, 0));
}

// ---- Withdrawals (step 7.5, D125) ------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn withdrawals_before_and_after_a_receipt_rebuild_to_its_members_hash() {
    let n = guarded().await;
    seed(&n);
    let (ha, hb) = (stored_hash(&n, "a", DAY).await, stored_hash(&n, "b", DAY).await);
    // Signed before the withdrawal.
    let (before, before_b) = answer(&n, &flood_path(DAY, Some(A0))).await;
    assert_eq!(before.value().lexical(), "300");
    publish(&n, &[withdrawal_line("b", DAY, "station b decommissioned")], R2);
    let fresh = node_on(Some(n.db.clone()), |_| {}).await;
    // Signed after it: b left out.
    let (after, after_b) = answer(&fresh, &flood_path(DAY, None)).await;
    assert_eq!(after.value().lexical(), "100");
    assert_ne!(after.inputs()[0], before.inputs()[0]);
    // A withdrawal after the receipt: the member kept in its rebuild.
    let slice = slice_of(&fresh, &before.inputs()[0]).await;
    assert_eq!(full(&fresh, &before.inputs()[0]).await, manifest_bytes(&slice, &[ha.clone(), hb]));
    // A withdrawal before the receipt: the member left out.
    assert_eq!(full(&fresh, &after.inputs()[0]).await, manifest_bytes(&slice, &[ha]));
    verifies_recomputed(&fresh, &before_b, "the receipt from before the withdrawal").await;
    verifies_recomputed(&fresh, &after_b, "the receipt from after it").await;
    assert_eq!(manifest_rows(&n), (0, 0));
}

// ---- G6: keys_for_receipt --------------------------------------------------------------------

const OTHER: &str = "gauges.example.org";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn verify_finds_the_keys_of_slice_members_with_no_manifest_held() {
    // The members are signed by another host whose key document the store holds: POST /verify
    // gathers it through the members (keys_for_receipt), rebuilt.
    let n = node(|c| c.namespaces = vec![HOST.to_string(), OTHER.to_string()]).await;
    let okey = key_for(OTHER);
    bl(|| n.store.put_namespace(OTHER, &key_doc(OTHER, &okey), BIND_TX).unwrap());
    let gauge = format!("{OTHER}/gauge");
    n.stored(&gauge, &flood_sig());
    for (i, v) in [("2016-11-20", "5"), ("2016-11-21", "6")] {
        n.put_value_signed(&okey, &gauge, &args(&[("d", d(i))]), common::n(v), T1);
    }
    n.formula(&format!("{HOST}/gauges_total"), &format!("sum({gauge}(d: *))"), &empty_sig());
    let (r, b) = answer(&n, &format!("/cell/{HOST}/gauges_total")).await;
    assert_eq!(r.value().lexical(), "11");
    assert_eq!(manifest_rows(&n), (0, 0));
    let fresh = node_on(Some(n.db.clone()), |c| c.namespaces = vec![HOST.to_string(), OTHER.to_string()]).await;
    verifies_recomputed(&fresh, &b, "a receipt over another host's members").await;
}

// ---- G7: what was stored before the patch ----------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn receipts_and_manifests_stored_before_the_patch_still_verify() {
    // A store written as 0.5.4 writes it (every manifest stored), then served at the default
    // configuration: the held manifests are served and the receipts verify, unchanged.
    let old = storing().await;
    seed(&old);
    let (flood, fb) = answer(&old, &flood_path(DAY, None)).await;
    let (alld, ab) = answer(&old, &format!("/cell/{ALLD}")).await;
    let before = manifest_rows(&old);
    assert_eq!(before.0, 2);
    let held: Vec<Vec<u8>> =
        [&flood, &alld].iter().map(|r| bl(|| old.store.manifest(&r.inputs()[0]).unwrap().into_owned())).collect();
    let new = node_on(Some(old.db.clone()), |_| {}).await;
    assert_eq!(full(&new, &flood.inputs()[0]).await, held[0]);
    assert_eq!(full(&new, &alld.inputs()[0]).await, held[1]);
    verifies_recomputed(&new, &fb, "flood").await;
    verifies_recomputed(&new, &ab, "alldouble").await;
    assert_eq!(answer(&new, &flood_path(DAY, None)).await.1, fb, "the stored version stands");
    assert_eq!(manifest_rows(&old), before, "nothing added to or removed from the manifest tables");
}

// ---- The flood timeline (coordinator, 26 Nov) ------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn flood_timeline_reads_between_and_after_a_backfill() {
    let n = guarded().await;
    seed(&n);
    // T0: computed and stored with stations a and b.
    let (r0, b0) = answer(&n, &flood_path(DAY, Some(A0))).await;
    assert_eq!(r0.value().lexical(), "300");
    // T1: station c's November 2016 readings recorded. Now nobody has read flood since.
    publish(&n, &run2(), R2);
    let m = node_on(Some(n.db.clone()), |_| {}).await;
    // 1. As of between T0 and T1: the T0 version.
    assert_eq!(answer(&m, &flood_path(DAY, Some(A1))).await.1, b0, "case 1");
    // 2. As of between T1 and now, before any recompute: the new station counts.
    let (r2, b2) = answer(&m, &flood_path(DAY, Some(A2))).await;
    assert_eq!(r2.value().lexical(), "350", "case 2: not the T0 version");
    assert_ne!(r2.inputs()[0], r0.inputs()[0]);
    // 3. A plain read now: the new answer (case 2's version, nothing changed since).
    let (r3, _) = answer(&m, &flood_path(DAY, None)).await;
    assert_eq!(r3.value().lexical(), "350", "case 3");
    assert_eq!(r3.inputs()[0], r2.inputs()[0]);
    // After it, case 1 still answers the T0 version.
    let again = node_on(Some(n.db.clone()), |_| {}).await;
    assert_eq!(answer(&again, &flood_path(DAY, Some(A1))).await.1, b0, "case 1 after case 3");
    // 4. Both verify at level recomputed, each manifest rebuilt as of its own asof.
    assert_eq!(Manifest::from_json(&full(&again, &r0.inputs()[0]).await).unwrap().members().len(), 2);
    assert_eq!(Manifest::from_json(&full(&again, &r2.inputs()[0]).await).unwrap().members().len(), 3);
    verifies_recomputed(&again, &b0, "the T0 receipt").await;
    verifies_recomputed(&again, &b2, "the new receipt").await;
    assert_eq!(manifest_rows(&n), (0, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn flood_a_plain_read_first_writes_a_new_version_and_row() {
    let n = guarded().await;
    seed(&n);
    let (r0, b0) = answer(&n, &flood_path(DAY, Some(A0))).await;
    publish(&n, &run2(), R2);
    let (versions, rows) = (count(&n, "computed_versions"), count(&n, "member_sets"));
    let m = node_on(Some(n.db.clone()), |_| {}).await;
    let (r3, b3) = answer(&m, &flood_path(DAY, None)).await;
    assert_eq!(r3.value().lexical(), "350");
    assert!(count(&n, "computed_versions") > versions, "a new computed version");
    assert!(count(&n, "member_sets") > rows, "a new member-set row");
    let row = bl(|| m.store.member_set(&r3.inputs()[0]).unwrap().expect("its row"));
    assert_ne!(row.members_hash, bl(|| m.store.member_set(&r0.inputs()[0]).unwrap().unwrap().members_hash));
    assert_eq!(answer(&m, &flood_path(DAY, Some(A1))).await.1, b0, "case 1 still the T0 version");
    verifies_recomputed(&m, &b0, "the T0 receipt").await;
    verifies_recomputed(&m, &b3, "the new receipt").await;
}

// ---- The paths that store cells: a new member must be seen, whatever its tx ----------------------

/// T0's flood, then `add` records station z's reading for DAY (value 7) by some path, then a fresh
/// mesh reads flood now: 307, whatever tx the path recorded.
async fn new_member_through(add: impl FnOnce(&Node)) -> (Receipt, Receipt) {
    let n = guarded().await;
    seed(&n);
    let (r0, _) = answer(&n, &flood_path(DAY, Some(A0))).await;
    assert_eq!(r0.value().lexical(), "300");
    add(&n);
    let m = node_on(Some(n.db.clone()), |_| {}).await;
    let (now, _) = answer(&m, &flood_path(DAY, None)).await;
    (r0, now)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn publish_stored_with_the_runs_tx_adds_a_member_that_is_seen() {
    let (_, now) = new_member_through(|n| publish(n, &[level_line("z", DAY, "7", None)], R2)).await;
    assert_eq!(now.value().lexical(), "307");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn publish_stored_with_an_older_tx_of_its_own_adds_a_member_that_is_seen() {
    // A line with its own `tx` (and asof), older than T0's members (R1): the store keeps that tx.
    let (_, now) = new_member_through(|n| publish(n, &[level_line("z", DAY, "7", Some(OLD))], R2)).await;
    assert_eq!(now.value().lexical(), "307", "the member is new to this store, whatever its tx");
}

/// A bundle of NS with LEVEL and one receipt of level(z, DAY) = 7 recorded at `tx`.
fn bundle_with(tx: &str) -> Vec<u8> {
    let desc = datumesh_core::FunctionDescriptor::from_json(
        format!(r#"{{"code":{{"body":"","lang":"stored"}},"inputs":[],"signature":{}}}"#, level_sig()).as_bytes(),
    )
    .unwrap();
    let fn_hash = desc.hash();
    let cell = datumesh_core::Cell {
        ns: NS.into(),
        fn_name: "level".into(),
        fn_hash: fn_hash.clone(),
        args: args(&[("d", d(DAY)), ("station", s("z"))]),
    };
    let r = ReceiptBuilder::new()
        .cell(cell)
        .value(common::n("7"))
        .valid(Default::default())
        .asof(tx)
        .tx(tx)
        .issued(tx)
        .publisher(NS)
        .signer_host(HOST)
        .licence("CC0-1.0")
        .summary("Synthetic test data.")
        .sign(&key_for(HOST))
        .unwrap();
    datumesh_http::bundle::build(
        NS,
        vec![desc.canonical()],
        vec![(LEVEL.to_string(), fn_hash)],
        vec![receipt_json(&r)],
        &seed_for(HOST),
    )
    .unwrap()
}

fn import(n: &Node, bytes: Vec<u8>) {
    let owned: Vec<String> = Vec::new();
    bl(|| datumesh_http::bundle::import_signed(&*n.store, HOST, &owned, &FixedClock(MESH_NOW), &bytes))
        .unwrap_or_else(|e| panic!("import refused: {e:?}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn import_signed_with_a_new_tx_adds_a_member_that_is_seen() {
    // `import-signed` and `POST /bundles` share bundle::import_signed's writes.
    let (_, now) = new_member_through(|n| import(n, bundle_with(R2))).await;
    assert_eq!(now.value().lexical(), "307");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn import_signed_with_an_older_tx_adds_a_member_that_is_seen() {
    // The bundle's receipt keeps the tx it was signed with, older than T0's members: the store
    // records no time of its own, so "the slice's latest change" must not be the greatest member tx
    // alone, or this member is never read.
    let (_, now) = new_member_through(|n| import(n, bundle_with(OLD))).await;
    assert_eq!(now.value().lexical(), "307", "the member is new to this store, whatever its tx");
}
