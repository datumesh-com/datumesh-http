//! The 0.7 round (CONTRACT-round-0.7.md), R11: the bounded paths of the round's own new surfaces. Failing tests.
//!
//! * `extensions_of` is read with `LIMIT 101` (and a store index on `extends`, a store test): `describe` of an object lists at most
//!   100 extensions, the first 100 by name, however many functions extend its class;
//! * the `datumesh-catalog` thread reads the subjects from the `extent.keys` list in pages, never `usize::MAX` receipts (a source
//!   test: neither `catalog_build.rs` nor `catalog_view.rs` says `usize::MAX`), and its first build is not inside `Mesh::start`
//!   (right after the mesh is up, a catalogue of a few hundred subjects has not been built yet: the builds counter is 0 and the
//!   pages are `503`; the build happens on the thread). This one depends on the build being slower than the start: the world has
//!   432 subjects;
//! * `/catalog/<domain>` is paged at 200 subjects with `?after=`: the page has a link whose `after` member continues it, followed
//!   to the last page (no such link); the pages together list each subject once;
//! * caps: a facet product of more than 256 subjects in one entry is refused at publish or cut by the job (never more than 256
//!   subjects from an entry); a subject keeps at most 20 sources (`sources()` is at most 20, `source(21)` is no cell);
//! * a request's chain is at most 8 methods long (a longer one is `400`).

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/r07world.rs"]
mod r07world;

use std::collections::BTreeSet;

use catjob::*;
use catworld::*;
use common::lite::*;
use common::pub031::{function_line, T2};
use common::readpath::*;
use crworld::*;
use r07world::*;
use serde_json::json;

/// A catalogue of one entry whose facets give `a` things, `b` measures and `c` regions under `domain`.
fn product_lines(domain: &str, name: &str, a: usize, b: usize, c: usize) -> Vec<String> {
    let mk = |p: &str, n: usize| -> Vec<String> { (1..=n).map(|i| format!("{p}{i}")).collect() };
    let (t, m, r) = (mk("t", a), mk("m", b), mk("r", c));
    let f = |v: &[String]| -> Vec<String> { v.to_vec() };
    let (t, m, r) = (f(&t), f(&m), f(&r));
    let (t, m, r): (Vec<&str>, Vec<&str>, Vec<&str>) = (
        t.iter().map(String::as_str).collect(),
        m.iter().map(String::as_str).collect(),
        r.iter().map(String::as_str).collect(),
    );
    entries_lines(&[(host_name(name), fac(&[domain], &t, &m, &r, &["daily"]))])
}

async fn wide_served(label: &str) -> Served {
    let lite = Lite::new(label);
    let mut lines = product_lines("wide", "wide/f", 6, 6, 6);
    lines.extend(product_lines("small", "small/f", 1, 1, 3));
    crworld::publish(&lite, "w.jsonl", &lines, NOW).unwrap_or_else(|e| panic!("{e}"));
    let b = build(&lite, T3);
    assert_eq!(b.subjects, 219, "{b:?}");
    serve(lite, &[]).await
}

fn subject_links(body: &str) -> Vec<String> {
    hrefs(body).into_iter().map(|h| local(&h)).filter(|l| l.contains("/subject?key=")).collect()
}

fn next_link(body: &str) -> Option<String> {
    hrefs(body).into_iter().map(|h| local(&h)).find(|l| l.contains("after="))
}

// ---- /catalog/<domain> paged at 200 ----------------------------------------------------------------------------------

