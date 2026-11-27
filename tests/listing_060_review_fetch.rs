//! http 0.6.0, the review round (REVIEW-listing-060.md; the coordinator's decisions of 27 Nov at the end of
//! CONTRACT-http-listing.md): the fetch and apply side. Written before the fixes; the implementer never edits them.
//!
//! * B2: the catalogue's `next` is followed only when it is relative or on `https://<host>`; never to another origin and
//!   never `http://`; such a page does not drop the host's listings.
//! * B3: one name the store would refuse does not fail the run: the run is `Ok`, the other names are listed, the host's row
//!   is written; a 429 on `/fn/<name>?format=json` does not fail the run either.
//! * B4 (decided): a listing may create subjects in this mesh's catalogue, at most 64 per listed host; over that the names
//!   are cut (in name order) and reported as a cut (`Cut::Names`). A name in a subject that exists here creates none.
//! * C1: a cut while reading the catalogue (time or bytes), after the documents verified, is a cut, not `Unreachable`.
//! * C2 (decided): describe 404 drops that name; 429, 5xx or a network error counts as unreachable for that name (it stays
//!   listed); re-listing a dropped name is a new listing (new `listed_at`); a 404 on the catalogue's first page is not an
//!   empty catalogue.
//! * Smaller 2: a name or hash from the other mesh is checked before it goes into a URL. Smaller 3: only cells of
//!   `<host>/catalog/subject/source` count. Smaller 4: duplicate hosts. Smaller 6: instants compared as instants. Smaller 7:
//!   a 5xx on one name's request is not the whole run unreachable.

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/listing060.rs"]
mod listing060;

use std::time::Duration;

use catworld::*;
use common::lite::*;
use common::*;
use crworld::cell_line;
use datumesh_core::json::{self, Value};
use datumesh_http::listing::{self, Cut, Fetch, Limits, Outcome};
use datumesh_store::{Store, SubjectKeys};
use listing060::*;

fn slice_path() -> String {
    format!("/slice/{SRC}/catalog/subject/source")
}

fn listed(lite: &Lite) -> Vec<String> {
    lite.store.listings_of(SRC).unwrap().into_iter().map(|l| l.name).collect()
}

// ---- B2 ------------------------------------------------------------------------------------------------------------------

/// A `next` to another origin, or an `http://` one, is never fetched, and the page does not drop what is listed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b2_a_next_link_off_the_hosts_origin_is_not_followed() {
    let src = source("r060-ssrf", &[]).await;
    let other = source("r060-ssrf-other", &[]).await;
    let lite = listing_lite("r060-ssrf");
    assert_eq!(run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await.added, [bank_rate()]);
    for next in [
        format!("{}/internal/admin?x=1", other.proxy.base),
        "https://other.example/internal/admin?x=2".to_string(),
        format!("http://{SRC}/slice/{SRC}/catalog/subject/source?x=3"),
    ] {
        other.proxy.clear_log();
        src.proxy.clear_log();
        let page = format!(r#"{{"members":[],"next":"{next}"}}"#);
        src.proxy.override_path(&slice_path(), 200, page.into_bytes());
        let a = run_at(&lite, &src.proxy.base, Limits::default(), RUN2).await;
        assert_eq!(other.proxy.log(), Vec::<String>::new(), "{next}: no request to another origin");
        assert!(
            !src.proxy.log().iter().any(|p| p.contains("internal")),
            "{next}: not re-rooted either: {:?}",
            src.proxy.log()
        );
        assert!(src.proxy.log().iter().filter(|p| p.starts_with("/slice/")).count() <= 1, "{next}: not followed");
        assert!(a.dropped.is_empty(), "{next}: a refused page drops nothing: {a:?}");
        assert_eq!(listed(&lite), [bank_rate()], "{next}");
    }
}

// ---- B3 ------------------------------------------------------------------------------------------------------------------

