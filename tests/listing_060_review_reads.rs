//! http 0.6.0, the review round (REVIEW-listing-060.md; the coordinator's decisions of 27 Nov at the end of
//! CONTRACT-http-listing.md): the mesh side. Written before the fixes; the implementer never edits them. The listing mesh
//! keeps the default cross-mesh settings (on, every host), as the review's scenarios do.
//!
//! * B1: the listed-name refusal covers every route: HEAD, `POST /cells` (per item), `/x/versions`, an alias; with no request
//!   to the listed mesh and no row written. `Foreign` refuses a listed host. A store error fails closed (503).
//! * B5 (decided): with `DATUMESH_LISTED_MESHES` empty, find's order is 0.5.x's; the grouped rank order applies only to a
//!   group that holds a listed result.
//! * C3 (http side): a name bound here through a bundle is not taken over by a listing of its host.
//! * C4: the relay page and the takeover page name a listed sub-domain's rank-1 function of the relay's subject, and not a
//!   listed name that is rank 1 in another subject.
//! * C5 (decided): at start, the rows of a host the setting does not name are dropped.
//! * C6: a run that fails still records its outcome (`error`) and time in `/metrics`; with the store failing,
//!   `datumesh_listed_names_total` is absent rather than 0 (smaller 9). B3: a run with a rate-limited function page records
//!   its outcome.
//! * Smaller 1: with the list empty, no request looks up a listing (`datumesh_listing_lookups_total` stays 0).
//! * Smaller 9: the takeover page does not invite a listed domain to `POST /bundles`.

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/listing060.rs"]
mod listing060;

use std::sync::Arc;
use std::time::{Duration, Instant};

use catjob::*;
use catworld::*;
use common::lite::*;
use common::readpath::*;
use common::world::call_path;
use common::*;
use crworld::{catalog_line_of, publish};
use datumesh_http::Mesh;
use datumesh_store::{Store, SubjectKeys};
use listing060::*;
use serde_json::{json, Value};

const D: &str = "2026-03-02";

async fn get_json(m: &Served, path: &str) -> (u16, Value, String) {
    let (status, body) = m.raw(path).await;
    let text = String::from_utf8_lossy(&body).into_owned();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null), text)
}

async fn post_json(m: &Served, path: &str, body: Vec<u8>) -> (u16, Value, String) {
    let r = m
        .http
        .post(format!("{}{path}", m.base))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    let b = r.bytes().await.unwrap().to_vec();
    let text = String::from_utf8_lossy(&b).into_owned();
    (status, serde_json::from_slice(&b).unwrap_or(Value::Null), text)
}

#[track_caller]
fn assert_listed_elsewhere(status: u16, v: &Value, body: &str, what: &str) {
    assert_eq!(status, 404, "{what}: {body}");
    assert_eq!(v["error"], "E_LISTED_ELSEWHERE", "{what}: {body}");
    assert_eq!(v["mesh"], SRC_ORIGIN, "{what}: {body}");
}

async fn metrics(m: &Served) -> String {
    page(m, "/metrics").await.text()
}

// ---- B1 ------------------------------------------------------------------------------------------------------------------

/// HEAD on every read route is refused like GET: 404, no request to the listed mesh, no binding row.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b1_head_is_refused_like_get() {
    let w = listed_world("r060-head").await;
    let before = untouched(&w.lite);
    let (m, src) = serve_listed(w, &[]).await;
    src.proxy.clear_log();
    let l = bank_rate();
    for p in [
        format!("/cell/{l}?date={D}"),
        format!("/slice/{l}?date=*"),
        format!("/versions/{l}?date={D}"),
        format!("/explain/{l}?date={D}"),
        format!("/series/{l}"),
        format!("/x/versions/{l}?date={D}"),
    ] {
        let r = m.http.head(format!("{}{p}", m.base)).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 404, "HEAD {p}");
    }
    assert_eq!(src.proxy.log(), Vec::<String>::new(), "no request leaves the mesh");
    assert_eq!(untouched(&m.lite), before, "no row written");
    assert_eq!(hosted_rows(&m.lite, SRC), 0);
}