/// A domain of 216 subjects: the first page lists 200 and links to the next with `?after=`; the second lists the other 16 and has no
/// such link; together they list each subject once. The root counts all 216; the small domain is one page with no link.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r11_a_domain_page_is_paged_at_200_subjects() {
    let m = wide_served("r11-paging").await;
    let root = until_built(&m, "/catalog").await;
    assert_eq!(root.status, 200, "{}", root.text());
    assert!(plain(&root.text()).contains("216"), "the root counts the subjects of the domain: {}", plain(&root.text()));
    let mut seen: Vec<String> = Vec::new();
    let mut sizes = Vec::new();
    let mut at = "/catalog/wide".to_string();
    for _ in 0..4 {
        let p = page(&m, &at).await;
        assert_eq!(p.status, 200, "{at}: {}", p.text());
        let body = p.text();
        let links = subject_links(&body);
        sizes.push(links.len());
        seen.extend(links);
        match next_link(&body) {
            Some(n) => {
                assert!(n.starts_with("/catalog/wide?") && n.contains("after="), "the next page of the same domain: {n}");
                at = n;
            }
            None => break,
        }
    }
    assert_eq!(sizes, [200, 16], "pages of at most 200 subjects");
    let uniq: BTreeSet<&String> = seen.iter().collect();
    assert_eq!(uniq.len(), 216, "each subject once: {}", seen.len());
    let want: BTreeSet<String> = (1..=6)
        .flat_map(|t| {
            (1..=6).flat_map(move |me| (1..=6).map(move |r| subject_page("wide", &format!("t{t}"), &format!("m{me}"), &format!("r{r}"))))
        })
        .collect();
    assert_eq!(seen.into_iter().collect::<BTreeSet<_>>(), want, "all the subjects of the domain");
    // a small domain is one page
    let s = page(&m, "/catalog/small").await;
    assert_eq!(s.status, 200, "{}", s.text());
    assert_eq!(subject_links(&s.text()).len(), 3);
    assert!(next_link(&s.text()).is_none(), "no next page for 3 subjects");
}

/// Every page of a large domain is built by the thread, not on a request, and the root's `describe` counts them all (the thread
/// read every subject from the `extent.keys` list, however many pages that took).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r11_the_thread_reads_every_subject_of_a_large_catalogue() {
    let m = wide_served("r11-all").await;
    until_built(&m, "/catalog").await;
    let d = describe(&m, &format!("{HOST}/catalog")).await;
    assert_eq!(d.status, 200, "{}", d.text);
    let wide = d.v["domains"].as_array().unwrap().iter().find(|x| x["domain"] == "wide").unwrap_or_else(|| panic!("{}", d.text));
    assert_eq!(wide["subjects"], 216, "{}", d.text);
    assert_eq!(builds(&m, "request").await, 0);
}

/// The first build is not inside `Mesh::start`: right after the mesh is up the pages are cold and nothing has been built; the
/// build comes from the thread.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r11_the_first_build_is_not_inside_mesh_start() {
    let lite = Lite::new("r11-start");
    let mut lines = product_lines("wide", "wide/f", 6, 6, 6);
    lines.extend(product_lines("wide2", "wide2/f", 6, 6, 6));
    crworld::publish(&lite, "w.jsonl", &lines, NOW).unwrap_or_else(|e| panic!("{e}"));
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    assert_eq!(builds(&m, "background").await, 0, "nothing was built while the mesh started");
    let r = page(&m, "/catalog").await;
    assert_cold(&r, "right after the start");
    let built = until_built(&m, "/catalog").await;
    assert_eq!(built.status, 200, "{}", built.text());
    assert!(builds(&m, "background").await >= 1);
}

/// No `usize::MAX` read of receipts in the catalogue's job and view code (the extent list is read in pages).
#[test]
fn r11_no_unbounded_receipt_reads_in_the_catalogue_code() {
    for (file, src) in [
        ("src/catalog_build.rs", include_str!("../src/catalog_build.rs")),
        ("src/catalog_view.rs", include_str!("../src/catalog_view.rs")),
    ] {
        assert!(!src.contains("usize::MAX"), "{file} reads receipts without a bound");
    }
}

// ---- caps ------------------------------------------------------------------------------------------------------------

/// A facet product of 256 subjects in one entry is accepted and gives 256 subjects; one of 320 is refused at publish (the
/// refusal says 256) or cut by the job, never more than 256 subjects from the entry.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r11_an_entry_gives_at_most_256_subjects() {
    let exact = Lite::new("r11-cap-256");
    crworld::publish(&exact, "c.jsonl", &product_lines("capd", "cap/f", 8, 8, 4), NOW).unwrap_or_else(|e| panic!("256 is accepted: {e}"));
    let b = build(&exact, T3);
    assert_eq!(b.subjects, 256, "{b:?}");

    let over = Lite::new("r11-cap-320");
    match crworld::publish(&over, "c.jsonl", &product_lines("capd", "cap/f", 8, 8, 5), NOW) {
        Err(e) => assert!(e.to_string().contains("256"), "the refusal names the cap: {e}"),
        Ok(_) => {
            let b = build(&over, T3);
            assert!(b.subjects <= 256, "an entry gives at most 256 subjects: {b:?}");
        }
    }
}