/// Names the store would refuse are refused one by one: the run is Ok, the good name is listed, the host's row written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b3_one_refused_name_does_not_fail_the_run() {
    let lite = listing_lite("r060-b3");
    let good = name_of(&bank_rate(), "a", vec![gb()]);
    let mut long_key =
        name_of(&format!("{SRC}/long_key"), "b", vec![SubjectKeys::new("finance", "rate", &"k".repeat(40), "gb")]);
    long_key.subjects.sort();
    let mut long_label = name_of(&format!("{SRC}/long_label"), "c", vec![gb()]);
    long_label.label = "L".repeat(121);
    let mut many = name_of(&format!("{SRC}/many_subjects"), "d", vec![]);
    many.subjects = (0..257).map(|i| SubjectKeys::new("finance", "rate", "m", &format!("r{i:04}"))).collect();
    let bad_name = name_of(&format!("{SRC}/Bad Name"), "e", vec![gb()]);
    let mut no_desc = name_of(&format!("{SRC}/no_description"), "f", vec![gb()]);
    no_desc.description = String::new();
    let f = fetched(vec![good, long_key, long_label, many, bad_name, no_desc]);
    let a = listing::apply(&*lite.store, SRC, &f, RUN1).unwrap_or_else(|e| panic!("the run fails as a whole: {e}"));
    assert!(a.added.contains(&bank_rate()), "{a:?}");
    for n in ["long_key", "long_label", "many_subjects", "Bad Name"] {
        assert_eq!(lite.store.listing(&format!("{SRC}/{n}")).unwrap(), None, "{n} is refused");
    }
    if let Some(l) = lite.store.listing(&format!("{SRC}/no_description")).unwrap() {
        assert!(!l.description.is_empty(), "a listed name has a description (a fallback)");
    }
    let h = lite.store.listed_host(SRC).unwrap().expect("the host's row is written");
    assert_eq!(h.last_seen.as_deref(), Some(DAY1));
}

/// A 429 on `/fn/<name>?format=json` (label and description) fails neither the first run nor a later one; a name listed
/// before stays listed and the host's last-seen day moves.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b3_a_rate_limited_function_page_does_not_fail_the_run() {
    let src = source("r060-429fn", &[]).await;
    let lite = listing_lite("r060-429fn");
    assert_eq!(run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await.added, [bank_rate()]);
    src.proxy.override_path(&format!("/fn/{}", bank_rate()), 429, br#"{"outcome":"rate_limited"}"#.to_vec());
    let a = run_at(&lite, &src.proxy.base, Limits::default(), RUN2).await;
    assert!(a.dropped.is_empty(), "{a:?}");
    assert_eq!(listed(&lite), [bank_rate()]);
    assert_eq!(lite.store.listed_host(SRC).unwrap().unwrap().last_seen.as_deref(), Some(DAY2));
    let fresh = listing_lite("r060-429fn-fresh");
    let store = fresh.store.clone();
    let origin = src.proxy.base.clone();
    let r = tokio::task::spawn_blocking(move || listing::run_host(&*store, SRC, &origin, &Limits::default(), RUN1))
        .await
        .unwrap();
    assert!(r.is_ok(), "a first run with the page rate-limited: {r:?}");
    assert!(fresh.store.listed_host(SRC).unwrap().is_some(), "the host's row is written");
}

// ---- B4: at most 64 subjects created per listed host ----------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b4_a_host_creates_at_most_64_subjects_here() {
    let lite = listing_lite("r060-b4");
    let mut names: Vec<_> = (0..70)
        .map(|i| {
            name_of(
                &format!("{SRC}/r{i:02}"),
                &format!("r{i}"),
                vec![SubjectKeys::new("finance", "rate", "interest_rate", &format!("x{i:02}"))],
            )
        })
        .collect();
    // a name in GB (the relay's subject, which exists here) creates no subject; it sorts last
    names.push(name_of(&format!("{SRC}/zz_in_gb"), "z", vec![gb()]));
    let a = apply(&lite, &fetched(names), RUN1);
    assert_eq!((a.outcome, a.cut), (Outcome::Cut, Some(Cut::Names)), "{a:?}");
    let got = listed(&lite);
    let want: Vec<String> = (0..64).map(|i| format!("{SRC}/r{i:02}")).chain([format!("{SRC}/zz_in_gb")]).collect();
    assert_eq!(got, want, "the first 64 subject-creating names by name, and the name in an existing subject");
}