/// `POST /cells`: the listed item answers `E_LISTED_ELSEWHERE`, the others as usual; nothing is fetched.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b1_post_cells_refuses_the_listed_item() {
    let w = listed_world("r060-cells").await;
    let (m, src) = serve_listed(w, &[]).await;
    src.proxy.clear_log();
    let body =
        serde_json::to_vec(&json!([format!("{}(date: {D})", bank_rate()), format!("{}(date: {D})", relay())])).unwrap();
    let (status, v, text) = post_json(&m, "/cells", body).await;
    assert_eq!(status, 200, "{text}");
    let items = v.as_array().unwrap_or_else(|| panic!("an array: {text}"));
    // each item is {"envelope": ..., "status": n}
    assert_eq!(items[0]["status"], 404, "{text}");
    assert_eq!(items[0]["envelope"]["error"], "E_LISTED_ELSEWHERE", "{text}");
    assert_eq!(items[0]["envelope"]["mesh"], SRC_ORIGIN, "{text}");
    assert_eq!(items[1]["envelope"]["outcome"], "ok", "{text}");
    assert_eq!(src.proxy.log(), Vec::<String>::new());
    assert_eq!(hosted_rows(&m.lite, SRC), 0);
}

/// `/x/versions` and a read through an alias are refused too.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b1_x_versions_and_an_alias_are_refused() {
    let w = listed_world("r060-xv").await;
    let mut c = listing_cfg(
        Some(&w.src.proxy.base),
        &[("DATUMESH_LISTED_MESHES", SRC), ("DATUMESH_LISTING_FIRST_DELAY_SECS", "3600")],
    );
    c.aliases.insert("br".into(), bank_rate());
    let src = w.src;
    let m = build_served(w.lite, Some(c), |_| {}).await;
    src.proxy.clear_log();
    let (status, v, text) = get_json(&m, &format!("/x/versions/{}?date={D}", bank_rate())).await;
    assert_listed_elsewhere(status, &v, &text, "/x/versions");
    let (status, v, text) = get_json(&m, &format!("/cell/{}", call_path(&format!("@br(date: {D})")))).await;
    assert_listed_elsewhere(status, &v, &text, "an alias of a listed name");
    assert_eq!(src.proxy.log(), Vec::<String>::new());
    assert_eq!(hosted_rows(&m.lite, SRC), 0);
}

/// `Foreign` refuses a listed host: a listed name is not foreign here, and resolving it asks the listed mesh nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b1_foreign_refuses_a_listed_host() {
    let src = source("r060-foreign", &[]).await;
    let lite = listing_lite("r060-foreign");
    let c = listing_cfg(
        Some(&src.proxy.base),
        &[("DATUMESH_LISTED_MESHES", SRC), ("DATUMESH_LISTING_FIRST_DELAY_SECS", "3600")],
    );
    let shared: Arc<dyn Store + Send + Sync> = lite.store.clone();
    let key = lite.key.clone();
    let mesh = tokio::task::spawn_blocking(move || {
        Mesh::builder(c, shared).signing_key(key).clock(Arc::new(FixedClock(MESH_NOW))).build().unwrap()
    })
    .await
    .unwrap();
    src.proxy.clear_log();
    let m2 = mesh.clone();
    let (foreign, ensured) = tokio::task::spawn_blocking(move || {
        (m2.foreign().is_foreign(&bank_rate()), m2.foreign().ensure_name(&bank_rate()).is_ok())
    })
    .await
    .unwrap();
    assert!(!foreign, "a listed host is not resolved across meshes");
    assert!(!ensured, "ensure_name of a listed name does not succeed");
    assert_eq!(src.proxy.log(), Vec::<String>::new(), "nothing asked of the listed mesh");
    assert!(
        tokio::task::spawn_blocking(move || mesh.foreign().is_foreign("third.example/x")).await.unwrap(),
        "control"
    );
}

