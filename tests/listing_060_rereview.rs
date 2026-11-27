//! http 0.6.0, the re-review round (REVIEW-listing-060.md "Re-review"; decided 27 Nov at the end of
//! CONTRACT-http-listing.md). Written before the fixes; the implementer never edits them. Needs store 0.7.2.
//!
//! * N1: a catalogue that cannot be read (404, 5xx, no answer on its first page) makes the run `Unreachable` for the host:
//!   `last_seen` is not written and the host's 7-day clock runs. A name not reached in a run whose catalogue was read gets
//!   its own clock (`Store::listing_unreached`), set at the first such run, cleared when it answers, written only on a
//!   change; the name drops at 7 days, not at 6 days 23 hours. The re-check: in a run cut for any reason.
//! * N2: a request looks up only the words whose host is listed, each once (`datumesh_listing_lookups_total`).
//! * N3: the 64 subjects a host's listings create are counted across runs, an offered name's old ones not (re-check).
//! * N4: a cut in the middle of the catalogue keeps a name's earlier subjects (and so the ranks).
//! * N6: `/x/interval` refuses a listed name with `E_LISTED_ELSEWHERE`.
//! * C3 (config): a listed host that `DATUMESH_PUBLISH_DOMAINS` allows, or `DATUMESH_CROSS_MESH_HOSTS` names, is refused
//!   at start.
//!
//! (N5, the start-up drop's log line, is in `listing_060_rereview_log.rs`: it installs a global subscriber.)

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/listing060.rs"]
mod listing060;

use catjob::*;
use catworld::*;
use common::lite::*;
use common::readpath::*;
use common::*;
use datumesh_http::listing::{Cut, Fetch, Limits, Outcome};
use datumesh_store::{Store, SubjectKeys};
use listing060::*;
use serde_json::{json, Value};

const D: &str = "2026-03-02";

fn slice_path() -> String {
    format!("/slice/{SRC}/catalog/subject/source")
}

fn listed(lite: &Lite) -> Vec<String> {
    lite.store.listings_of(SRC).unwrap().into_iter().map(|l| l.name).collect()
}

// ---- N1: an unreadable catalogue is the host unreachable --------------------------------------------------------------

/// The catalogue's first page answers 404 (or 503) for ever: each run is `Unreachable`; `last_seen` stays the day of the last
/// readable run; the listing stays at 6 days 23 hours and drops at 7 days.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn n1_an_unreadable_catalogue_is_the_host_unreachable() {
    for status in [404u16, 503] {
        let src = source(&format!("r3-n1cat-{status}"), &[]).await;
        let lite = listing_lite(&format!("r3-n1cat-{status}"));
        assert_eq!(run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await.added, [bank_rate()]);
        src.proxy.override_path(&slice_path(), status, br#"{"outcome":"error"}"#.to_vec());
        let a = run_at(&lite, &src.proxy.base, Limits::default(), "2026-11-17T00:00:00.000000Z").await;
        assert_eq!(a.outcome, Outcome::Unreachable, "{status}: {a:?}");
        let h = lite.store.listed_host(SRC).unwrap().unwrap();
        assert_eq!(h.last_seen.as_deref(), Some(DAY1), "{status}: not seen");
        assert_eq!(h.unreachable_since.as_deref(), Some("2026-11-17T00:00:00.000000Z"), "{status}");
        let a = run_at(&lite, &src.proxy.base, Limits::default(), "2026-11-23T23:00:00.000000Z").await;
        assert!(a.dropped.is_empty(), "{status}: kept at 6 d 23 h: {a:?}");
        let a = run_at(&lite, &src.proxy.base, Limits::default(), "2026-11-24T00:00:00.000000Z").await;
        assert_eq!(a.dropped, [bank_rate()], "{status}: dropped at 7 days");
    }
}

/// The same with no answer at all from the catalogue (the documents still answer): a page that never comes within the time.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn n1_a_catalogue_that_never_answers_is_the_host_unreachable() {
    let src = source("r3-n1slow", &[]).await;
    let lite = listing_lite("r3-n1slow");
    run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await;
    src.proxy.delay("/slice/", 3_000);
    let lim = Limits { time: std::time::Duration::from_millis(800), ..Limits::default() };
    let f = fetch_at(&src.proxy.base, lim, RUN2).await;
    assert!(matches!(f, Fetch::Unreachable { .. }), "nothing of the catalogue read: {f:?}");
}

// ---- N1: a name's own clock ---------------------------------------------------------------------------------------------