// ---- C1 ------------------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c1_a_slow_catalogue_is_a_time_cut() {
    let src = source("r060-c1-time", &[]).await;
    let lite = listing_lite("r060-c1-time");
    run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await;
    src.proxy.delay("/slice/", 3_000);
    let lim = Limits { time: Duration::from_millis(1_000), ..Limits::default() };
    let f = fetch_at(&src.proxy.base, lim.clone(), RUN2).await;
    assert!(matches!(f, Fetch::Fetched { cut: Some(Cut::Time), .. }), "{f:?}");
    let a = run_at(&lite, &src.proxy.base, lim, RUN2).await;
    assert_eq!((a.outcome, a.cut), (Outcome::Cut, Some(Cut::Time)));
    assert_eq!(listed(&lite), [bank_rate()], "nothing drops on a cut");
    let h = lite.store.listed_host(SRC).unwrap().unwrap();
    assert_eq!((h.last_seen.as_deref(), h.unreachable_since.as_deref()), (Some(DAY2), None), "the host answered");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c1_a_catalogue_over_the_byte_limit_is_a_bytes_cut() {
    let src = source("r060-c1-bytes", &[]).await;
    let big = format!(r#"{{"members":[],"pad":"{}"}}"#, "x".repeat(200_000));
    src.proxy.override_path(&slice_path(), 200, big.into_bytes());
    let f = fetch_at(&src.proxy.base, Limits { bytes: 100_000, ..Limits::default() }, RUN1).await;
    assert!(matches!(f, Fetch::Fetched { cut: Some(Cut::Bytes), .. }), "{f:?}");
}

// ---- C2 ------------------------------------------------------------------------------------------------------------------

/// describe 404: that name drops; listed again later, it is a new listing (its `listed_at` is the new run's).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2_describe_404_drops_the_name_and_a_return_is_a_new_listing() {
    let src = source("r060-c2-404", &[]).await;
    let lite = listing_lite("r060-c2-404");
    run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await;
    src.proxy.override_path(&format!("/describe/{}", bank_rate()), 404, br#"{"outcome":"not_covered"}"#.to_vec());
    let a = run_at(&lite, &src.proxy.base, Limits::default(), RUN2).await;
    assert_eq!(a.dropped, [bank_rate()], "{a:?}");
    let back = source("r060-c2-back", &[]).await; // the same source again, answering
    let a = run_at(&lite, &back.proxy.base, Limits::default(), RUN3).await;
    assert_eq!(a.added, [bank_rate()]);
    assert_eq!(lite.store.listing(&bank_rate()).unwrap().unwrap().listed_at, RUN3);
}

/// describe 429 or 5xx: unreachable for that name; it stays listed, with its listed_at.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2_describe_429_or_5xx_keeps_the_name() {
    for status in [429u16, 503] {
        let src = source(&format!("r060-c2-{status}"), &[]).await;
        let lite = listing_lite(&format!("r060-c2-{status}"));
        run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await;
        src.proxy.override_path(&format!("/describe/{}", bank_rate()), status, br#"{"outcome":"error"}"#.to_vec());
        let a = run_at(&lite, &src.proxy.base, Limits::default(), RUN2).await;
        assert!(a.dropped.is_empty(), "{status}: {a:?}");
        let l = lite.store.listing(&bank_rate()).unwrap().expect("still listed");
        assert_eq!(l.listed_at, RUN1, "{status}");
    }
}

/// A 404 on the catalogue's first page is not an empty catalogue (that would drop every listing with no cut).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2_a_missing_catalogue_is_not_an_empty_one() {
    let src = source("r060-c2-cat404", &[]).await;
    src.proxy.override_path(&slice_path(), 404, br#"{"outcome":"not_covered"}"#.to_vec());
    let f = fetch_at(&src.proxy.base, Limits::default(), RUN1).await;
    assert!(!matches!(f, Fetch::Fetched { cut: None, .. }), "a 404 catalogue read as complete and empty: {f:?}");
}

// ---- smaller issues --------------------------------------------------------------------------------------------------------

/// Smaller 2: a function hash from describe is checked (64 hex digits) before it goes into a URL.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_hash_from_the_other_mesh_is_checked_before_a_request() {
    let src = source("r060-s2", &[]).await;
    let body = format!(r#"{{"fn":"{}","fn_hash":"../../internal?x=","kind":"function"}}"#, bank_rate());
    src.proxy.override_path(&format!("/describe/{}", bank_rate()), 200, body.into_bytes());
    let _ = fetch_at(&src.proxy.base, Limits::default(), RUN1).await;
    let log = src.proxy.log();
    assert!(!log.iter().any(|p| p.contains("..") || p.contains("internal")), "{log:?}");
    assert!(
        log.iter()
            .filter_map(|p| p.strip_prefix("/descriptor/"))
            .all(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())),
        "{log:?}"
    );
}

