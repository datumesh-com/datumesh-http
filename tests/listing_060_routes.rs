//! http 0.6.0, the listing release (CONTRACT-http-listing.md L1, L5 to L11): two meshes in process, the listing mesh
//! (`mesh-test.datumesh.com`, with a relay of the source in GB) listing `source.example` through `DATUMESH_PEERS` (a
//! recording proxy in front of the source). Failing on the test commit: the run is `todo!()` there, and no route knows a
//! listed name. The tests marked "unchanged" pin 0.5.x behaviour and pass on the stubs too.
//!
//! * `E_LISTED_ELSEWHERE` on every read route (`/cell`, `/slice`, `/series`, `/versions`, `/explain`) and on `/x/calc`:
//!   404, `not_covered`, `mesh` = `https://source.example`; no request leaves the mesh for it;
//! * `describe` and `/fn` for a listed name, with the day last seen;
//! * find: the listed source first in its subject (rank 1) though the relay matches exactly; what a listed result carries;
//! * the relay's function page and its takeover page point to rank 1, with the day last seen;
//! * the rule's text (L11);
//! * no hosting by the back door (L9): a listed host's bundle is refused; no cell, receipt or binding row for a listed name;
//! * an empty list: no request leaves the mesh (unchanged), metrics report 0;
//! * the listing thread runs every listed host and `/metrics` reports it (L10); `check.sh` reports the count.

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/listing060.rs"]
mod listing060;

use std::time::{Duration, Instant};

use catjob::*;
use catworld::*;
use common::lite::*;
use common::readpath::*;
use common::world::call_path;
use common::*;
use datumesh_store::Store;
use listing060::*;
use serde_json::{json, Value};

const D: &str = "2026-03-02";

#[track_caller]
fn assert_listed_elsewhere(status: u16, v: &Value, body: &str, what: &str) {
    assert_eq!(status, 404, "{what}: {body}");
    assert_eq!(v["outcome"], "not_covered", "{what}: {body}");
    assert_eq!(v["error"], "E_LISTED_ELSEWHERE", "{what}: {body}");
    assert_eq!(v["mesh"], SRC_ORIGIN, "{what}: {body}");
}

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

/// L6, L9: every read route and `/x/calc` answer `E_LISTED_ELSEWHERE` for a listed name, and none sends a request to the
/// source (its cells are never fetched); afterwards there is no cell, receipt, version or binding row of the name.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l6_e_listed_elsewhere_on_every_read_route_and_on_calc() {
    let w = listed_world("l060-routes").await;
    let before = untouched(&w.lite);
    let (m, src) = serve_listed(w, &[]).await;
    src.proxy.clear_log();
    let l = bank_rate();
    let paths = [
        format!("/cell/{l}?date={D}"),
        format!("/cell/{l}?date={D}&asof={MESH_NOW}"),
        format!("/cell/{}", call_path(&format!("{l}(date: {D})"))),
        format!("/slice/{l}?date=*"),
        format!("/series/{l}"),
        format!("/series/{l}/2026-03.csv"),
        format!("/versions/{l}?date={D}"),
        format!("/explain/{l}?date={D}"),
    ];
    for p in &paths {
        let (status, v, text) = get_json(&m, p).await;
        assert_listed_elsewhere(status, &v, &text, p);
    }
    for f in [
        format!("{l}(date: {D}) * 2"),
        format!("sum({l}(date: *))"),
        format!("{}(date: {D}) + {l}(date: {D})", relay()),
    ] {
        let (status, v, text) = post_json(&m, "/x/calc", serde_json::to_vec(&json!({ "formula": f })).unwrap()).await;
        assert_listed_elsewhere(status, &v, &text, &format!("/x/calc {f}"));
    }
    assert_eq!(src.proxy.log(), Vec::<String>::new(), "no request leaves the mesh for a listed name");
    assert_eq!(untouched(&m.lite), before, "no cell, receipt, version or binding row");
    assert_eq!(hosted_rows(&m.lite, SRC), 0);
    // the relay still answers (control)
    let (status, v, text) = get_json(&m, &format!("/cell/{}?date={D}", relay())).await;
    assert_eq!((status, v["outcome"].as_str()), (200, Some("ok")), "{text}");
}