/// describe answers 503 from day 2 on, while the catalogue reads: the name gets its own clock at the first such run, stays
/// at 6 days 23 hours, drops at 7 days.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn n1_a_name_not_reached_drops_after_seven_days_of_its_own() {
    const FIRST: &str = "2026-11-17T01:00:00.000000Z";
    let src = source("r3-n1name", &[]).await;
    let lite = listing_lite("r3-n1name");
    run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await;
    src.proxy.override_path(&format!("/describe/{}", bank_rate()), 503, br#"{"outcome":"error"}"#.to_vec());
    let a = run_at(&lite, &src.proxy.base, Limits::default(), FIRST).await;
    assert!(a.dropped.is_empty(), "{a:?}");
    assert_eq!(lite.store.listing_unreached(&bank_rate()).unwrap().as_deref(), Some(FIRST), "its clock starts");
    for at in ["2026-11-20T00:00:00.000000Z", "2026-11-24T00:00:00.000000Z"] {
        let a = run_at(&lite, &src.proxy.base, Limits::default(), at).await;
        assert!(a.dropped.is_empty(), "kept at {at}: {a:?}");
        assert_eq!(lite.store.listing_unreached(&bank_rate()).unwrap().as_deref(), Some(FIRST), "the first stays");
    }
    let a = run_at(&lite, &src.proxy.base, Limits::default(), "2026-11-24T01:00:00.000000Z").await;
    assert_eq!(a.dropped, [bank_rate()], "dropped at 7 days of its own");
}

/// A name that answers again has its clock cleared; a second unreached run the same day writes nothing (quiet).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn n1_the_clock_clears_and_a_quiet_run_writes_nothing() {
    let src = source("r3-n1quiet", &[]).await;
    let lite = listing_lite("r3-n1quiet");
    run_at(&lite, &src.proxy.base, Limits::default(), RUN1).await;
    src.proxy.override_path(&format!("/describe/{}", bank_rate()), 503, br#"{"outcome":"error"}"#.to_vec());
    let a = run_at(&lite, &src.proxy.base, Limits::default(), RUN2).await;
    assert!(a.writes >= 1, "the clock is written: {a:?}");
    let stamps = catalogue_stamps(&lite);
    let a = run_at(&lite, &src.proxy.base, Limits::default(), RUN3).await;
    assert_eq!(a.writes, 0, "the same day, still unreached: nothing written: {a:?}");
    assert_eq!(catalogue_stamps(&lite), stamps);
    let back = source("r3-n1quiet-back", &[]).await;
    run_at(&lite, &back.proxy.base, Limits::default(), "2026-11-18T00:00:00.000000Z").await;
    assert_eq!(lite.store.listing_unreached(&bank_rate()).unwrap(), None, "cleared when it answers");
    assert_eq!(listed(&lite), [bank_rate()]);
}

// ---- N2: lookups only for listed hosts' words, each once -----------------------------------------------------------------

async fn lookups(m: &Served) -> f64 {
    metric(&page(m, "/metrics").await.text(), "datumesh_listing_lookups_total")
}