/// A subject keeps at most 20 sources: 25 entries of one subject give `sources()` 20, `source(20)` the twentieth by the rank
/// order (the same instant, so by name), and `source(21)` is no cell (not a withdrawal either).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r11_a_subject_keeps_at_most_20_sources() {
    let lite = Lite::new("r11-ranks");
    let f = fac(&["rk"], &["tt"], &["mm"], &["world"], &["static"]);
    let entries: Vec<(String, serde_json::Value)> = (1..=25).map(|i| (host_name(&format!("cap/n{i:02}")), f.clone())).collect();
    crworld::publish(&lite, "r.jsonl", &entries_lines(&entries), NOW).unwrap_or_else(|e| panic!("{e}"));
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    let s = ("rk", "tt", "mm", "world");
    let n = cell(&m, &sc(s.0, s.1, s.2, s.3, "sources()")).await;
    assert_eq!((n.outcome(), n.value()), ("ok", Some("20")), "{}", n.text);
    let want: Vec<String> = (1..=20).map(|i| host_name(&format!("cap/n{i:02}"))).collect();
    assert_eq!(sources_list(&m, s).await, want);
    let over = cell(&m, &sc(s.0, s.1, s.2, s.3, "source(21)")).await;
    assert_eq!(over.outcome(), "not_covered", "{}", over.text);
    assert!(over.v.get("receipt").is_none_or(|r| r.is_null()), "{}", over.text);
}

// ---- chains ----------------------------------------------------------------------------------------------------------

/// A chain of at most 8 methods answers; one of 10 is `400`. (`Z1.successor()` is `Z1`: the chain can be any length.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r11_a_chain_is_at_most_8_methods_long() {
    let lite = Lite::new("r11-chain");
    publish(&lite, "functions.jsonl", &shop_functions(), common::pub031::BEFORE).unwrap_or_else(|e| panic!("functions: {e}"));
    let mut v = shop_classes(&lite);
    v.extend(shop_cells());
    v.push(successor_cell("Z1", &pk("Z1")));
    v.push(price_cell_of("Z1", SHOP_DAY, "5"));
    publish(&lite, "classes.jsonl", &v, NOW).unwrap_or_else(|e| panic!("classes and cells: {e}"));
    let m = build_served(lite, None, |_| {}).await;
    let chain = |links: usize| format!("{}(Z1){}.price({SHOP_DAY})", product(), ".successor()".repeat(links));
    // 8 methods: 7 successors and the price
    let ok = ask(&m, &path(&chain(7), None)).await;
    assert_eq!((ok.status, ok.outcome(), ok.value()), (200, "ok", Some("5")), "{}", ok.text);
    // 10 methods
    let long = ask(&m, &path(&chain(9), None)).await;
    assert_eq!(long.status, 400, "{}", long.text);
    assert_ne!(long.outcome(), "ok", "{}", long.text);
}

// ---- extensions ------------------------------------------------------------------------------------------------------

fn margin_sig() -> String {
    format!(
        r#"{{"args":{{"pct":{{"type":"number","unit":"1","valid":false}},"sku":{STR}}},"order":["sku","pct"],"returns":{NUM1}}}"#
    )
}

/// `describe` of an object lists at most 100 extensions, the first 100 by name, when 150 functions extend its class.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r11_the_extensions_of_an_object_are_at_most_100() {
    let lite = Lite::new("r11-ext");
    publish(&lite, "functions.jsonl", &shop_functions(), common::pub031::BEFORE).unwrap_or_else(|e| panic!("functions: {e}"));
    let mut v = shop_classes(&lite);
    v.extend(shop_cells());
    publish(&lite, "classes.jsonl", &v, NOW).unwrap_or_else(|e| panic!("classes and cells: {e}"));
    let mut lines = Vec::new();
    for i in 0..150 {
        let n = format!("{HOST}/ext/e{i:03}");
        lines.push(function_line(&n, &margin_sig(), None));
        lines.push(catalog_line_of(&n, "function", json!({"extends": product()})));
    }
    publish(&lite, "ext.jsonl", &lines, T2).unwrap_or_else(|e| panic!("extensions: {e}"));
    let m = build_served(lite, None, |_| {}).await;
    let a = ask(&m, &format!("/describe/{}", common::world::call_path(&pk("A1")))).await;
    assert_eq!(a.status, 200, "{}", a.text);
    let got: Vec<String> =
        a.v["extensions"].as_array().unwrap_or_else(|| panic!("{}", a.text)).iter().map(|e| e["fn"].as_str().unwrap().to_string()).collect();
    let want: Vec<String> = (0..100).map(|i| format!("{HOST}/ext/e{i:03}")).collect();
    assert_eq!(got, want, "the first 100 by name");
}