/// Smaller 3: only cells of `<host>/catalog/subject/source` name sources. A signed cell of another function of the host
/// with the same arguments, whose value is a name, lists nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_only_source_cells_name_sources() {
    let fake = format!("{SRC}/fake_catalogue");
    let sig = r#"{"args":{"domain":{"type":"string","valid":false},"measure":{"type":"string","valid":false},"region":{"type":"string","valid":false},"thing":{"type":"string","valid":false}},"order":["domain","thing","measure","region"],"returns":"string"}"#;
    let other = format!("{SRC}/other_rate");
    let more = vec![
        common::pub031::function_line(&fake, sig, None),
        cell_line(
            &fake,
            &[
                ("domain", "string", "finance"),
                ("measure", "string", "interest_rate"),
                ("region", "string", "gb"),
                ("thing", "string", "rate"),
            ],
            ("string", &other),
        ),
        common::pub031::function_line(&other, &date_sig(), None),
    ];
    let src = source("r060-s3", &more).await;
    let receipt_of = |v: &serde_json::Value| v["receipt"].clone();
    let real = src
        .mesh
        .get(&format!(
            "/cell/{}?asof={}",
            common::world::call_path(&format!("{SRC}/catalog/subject(finance, rate, interest_rate, gb).source(1)")),
            common::readpath::MESH_NOW
        ))
        .await;
    let fake_call = format!(r#"{fake}(domain: "finance", thing: "rate", measure: "interest_rate", region: "gb")"#);
    let forged = src
        .mesh
        .get(&format!("/cell/{}?asof={}", common::world::call_path(&fake_call), common::readpath::MESH_NOW))
        .await;
    let page = serde_json::json!({"members": [{"receipt": receipt_of(&real)}, {"receipt": receipt_of(&forged)}]});
    src.proxy.override_path(&slice_path(), 200, serde_json::to_vec(&page).unwrap());
    let f = fetch_at(&src.proxy.base, Limits::default(), RUN1).await;
    let names = fetched_names(&f);
    assert!(!names.contains(&other), "a cell of another function named a source: {names:?}");
}