async fn post(m: &Served, path: &str, body: Vec<u8>) -> u16 {
    m.http
        .post(format!("{}{path}", m.base))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn n2_only_words_of_a_listed_host_are_looked_up_each_once() {
    let w = listed_world("r3-n2").await;
    let (m, _src) = serve_listed(w, &[]).await;
    let l0 = lookups(&m).await;
    // names of hosts that are not listed: no lookup
    let _ = m.raw(&format!("/cell/{}?date={D}", relay())).await;
    let _ = m.raw(&format!("/cell/third.example/rates/x?date={D}")).await;
    let _ = m.raw(&format!("/slice/{HOST}/catalog/subject/exists?domain=*&measure=*&region=*&thing=*")).await;
    assert_eq!(lookups(&m).await, l0, "no lookup for names of unlisted hosts");
    // 20,000 words of unlisted hosts in one /whatif: none
    let words: Vec<String> = (0..20_000).map(|i| format!("x{i}/y")).collect();
    let body = serde_json::to_vec(&json!({"cell": words.join(" + "), "overrides": {}})).unwrap();
    let _ = post(&m, "/whatif", body).await;
    assert_eq!(lookups(&m).await, l0, "no lookup for 20,000 unlisted words");
    // 20,000 words, two distinct of the listed host: at most two lookups
    let words: Vec<String> =
        (0..20_000).map(|i| if i % 2 == 0 { bank_rate() } else { format!("{SRC}/other_rate") }).collect();
    let body = serde_json::to_vec(&json!({"cell": words.join(" + "), "overrides": {}})).unwrap();
    let _ = post(&m, "/whatif", body).await;
    let l1 = lookups(&m).await;
    assert!(l1 - l0 <= 2.0 && l1 > l0, "each distinct listed word once: {}", l1 - l0);
    // /cells: 100 items of one listed name, one request
    let items: Vec<String> = (0..100).map(|_| format!("{}(date: {D})", bank_rate())).collect();
    let _ = post(&m, "/cells", serde_json::to_vec(&items).unwrap()).await;
    let l2 = lookups(&m).await;
    assert!(l2 - l1 <= 2.0, "one request, one distinct listed name: {}", l2 - l1);
}

// ---- N3: the subjects cap across runs ------------------------------------------------------------------------------------

/// The first run lists 64 names in 64 new subjects (70 offered: cut). A second run, itself cut, offers 70 other names in other
/// new subjects: the 64 kept from the first run count, so the host holds at most 64 created subjects.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn n3_the_subjects_cap_counts_across_runs() {
    let lite = listing_lite("r3-n3");
    let offer = |prefix: &str| -> Vec<_> {
        (0..70)
            .map(|i| {
                name_of(
                    &format!("{SRC}/{prefix}{i:02}"),
                    &format!("{prefix}{i}"),
                    vec![SubjectKeys::new("finance", "rate", "interest_rate", &format!("{prefix}{i:02}"))],
                )
            })
            .collect()
    };
    let a = apply(&lite, &fetched(offer("r")), RUN1);
    assert_eq!(a.cut, Some(Cut::Names));
    assert_eq!(listed(&lite).len(), 64);
    let second = Fetch::Fetched { key_id: src_key_id(), names: offer("q"), refused: vec![], cut: Some(Cut::Time) };
    apply(&lite, &second, RUN1B);
    let all = lite.store.listings_of(SRC).unwrap();
    let subjects: std::collections::BTreeSet<SubjectKeys> = all.iter().flat_map(|l| l.subjects.clone()).collect();
    assert!(subjects.len() <= 64, "the host holds {} created subjects", subjects.len());
}

// ---- N4: a mid-catalogue cut keeps a name's earlier subjects -----------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn n4_a_catalogue_cut_keeps_a_names_subjects_and_ranks() {
    let ea = SubjectKeys::new("finance", "rate", "interest_rate", "ea");
    let lite = listing_lite("r3-n4");
    let full = name_of(&bank_rate(), "a", vec![ea.clone(), gb()]);
    apply(&lite, &fetched(vec![full]), RUN1);
    build(&lite, T3);
    let m = serve(lite, &kept(SRC)).await;
    assert_eq!(sources_at(&m, GB).await, [bank_rate(), relay()]);
    assert_eq!(sources_at(&m, ("finance", "rate", "interest_rate", "ea")).await, [bank_rate()]);
    for cut in [Cut::Bytes, Cut::Time, Cut::Unreached] {
        // the cut came before the page with `ea`: the name was seen in GB only
        let partial = Fetch::Fetched {
            key_id: src_key_id(),
            names: vec![name_of(&bank_rate(), "a", vec![gb()])],
            refused: vec![],
            cut: Some(cut),
        };
        let a = apply(&m.lite, &partial, RUN1B);
        assert!(a.changed.is_empty(), "{cut:?}: {a:?}");
        assert_eq!(m.lite.store.listing(&bank_rate()).unwrap().unwrap().subjects, [ea.clone(), gb()], "{cut:?}");
    }
    assert_eq!(build(&m.lite, T4).cells, 0, "the cut runs change no rank");
}

// ---- the re-check (CONTRACT "Decided 27 Nov (d)") ------------------------------------------------------------------------

/// N1, every cut: a held name that a run cut for names, bytes or time neither offered nor refused has its own clock too: set
/// at that run, kept at 6 days 23 hours, dropped at 7 days. The name that was offered has none.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn n1_a_name_not_reached_by_any_cut_has_its_own_clock() {
    for (i, cut) in [Cut::Names, Cut::Bytes, Cut::Time].into_iter().enumerate() {
        let lite = listing_lite(&format!("r4-n1cut{i}"));
        let other = format!("{SRC}/other_rate");
        let both = vec![name_of(&bank_rate(), "a", vec![gb()]), name_of(&other, "b", vec![gb()])];
        apply(&lite, &fetched(both), RUN1);
        assert_eq!(listed(&lite).len(), 2);
        let partial = Fetch::Fetched {
            key_id: src_key_id(),
            names: vec![name_of(&bank_rate(), "a", vec![gb()])],
            refused: vec![],
            cut: Some(cut),
        };
        let a = apply(&lite, &partial, RUN2);
        assert!(a.dropped.is_empty(), "{cut:?}: {a:?}");
        assert_eq!(lite.store.listing_unreached(&other).unwrap().as_deref(), Some(RUN2), "{cut:?}: its clock starts");
        assert_eq!(lite.store.listing_unreached(&bank_rate()).unwrap(), None, "{cut:?}: the offered name has no clock");
        let a = apply(&lite, &partial, "2026-11-24T00:59:59.000000Z");
        assert!(a.dropped.is_empty(), "{cut:?}: kept before 7 days: {a:?}");
        assert_eq!(lite.store.listing_unreached(&other).unwrap().as_deref(), Some(RUN2), "{cut:?}: the first stays");
        let a = apply(&lite, &partial, "2026-11-24T01:00:00.000000Z");
        assert_eq!(a.dropped, [other.as_str()], "{cut:?}: dropped at 7 days of its own");
        assert_eq!(listed(&lite), [bank_rate()], "{cut:?}");
    }
}