/// L6: `describe` gives the listed descriptor with `listed: true`, the mesh, and the day last seen (L8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l6_describe_a_listed_name() {
    let w = listed_world("l060-describe").await;
    let hash = src_hash(&w.src, &bank_rate());
    let (m, _src) = serve_listed(w, &[]).await;
    let (status, v, text) = get_json(&m, &format!("/describe/{}", bank_rate())).await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(v["listed"], true, "{text}");
    assert_eq!(v["mesh"], SRC_ORIGIN, "{text}");
    assert_eq!(v["last_seen"], DAY1, "{text}");
    assert_eq!(v["fn_hash"], hash.as_str(), "{text}");
    assert_eq!(v["publisher"], SRC, "{text}");
    assert_eq!(v["address"], format!("{SRC_ORIGIN}/fn/{}", bank_rate()), "{text}");
    assert_eq!(v["description"], DESCRIPTION, "{text}");
    // the descriptor is answered by hash here (descriptors take any hash)
    let (status, body) = m.raw(&format!("/descriptor/{hash}")).await;
    assert_eq!(status, 200);
    assert_eq!(datumesh_core::function_hash(&body).unwrap(), hash);
}

/// L6: `/fn/<name>` is a short page with the description, the mesh, a link to the name's page there, and the day last seen;
/// as JSON it carries the same members as `describe`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l6_the_function_page_of_a_listed_name() {
    let w = listed_world("l060-fnpage").await;
    let (m, _src) = serve_listed(w, &[]).await;
    let p = page(&m, &format!("/fn/{}", bank_rate())).await;
    assert_eq!(p.status, 200, "{}", p.text());
    assert!(p.header("content-type").unwrap_or_default().starts_with("text/html"), "{:?}", p.header("content-type"));
    let html = p.text();
    let text = plain(&html);
    assert!(text.contains(DESCRIPTION), "the description: {text}");
    assert!(text.contains(SRC_ORIGIN) || text.contains(SRC), "the mesh: {text}");
    assert!(text.contains(DAY1), "the day last seen: {text}");
    let there = format!("{SRC_ORIGIN}/fn/{}", bank_rate());
    assert!(hrefs(&html).contains(&there), "a link to the name's page on its own mesh: {:?}", hrefs(&html));
    assert!(!html.contains("<script"), "no script");
    let (status, v, t) = get_json(&m, &format!("/fn/{}?format=json", bank_rate())).await;
    assert_eq!(status, 200, "{t}");
    assert_eq!(
        (v["listed"].clone(), v["mesh"].clone(), v["last_seen"].clone()),
        (json!(true), json!(SRC_ORIGIN), json!(DAY1)),
        "{t}"
    );
    assert_eq!(v["address"], there, "{t}");
}

/// L5 (answer 6): find orders results that share a subject by the catalogue rank. The relay is an exact match of
/// `bank rate` (its label) and the listed source a text match: today's key puts the relay first; the group of the subject
/// takes the relay's place and inside it the source (rank 1) comes first. A listed result carries `publisher` = its host,
/// `mesh` = its origin, `listed: true`, the day last seen, its address on its own mesh, and no `cell` or `slice` link.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l5_find_puts_the_listed_source_first_in_its_subject() {
    let w = listed_world("l060-find").await;
    let (m, src) = serve_listed(w, &[]).await;
    src.proxy.clear_log();
    let v = m.find("bank rate").await;
    let n = names(&v);
    let (i_src, i_relay) = (pos(&v, &bank_rate()), pos(&v, &relay()));
    assert!(i_src < i_relay, "rank 1 first in its subject: {n:?}");
    assert_eq!(i_relay, i_src + 1, "the group stays together: {n:?}");
    let r = result(&v, &bank_rate());
    assert_eq!(r["listed"], true, "{r}");
    assert_eq!(r["publisher"], SRC, "{r}");
    assert_eq!(r["mesh"], SRC_ORIGIN, "{r}");
    assert_eq!(r["last_seen"], DAY1, "{r}");
    assert_eq!(r["address"], format!("{SRC_ORIGIN}/fn/{}", bank_rate()), "{r}");
    assert!(r.get("cell").is_none() && r.get("slice").is_none(), "no cell or slice link on this mesh: {r}");
    let rel = result(&v, &relay());
    assert!(rel.get("listed").is_none(), "a result of this mesh carries no listed member: {rel}");
    assert_eq!(src.proxy.log(), Vec::<String>::new(), "find asks the source nothing");
}