/// Smaller 4: a host named twice counts once (or the setting is refused).
#[test]
fn s4_duplicate_hosts() {
    let vars = [("DATUMESH_HOST", HOST), ("DATUMESH_LISTED_MESHES", "a.example,b.example,a.example")];
    match datumesh_http::Config::from_vars(|k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())) {
        Ok(c) => assert_eq!(c.listed_meshes, ["a.example", "b.example"], "deduplicated"),
        Err(e) => assert!(e.to_string().contains("DATUMESH_LISTED_MESHES"), "{e}"),
    }
    let many: String = std::iter::repeat_n("a.example", 70).collect::<Vec<_>>().join(",");
    let vars = [("DATUMESH_HOST", HOST), ("DATUMESH_LISTED_MESHES", many.as_str())];
    if let Ok(c) = datumesh_http::Config::from_vars(|k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()))
    {
        assert_eq!(c.listed_meshes, ["a.example"], "one host, named 70 times");
    }
}

fn keydoc_with(valid_from: &str, valid_to: Option<&str>) -> Vec<u8> {
    let doc = json::parse(&key_doc(SRC, &key_for(SRC))).unwrap();
    let Value::Object(mut top) = doc else { panic!() };
    for (k, v) in top.iter_mut() {
        if k == "keys" {
            let Value::Array(keys) = v else { panic!() };
            for key in keys.iter_mut() {
                let Value::Object(m) = key else { panic!() };
                for (mk, mv) in m.iter_mut() {
                    if mk == "valid_from" {
                        *mv = Value::String(valid_from.into());
                    }
                }
                if let Some(t) = valid_to {
                    m.push(("valid_to".into(), Value::String(t.into())));
                }
            }
        }
    }
    let d = Value::Object(top).to_jcs();
    datumesh_core::key_document_from_response(SRC, 200, Some(&d)).expect("a well-formed key document");
    d
}

/// Smaller 6: key validity compares instants, not strings. `now` written without a fraction (`...:00Z`) is before a key that
/// starts half a second later, and inside one that ends half a second later.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_key_validity_compares_instants() {
    let src = source("r060-s6", &[]).await;
    src.proxy.override_path("/.well-known/datumesh.json", 200, keydoc_with("2026-11-16T00:00:00.500000Z", None));
    let f = fetch_at(&src.proxy.base, Limits::default(), "2026-11-16T00:00:00Z").await;
    assert!(matches!(f, Fetch::KeyDocFailed { .. }), "not yet valid: {f:?}");
    let src = source("r060-s6b", &[]).await;
    src.proxy.override_path(
        "/.well-known/datumesh.json",
        200,
        keydoc_with(K_FROM, Some("2026-11-16T00:00:00.500000Z")),
    );
    let f = fetch_at(&src.proxy.base, Limits::default(), "2026-11-16T00:00:00Z").await;
    assert!(matches!(f, Fetch::Fetched { .. }), "still valid: {f:?}");
}

/// Smaller 7: a 5xx on one name's request is a cut (or unreachable for that name), not the whole run unreachable: the other
/// names verified are kept, and the name stays listed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_a_5xx_on_one_name_is_not_the_whole_run_unreachable() {
    let more = src_fn_lines(&format!("{SRC}/a_rate"), "A rate", "A rate of the source.", gb_facets());
    let src = source("r060-s7", &more).await;
    let lite = listing_lite("r060-s7");
    run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await;
    assert_eq!(listed(&lite).len(), 2);
    let h = src_hash(&src, &bank_rate());
    src.proxy.override_path(&format!("/descriptor/{h}"), 503, br#"{"error":"down"}"#.to_vec());
    let f = fetch_at(&src.proxy.base, Limits::default(), RUN2).await;
    assert!(fetched_names(&f).contains(&format!("{SRC}/a_rate")), "{f:?}");
    let a = run_at(&lite, &src.proxy.base, Limits::default(), RUN2).await;
    assert_ne!(a.outcome, Outcome::Unreachable);
    assert!(a.dropped.is_empty(), "{a:?}");
    assert_eq!(listed(&lite).len(), 2);
}