/// N3, no double count: a host holding 64 created subjects whose next full run offers the same 64 names, each in a new
/// subject in place of its old one, is not cut: every name moves to its new subject, and the host still holds 64.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn n3_a_name_that_changes_its_subjects_is_not_counted_twice() {
    let lite = listing_lite("r4-n3move");
    let offer = |region: &str| -> Vec<_> {
        (0..64)
            .map(|i| {
                name_of(
                    &format!("{SRC}/m{i:02}"),
                    &format!("m{i}"),
                    vec![SubjectKeys::new("finance", "rate", "interest_rate", &format!("{region}{i:02}"))],
                )
            })
            .collect()
    };
    let a = apply(&lite, &fetched(offer("r")), RUN1);
    assert_eq!(a.cut, None, "64 is within the cap: {a:?}");
    assert_eq!(listed(&lite).len(), 64);
    let a = apply(&lite, &fetched(offer("q")), RUN2);
    assert_eq!(a.cut, None, "the old subjects of offered names do not count: {a:?}");
    assert_eq!(a.changed.len(), 64, "{a:?}");
    let all = lite.store.listings_of(SRC).unwrap();
    assert!(
        all.iter().all(|l| l.subjects.len() == 1 && l.subjects[0].region.starts_with('q')),
        "moved to the new subjects"
    );
    let subjects: std::collections::BTreeSet<SubjectKeys> = all.iter().flat_map(|l| l.subjects.clone()).collect();
    assert_eq!(subjects.len(), 64);
}

// ---- N6 ------------------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn n6_x_interval_refuses_a_listed_name() {
    let w = listed_world("r3-n6").await;
    let (m, src) = serve_listed(w, &[]).await;
    src.proxy.clear_log();
    let (status, body) = m.raw(&format!("/x/interval/{}?from=2026-03-01&to=2026-03-05", bank_rate())).await;
    let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let text = String::from_utf8_lossy(&body);
    assert_eq!(status, 404, "{text}");
    assert_eq!(v["error"], "E_LISTED_ELSEWHERE", "{text}");
    assert_eq!(v["mesh"], SRC_ORIGIN, "{text}");
    assert_eq!(src.proxy.log(), Vec::<String>::new());
}

// ---- C3 (config) ---------------------------------------------------------------------------------------------------------

fn from(vars: &[(&str, &str)]) -> Result<datumesh_http::Config, String> {
    let mut env: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    env.push(("DATUMESH_HOST".into(), HOST.into()));
    datumesh_http::Config::from_vars(move |k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()))
        .map_err(|e| e.to_string())
}

#[test]
fn c3_a_listed_host_that_may_publish_or_is_resolved_is_refused_at_start() {
    for (k, v) in
        [("DATUMESH_PUBLISH_DOMAINS", SRC), ("DATUMESH_PUBLISH_DOMAINS", "*"), ("DATUMESH_CROSS_MESH_HOSTS", SRC)]
    {
        match from(&[("DATUMESH_LISTED_MESHES", SRC), (k, v)]) {
            Ok(_) => panic!("{k}={v}: a listed host that is also {k} is accepted"),
            Err(e) => assert!(e.contains("DATUMESH_LISTED_MESHES"), "{k}={v}: {e}"),
        }
    }
    from(&[("DATUMESH_LISTED_MESHES", SRC), ("DATUMESH_PUBLISH_DOMAINS", "other.example")])
        .expect("another host may publish");
    from(&[("DATUMESH_LISTED_MESHES", SRC), ("DATUMESH_CROSS_MESH_HOSTS", "other.example")])
        .expect("another host resolved");
    from(&[("DATUMESH_PUBLISH_DOMAINS", SRC)]).expect("no list: unchanged");
}