/// L7, L8: when a listed function is rank 1 in a relay's subject, the relay's function page and its takeover page say so
/// and link to it, with the day it was last seen; the stand-in sentence stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l7_the_relay_page_and_the_takeover_page_point_to_rank_1() {
    let w = listed_world("l060-relaypage").await;
    let (m, _src) = serve_listed(w, &[]).await;
    let links_to_listed = |html: &str| hrefs(html).iter().any(|h| h.ends_with(&format!("/fn/{}", bank_rate())));
    // the note reads the catalogue pages' build (the subjects' places): wait for its first run
    assert_eq!(until_built(&m, "/catalog").await.status, 200);
    let p = page(&m, &format!("/fn/{}", relay())).await;
    assert_eq!(p.status, 200, "{}", p.text());
    let html = p.text();
    assert!(links_to_listed(&html), "the relay page links to rank 1: {:?}", hrefs(&html));
    assert!(plain(&html).contains(DAY1), "the day last seen: {}", plain(&html));
    assert!(plain(&html).contains("stand-in"), "the stand-in sentence stays: {}", plain(&html));
    let t = page(&m, &format!("/takeover/{SRC}")).await;
    assert_eq!(t.status, 200, "{}", t.text());
    assert!(links_to_listed(&t.text()), "the takeover page links to rank 1: {:?}", hrefs(&t.text()));
}

/// L11: the rule's text adds one sentence: listed names are ranked by the same rule, and a listed name is described here,
/// not served here.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l11_the_rule_text_names_listed_names() {
    let w = listed_world("l060-rule").await;
    let (m, _src) = serve_listed(w, &[]).await;
    let p = until_built(&m, "/catalog/rule").await;
    assert_eq!(p.status, 200, "{}", p.text());
    let text = plain(&p.text());
    assert!(text.contains("Listed names are ranked by the same rule"), "{text}");
    assert!(text.contains("described here, not served here"), "{text}");
}

/// L9: production's rule is unchanged: `POST /bundles` admits no listed host, even one `DATUMESH_PUBLISH_DOMAINS` allows.
/// Control: the same bundle on a mesh that does not list the host is admitted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l9_a_listed_hosts_bundle_is_refused() {
    let w = listed_world("l060-bundle").await;
    let bundle = foreign_bundle(SRC, &format!("{SRC}/data"), &format!("{SRC}/data/series_x"), None);
    let (m, src) = serve_listed(w, &[("DATUMESH_PUBLISH_DOMAINS", SRC)]).await;
    src.proxy.clear_log();
    let (status, v, text) = post_json(&m, "/bundles", bundle.clone()).await;
    assert_eq!((status, v["error"].as_str()), (403, Some("E_DOMAIN_NOT_ALLOWED")), "{text}");
    assert_eq!(m.lite.store.resolve_name(&format!("{SRC}/data/series_x"), END).unwrap(), None);
    assert_eq!(hosted_rows(&m.lite, SRC), 0);
    // control: a mesh that does not list the host admits it (its key document fetched through the peers mapping)
    let other = listing_lite("l060-bundle-control");
    let c = listing_cfg(Some(&src.proxy.base), &[("DATUMESH_PUBLISH_DOMAINS", SRC)]);
    let m2 = build_served(other, Some(c), |_| {}).await;
    let (status, _, text) = post_json(&m2, "/bundles", bundle).await;
    assert_eq!(status, 201, "control: {text}");
}