// ---- C6, B3 (metrics), smaller 9 (metrics on a store error), B1 (fail closed) ---------------------------------------------

async fn until_metric(m: &Served, line: &str, want: f64, secs: u64) -> String {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        let t = metrics(m).await;
        if metric(&t, line) == want || Instant::now() > end {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// A run that fails (the store's listing tables are gone) still records its outcome `error` and its time; the total line is
/// absent, not 0; a read of the listed host's name fails closed (503) and asks the listed mesh nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c6_a_failed_run_is_recorded_and_reads_fail_closed() {
    let src = source("r060-c6", &[]).await;
    let lite = listing_lite("r060-c6");
    let c =
        listing_cfg(Some(&src.proxy.base), &[("DATUMESH_LISTED_MESHES", SRC), ("DATUMESH_LISTING_INTERVAL_SECS", "1")]);
    let m = build_served(lite, Some(c), |_| {}).await;
    let ok = format!("datumesh_listing_outcome{{host=\"{SRC}\",outcome=\"ok\"}}");
    let t = until_metric(&m, &ok, 1.0, 30).await;
    assert_eq!(metric(&t, &ok), 1.0, "{t}");
    let at0 = metric(&t, &format!("datumesh_listing_last_run_seconds{{host=\"{SRC}\"}}"));
    let conn = rusqlite::Connection::open(m.lite.path("store.db")).unwrap();
    conn.execute_batch("DROP TABLE listings; DROP TABLE listed_hosts;").unwrap();
    let err = format!("datumesh_listing_outcome{{host=\"{SRC}\",outcome=\"error\"}}");
    let t = until_metric(&m, &err, 1.0, 30).await;
    assert_eq!(metric(&t, &err), 1.0, "a failed run is recorded: {t}");
    let v = metric(&t, &ok);
    assert!(v.is_nan() || v <= 0.0, "only the last outcome: {t}");
    assert!(metric(&t, &format!("datumesh_listing_last_run_seconds{{host=\"{SRC}\"}}")) >= at0, "{t}");
    assert!(!t.lines().any(|l| l.starts_with("datumesh_listed_names_total ")), "no total on a store error: {t}");
    src.proxy.clear_log();
    let (status, _, text) = get_json(&m, &format!("/cell/{}?date={D}", bank_rate())).await;
    assert_eq!(status, 503, "fail closed: {text}");
    assert!(!src.proxy.log().iter().any(|p| p.contains("bank_rate")), "{:?}", src.proxy.log());
}

/// B3: a run whose function page is rate-limited from the start records its outcome in `/metrics`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b3_a_rate_limited_page_still_records_the_run() {
    let src = source("r060-b3m", &[]).await;
    src.proxy.override_path(&format!("/fn/{}", bank_rate()), 429, br#"{"outcome":"rate_limited"}"#.to_vec());
    let lite = listing_lite("r060-b3m");
    let c = listing_cfg(Some(&src.proxy.base), &[("DATUMESH_LISTED_MESHES", SRC)]);
    let m = build_served(lite, Some(c), |_| {}).await;
    let line = format!("datumesh_listing_last_run_seconds{{host=\"{SRC}\"}}");
    let end = Instant::now() + Duration::from_secs(30);
    let t = loop {
        let t = metrics(&m).await;
        if metric(&t, &line) > 0.0 || Instant::now() > end {
            break t;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert!(metric(&t, &line) > 1.7e9, "{t}");
    assert!(m.lite.store.listed_host(SRC).unwrap().is_some(), "the host's row is written");
}

// ---- B5 ------------------------------------------------------------------------------------------------------------------

/// A relay (rank 1 of GB, a text match) and a local name of the same subject (rank 2, an exact match): 0.5.x orders by score,
/// so the local name comes first; with the list empty that is the order, and with a list but no listed result in the group.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b5_without_a_listed_result_the_order_is_0_5_x() {
    for vars in [vec![], kept(SRC).to_vec()] {
        let lite = Lite::new("r060-b5");
        let local = host_name("finance/bank_rate_local");
        let mut lines = relay_lines(&relay(), gb_facets(), "Relay of the bank rate");
        lines.push(fn_line(&local));
        lines.push(catalog_line_of(
            &local,
            "function",
            json!({"facets": gb_facets(), "label": "Bank Rate", "description": "A local rate."}),
        ));
        publish(&lite, "b5.jsonl", &lines, T_RELAY).unwrap_or_else(|e| panic!("{e}"));
        build(&lite, T3);
        let m = serve(lite, &vars).await;
        assert_eq!(sources_at(&m, GB).await, [relay(), local.clone()], "the catalogue ranks the relay first");
        assert_eq!(until_built(&m, "/catalog").await.status, 200);
        let v = m.find("bank rate").await;
        assert!(pos(&v, &local) < pos(&v, &relay()), "{vars:?}: by score, as 0.5.x: {:?}", names(&v));
    }
}

// ---- C3 (http) -----------------------------------------------------------------------------------------------------------

/// A name of the host already bound here (a bundle it published) is not taken over by its listing: the run does not fail,
/// the name is not listed, and describe answers it as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c3_a_name_bound_here_is_not_taken_over() {
    let lite = listing_lite("r060-c3");
    let bound = format!("{SRC}/data/bank_rate");
    import_foreign(&lite, SRC, &foreign_bundle(SRC, &format!("{SRC}/data"), &bound, Some(gb_facets())), T1)
        .unwrap_or_else(|e| panic!("{e}"));
    let before = lite
        .store
        .catalog_entry_ext(datumesh_store::CatalogKind::Function, &bound)
        .unwrap()
        .expect("the bundle's entry");
    assert_eq!(before.entry.source, datumesh_store::CatalogSource::Publisher);
    let a = datumesh_http::listing::apply(&*lite.store, SRC, &fetched(vec![name_of(&bound, "a", vec![gb()])]), RUN1)
        .unwrap_or_else(|e| panic!("the run fails: {e}"));
    assert!(a.added.is_empty(), "{a:?}");
    assert_eq!(lite.store.listing(&bound).unwrap(), None);
    let e = lite.store.catalog_entry_ext(datumesh_store::CatalogKind::Function, &bound).unwrap().unwrap();
    assert_eq!(e, before, "the publisher's entry is untouched");
    let m = serve(lite, &kept(SRC)).await;
    let (status, v, text) = get_json(&m, &format!("/describe/{bound}")).await;
    assert_eq!(status, 200, "{text}");
    assert!(v.get("listed").is_none(), "{text}");
}

// ---- C4 ------------------------------------------------------------------------------------------------------------------

fn links_to(html: &str, name: &str) -> bool {
    hrefs(html).iter().any(|h| h.ends_with(&format!("/fn/{name}")))
}

/// A listed sub-domain of the relay's domain that is rank 1 of the relay's subject is named on both pages.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c4_a_listed_sub_domain_at_rank_1_is_named() {
    let lite = listing_lite("r060-c4-sub");
    let sub = format!("stats.{SRC}");
    let name = format!("{sub}/bank_rate");
    put_listing(&lite, &sub, &name, vec![gb()], RUN1);
    build(&lite, T3);
    let m = serve(lite, &kept(&sub)).await;
    assert_eq!(sources_at(&m, GB).await, [name.clone(), relay()]);
    let _ = until_built(&m, "/catalog").await;
    let p = page(&m, &format!("/fn/{}", relay())).await;
    assert!(links_to(&p.text(), &name), "the relay page: {:?}", hrefs(&p.text()));
    let t = page(&m, &format!("/takeover/{SRC}")).await;
    assert!(links_to(&t.text(), &name), "the takeover page: {:?}", hrefs(&t.text()));
}

/// A listed name of the domain that is rank 1 of another subject (not the relay's) is not announced on the relay's pages.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c4_rank_1_of_another_subject_is_not_announced() {
    let lite = listing_lite("r060-c4-other");
    let name = format!("{SRC}/euro_rate");
    put_listing(&lite, SRC, &name, vec![SubjectKeys::new("finance", "rate", "interest_rate", "ea")], RUN1);
    build(&lite, T3);
    let m = serve(lite, &kept(SRC)).await;
    assert_eq!(sources_at(&m, ("finance", "rate", "interest_rate", "ea")).await, std::slice::from_ref(&name));
    let _ = until_built(&m, "/catalog").await;
    let p = page(&m, &format!("/fn/{}", relay())).await;
    assert!(!links_to(&p.text(), &name), "the relay page: {:?}", hrefs(&p.text()));
    let t = page(&m, &format!("/takeover/{SRC}")).await;
    assert!(!links_to(&t.text(), &name), "the takeover page: {:?}", hrefs(&t.text()));
}

// ---- C5 ------------------------------------------------------------------------------------------------------------------

async fn until_gone(m: &Served, host: &str) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let gone =
            m.lite.store.listings_of(host).unwrap().is_empty() && m.lite.store.listed_host(host).unwrap().is_none();
        if gone || Instant::now() > end {
            return gone;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// At start, the rows of a host the setting does not name are dropped; a named host's rows stay.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c5_an_unlisted_hosts_rows_are_dropped_at_start() {
    let lite = listing_lite("r060-c5");
    put_listing(&lite, SRC, &bank_rate(), vec![gb()], RUN1);
    put_listing(&lite, "gone.example", "gone.example/rate", vec![gb()], RUN1);
    let m = serve(lite, &kept(SRC)).await;
    assert!(until_gone(&m, "gone.example").await, "the unlisted host's rows are dropped");
    assert_eq!(m.lite.store.listings_of(SRC).unwrap().len(), 1, "the listed host's stay");
}