/// L1 (unchanged): with the list empty, nothing is fetched: no request leaves the mesh, though `DATUMESH_PEERS` maps the
/// host; `/metrics` reports 0 listed names.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l1_an_empty_list_sends_no_request() {
    let src = source("l060-empty", &[]).await;
    let lite = listing_lite("l060-empty");
    build(&lite, T3);
    let c = listing_cfg(Some(&src.proxy.base), &[("DATUMESH_LISTING_INTERVAL_SECS", "1")]);
    let m = build_served(lite, Some(c), |_| {}).await;
    let _ = m.find("bank rate").await;
    let _ = get_json(&m, &format!("/describe/{}", relay())).await;
    let _ = page(&m, &format!("/fn/{}", relay())).await;
    let _ = until_built(&m, "/catalog").await;
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert_eq!(src.proxy.log(), Vec::<String>::new(), "no request leaves the mesh");
    assert_eq!(m.lite.store.listing_count().unwrap(), 0);
    let metrics = page(&m, "/metrics").await.text();
    assert_eq!(metric(&metrics, "datumesh_listed_names_total"), 0.0, "{metrics}");
}

/// L2, L10: the mesh's listing thread runs each listed host (at start, then every interval) and `/metrics` reports, per
/// host, the names listed, the last run's outcome and its time; `datumesh_listed_names_total` is the count for check.sh.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l10_the_listing_thread_runs_and_metrics_report_it() {
    let src = source("l060-thread", &[]).await;
    let lite = listing_lite("l060-thread");
    let c = listing_cfg(Some(&src.proxy.base), &[("DATUMESH_LISTED_MESHES", SRC)]);
    let m = build_served(lite, Some(c), |_| {}).await;
    let names_line = format!("datumesh_listed_names{{host=\"{SRC}\"}}");
    let end = Instant::now() + Duration::from_secs(30);
    let metrics = loop {
        let t = page(&m, "/metrics").await.text();
        if metric(&t, &names_line) == 1.0 || Instant::now() > end {
            break t;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(metric(&metrics, &names_line), 1.0, "{metrics}");
    assert_eq!(
        metric(&metrics, &format!("datumesh_listing_outcome{{host=\"{SRC}\",outcome=\"ok\"}}")),
        1.0,
        "{metrics}"
    );
    for o in ["key_document_failed", "mesh_document_failed", "cut", "unreachable"] {
        let line = format!("datumesh_listing_outcome{{host=\"{SRC}\",outcome=\"{o}\"}}");
        let v = metric(&metrics, &line);
        assert!(v.is_nan() || v <= 0.0, "only the last outcome is 1: {line} in {metrics}");
    }
    let at = metric(&metrics, &format!("datumesh_listing_last_run_seconds{{host=\"{SRC}\"}}"));
    assert!(at > 1.7e9, "the run's time in unix seconds: {at} in {metrics}");
    assert_eq!(metric(&metrics, "datumesh_listed_names_total"), 1.0, "{metrics}");
    let (status, v, text) = get_json(&m, &format!("/describe/{}", bank_rate())).await;
    assert_eq!((status, v["listed"].clone()), (200, json!(true)), "{text}");
    assert!(src.proxy.log().iter().any(|p| p == "/.well-known/datumesh.json"), "fetched through the peers mapping");
}

/// L10: check.sh on production reports the count of listed names (0 while the list is empty).
#[test]
fn l10_check_sh_reports_the_count_of_listed_names() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("deploy/prod/check.sh");
    let s = std::fs::read_to_string(&p).unwrap();
    assert!(s.contains("datumesh_listed_names_total"), "check.sh reads the metric");
    assert!(s.contains("listed names"), "check.sh's line says `listed names <n>`");
}