/// With the list emptied, every listed host's rows are dropped at start (L1: answers as in 0.5.x).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c5_an_emptied_list_drops_every_listing_at_start() {
    let lite = listing_lite("r060-c5-empty");
    put_listing(&lite, SRC, &bank_rate(), vec![gb()], RUN1);
    let m = serve(lite, &[]).await;
    assert!(until_gone(&m, SRC).await);
    assert_eq!(m.lite.store.listing_count().unwrap(), 0);
}

// ---- smaller 1 and 9 ---------------------------------------------------------------------------------------------------------

/// Smaller 1: with the list empty, no request looks up a listing: `datumesh_listing_lookups_total` stays 0 across reads,
/// find, describe and the function page. With a listing it counts.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_an_empty_list_looks_nothing_up() {
    let lite = listing_lite("r060-s1");
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    let _ = get_json(&m, &format!("/cell/{}?date={D}", relay())).await;
    let _ = m.find("bank rate").await;
    let _ = get_json(&m, &format!("/describe/{}", relay())).await;
    let _ = page(&m, &format!("/fn/{}", relay())).await;
    let t = metrics(&m).await;
    assert_eq!(metric(&t, "datumesh_listing_lookups_total"), 0.0, "{t}");
    let w = listed_world("r060-s1-listed").await;
    let (m, _src) = serve_listed(w, &[]).await;
    let _ = get_json(&m, &format!("/cell/{}?date={D}", bank_rate())).await;
    let t = metrics(&m).await;
    assert!(metric(&t, "datumesh_listing_lookups_total") > 0.0, "{t}");
}

/// Smaller 9: the takeover page does not tell a listed domain that `POST /bundles` is open to it (bundles refuse it, L9).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_the_takeover_page_does_not_invite_a_listed_domain() {
    let w = listed_world("r060-s9").await;
    // since the re-review's C3 (decided 27 Nov) a listed host that DATUMESH_PUBLISH_DOMAINS allows is refused at start, so
    // the page is read with the default publish domains
    let (m, _src) = serve_listed(w, &[]).await;
    let t = page(&m, &format!("/takeover/{SRC}")).await;
    assert_eq!(t.status, 200);
    assert!(!plain(&t.text()).contains("POST /bundles is open to"), "{}", plain(&t.text()));
}
