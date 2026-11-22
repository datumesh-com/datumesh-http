//! The catalogue, phase 1 (CONTRACT-classes-round.md), C9 and C10: the pages for people and the addresses.
//!
//! The pages, at most three steps deep: `/catalog` (the domains, with counts), `/catalog/<domain>` (its subjects, with labels and
//! their rank-1 source), and a subject's page (its ranked sources, each linking to the existing `/fn/<name>` page of a function
//! or the class's page). They are cached with the overview rule: built by a background thread from the catalogue's cells, so a
//! cold request is `503` with `Retry-After` and `no-store`, and no work grows with the data on a request.
//!
//! C10: no mesh name, page path or link contains a facet value or a rank as a path segment that is not part of a stable name;
//! a subject's page addresses its subject by its key encoding, in the query; every link to a source is by its full name; and
//! `catalog/subject(...).source(1)` is a stable full name.
//!
//! Interfaces pinned here (the implementer follows them; the report lists them):
//! * the subject's page is `/catalog/<domain>/subject?key=<percent-encoded key encoding>`; a domain in the path that is not the
//!   key's is a 404, as is a subject that does not exist, a domain that is none, and a malformed key (400 is also accepted for
//!   that last one);
//! * a class source links to `/describe/<class>` (the class's page: the only one a class has), a function to `/fn/<name>`;
//!   hrefs may be absolute (`https://<host>/...`) or root-relative;
//! * the domains of `/catalog` are in the order of C7 (subjects descending, then name); the page shows each domain's label next
//!   to its number of subjects, and links to `/catalog/<domain>`;
//! * a page links only to the catalogue's own pages, `/fn/<name>` and `/describe/<class>` of known names, `/`, and other hosts;
//! * the knobs and metrics are those of the series pages: `DATUMESH_CATALOG_FIRST_BUILD_DELAY_SECS` (a test knob),
//!   `DATUMESH_CATALOG_REFRESH_SECS` (default 300), and `datumesh_catalog_builds_total{by="background"|"request"}` (a line that
//!   is absent counts as 0).
//! * the pages are HTML (`text/html`).

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use catjob::*;
use catworld::*;
use common::lite::*;
use common::readpath::*;
use common::world::qenc;
use crworld::*;

async fn served_s(label: &str, vars: &[(&str, &str)]) -> Served {
    let lite = scratch(label);
    build(&lite, T3);
    serve(lite, vars).await
}

/// The count of `datumesh_catalog_builds_total{by="<by>"}`; an absent line is 0.
async fn builds(m: &Served, by: &str) -> u64 {
    let t = page(m, "/metrics").await.text();
    let want = format!("datumesh_catalog_builds_total{{by=\"{by}\"}}");
    t.lines().find_map(|l| l.strip_prefix(&want).map(|r| r.trim().parse::<u64>().unwrap())).unwrap_or(0)
}

fn domains() -> Vec<&'static str> {
    vec!["finance", "scripture", "earth", "registry", "tools"]
}

fn encoding_of(s: &Subject) -> String {
    sk(s.domain, s.thing, s.measure, s.region)
}

fn label_of(s: &Subject) -> String {
    let region = match s.region {
        "ea" => "euro area",
        "gb" => "United Kingdom",
        "us" => "United States",
        r => r,
    };
    let mut parts: Vec<String> = Vec::new();
    for (v, is_region) in [(s.measure, false), (s.thing, false), (region, true), (s.domain, false)] {
        let text = if is_region { v.to_string() } else { v.replace('_', " ") };
        if text != "none" {
            parts.push(text);
        }
    }
    parts.join(" · ")
}

fn html(p: &Page, what: &str) -> String {
    assert_eq!(p.status, 200, "{what}: {}", p.text());
    assert!(
        p.header("content-type").unwrap_or_default().starts_with("text/html"),
        "{what}: {:?}",
        p.header("content-type")
    );
    p.text()
}

// ---- the three pages --------------------------------------------------------------------------------------------------

/// `/catalog`: each domain with its number of subjects, in the order of C7, linking to its page; a link to the rule.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c9_the_catalogue_page_lists_the_domains_with_counts() {
    let m = served_s("c9-root", &[]).await;
    let body = html(&until_built(&m, "/catalog").await, "/catalog");
    let text = plain(&body);
    let counts = [("finance", 6), ("scripture", 2), ("earth", 1), ("registry", 1), ("tools", 1)];
    let words = words(&text);
    let mut last = 0;
    for (d, n) in counts {
        let i = words
            .iter()
            .skip(last)
            .position(|w| w.trim_matches(|c: char| "(),:".contains(c)) == d)
            .map(|i| i + last)
            .unwrap_or_else(|| panic!("{d} is not on the page, in order: {text}"));
        // the count is among the next few words, before the next domain
        let window: Vec<String> = words[i + 1..(i + 6).min(words.len())]
            .iter()
            .map(|w| w.trim_matches(|c: char| "(),:".contains(c)).to_string())
            .collect();
        assert!(window.contains(&n.to_string()), "{d} has {n} subjects: {window:?} in {text}");
        last = i + 1;
    }
    let links: Vec<String> = hrefs(&body).into_iter().map(|h| local(&h)).collect();
    for d in domains() {
        assert!(links.contains(&format!("/catalog/{d}")), "no link to /catalog/{d}: {links:?}");
    }
    assert!(links.contains(&"/catalog/rule".to_string()), "no link to the rule: {links:?}");
}

/// `/catalog/<domain>`: the subjects with their labels and their rank-1 source, each linking to the subject's page.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c9_a_domain_page_lists_its_subjects_with_labels_and_rank_one_sources() {
    let m = served_s("c9-domain", &[]).await;
    until_built(&m, "/catalog").await;
    for d in domains() {
        let body = html(&page(&m, &format!("/catalog/{d}")).await, d);
        let text = plain(&body);
        let links: Vec<String> = hrefs(&body).into_iter().map(|h| local(&h)).collect();
        for s in s_subjects().iter().filter(|s| s.domain == d) {
            let label = label_of(s);
            assert!(text.contains(&label), "{d}: no label {label:?}: {text}");
            assert!(
                text.contains(&s.sources[0]) || links.iter().any(|l| l.ends_with(&s.sources[0])),
                "{d}: no rank-1 source {}: {text}",
                s.sources[0]
            );
            assert!(
                links.contains(&subject_page(s.domain, s.thing, s.measure, s.region)),
                "{d}: no link to the page of {label}: {links:?}"
            );
        }
    }
    assert_eq!(page(&m, "/catalog/nosuchdomain").await.status, 404);
    assert_eq!(page(&m, "/catalog/none").await.status, 404);
}

/// A subject's page: its label, its sources in rank order, each linking to `/fn/<name>` or, for a class, `/describe/<class>`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c9_a_subject_page_lists_the_ranked_sources_with_links() {
    let m = served_s("c9-subject", &[]).await;
    until_built(&m, "/catalog").await;
    let class = company();
    for s in s_subjects() {
        let path = subject_page(s.domain, s.thing, s.measure, s.region);
        let body = html(&page(&m, &path).await, &path);
        assert!(plain(&body).contains(&label_of(&s)), "{path}: no label: {}", plain(&body));
        let links: Vec<String> = hrefs(&body).into_iter().map(|h| local(&h)).collect();
        // the links to sources, in page order
        let got: Vec<String> = links
            .iter()
            .filter_map(|l| l.strip_prefix("/fn/").or_else(|| l.strip_prefix("/describe/")).map(|n| unpct(n)))
            .collect();
        assert_eq!(got, s.sources, "{path}: the sources in rank order");
        for src in &s.sources {
            let want = if *src == class { format!("/describe/{src}") } else { format!("/fn/{src}") };
            assert!(links.iter().any(|l| unpct(l) == want), "{path}: no link {want}: {links:?}");
        }
    }
    // a wrong domain in the path, a subject that does not exist, a malformed key
    let s = &s_subjects()[4];
    let key = qenc(&encoding_of(s));
    assert_eq!(page(&m, &format!("/catalog/earth/subject?key={key}")).await.status, 404);
    let ghost = qenc(&sk("finance", "rate", "interest_rate", "de"));
    assert_eq!(page(&m, &format!("/catalog/finance/subject?key={ghost}")).await.status, 404);
    let bad = page(&m, "/catalog/finance/subject?key=not-a-key").await.status;
    assert!(bad == 400 || bad == 404, "{bad}");
    assert_eq!(page(&m, "/catalog/finance/subject").await.status, 404);
}

// ---- depth and links --------------------------------------------------------------------------------------------------

/// From `/catalog`, every page of the catalogue is at most two links away (three pages deep), and they are exactly the root, the
/// domains' pages and the subjects' pages.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c9_every_page_is_at_most_three_steps_deep() {
    let m = served_s("c9-depth", &[]).await;
    until_built(&m, "/catalog").await;
    let mut depth: BTreeMap<String, usize> = BTreeMap::new();
    let mut queue = VecDeque::new();
    depth.insert("/catalog".into(), 0);
    queue.push_back("/catalog".to_string());
    while let Some(p) = queue.pop_front() {
        let r = page(&m, &p).await;
        assert_eq!(r.status, 200, "{p}: {}", r.text());
        for h in hrefs(&r.text()) {
            let l = local(&h);
            let (segs, _) = split_link(&l);
            if segs.first().map(String::as_str) == Some("catalog") && !depth.contains_key(&l) {
                depth.insert(l.clone(), depth[&p] + 1);
                queue.push_back(l);
            }
        }
    }
    let deepest = depth.values().max().copied().unwrap();
    assert!(deepest <= 2, "the catalogue is at most three pages deep: {depth:?}");
    let mut want: BTreeSet<String> = BTreeSet::new();
    want.insert("/catalog".into());
    want.insert("/catalog/rule".into());
    for d in domains() {
        want.insert(format!("/catalog/{d}"));
    }
    for s in s_subjects() {
        want.insert(subject_page(s.domain, s.thing, s.measure, s.region));
    }
    let got: BTreeSet<String> = depth.keys().cloned().collect();
    assert_eq!(got, want, "the pages reachable from /catalog");
    for s in s_subjects() {
        assert_eq!(
            depth[&subject_page(s.domain, s.thing, s.measure, s.region)],
            2,
            "a subject's page is the third step"
        );
    }
}

/// Links are to stable names only: `/fn/<name>` and `/describe/<class>` of a bound full name exactly as bound, the catalogue's
/// own pages, and nothing else of this host. C10: no segment of a catalogue link is a rank, and none but the domain is a facet
/// value; the subject is addressed by its key encoding in the query, and only by it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c10_links_are_to_stable_names_and_a_subject_is_addressed_by_its_key_in_the_query() {
    let m = served_s("c10-links", &[]).await;
    until_built(&m, "/catalog").await;
    let names: BTreeSet<String> = s_all_sources().into_iter().collect();
    let encodings: BTreeSet<String> = s_subjects().iter().map(encoding_of).collect();
    let mut paths = vec!["/catalog".to_string(), "/catalog/rule".to_string()];
    paths.extend(domains().into_iter().map(|d| format!("/catalog/{d}")));
    paths.extend(s_subjects().iter().map(|s| subject_page(s.domain, s.thing, s.measure, s.region)));
    let facet_values: BTreeSet<&str> =
        s_subjects().iter().flat_map(|s| [s.thing, s.measure, s.region]).filter(|v| *v != "none").collect();
    let mut seen_fn = 0;
    for p in paths {
        let r = page(&m, &p).await;
        assert_eq!(r.status, 200, "{p}: {}", r.text());
        for h in hrefs(&r.text()) {
            let l = local(&h);
            if l.starts_with("https://") || l.starts_with("http://") {
                continue; // another host's
            }
            if l == "/" {
                continue;
            }
            if let Some(n) = l.strip_prefix("/fn/").or_else(|| l.strip_prefix("/describe/")) {
                let n = unpct(n);
                assert!(names.contains(&n), "{p}: {l} is not a link to a bound name as bound");
                seen_fn += 1;
                continue;
            }
            let (segs, query) = split_link(&l);
            assert_eq!(
                segs.first().map(String::as_str),
                Some("catalog"),
                "{p}: {l} is not a catalogue link or a name's page"
            );
            for seg in &segs {
                assert!(!seg.bytes().all(|b| b.is_ascii_digit()), "{p}: {l}: a rank is not a path segment");
                assert!(!facet_values.contains(seg.as_str()), "{p}: {l}: {seg} is a facet value, not a stable segment");
            }
            if segs.len() == 3 {
                assert_eq!((segs[0].as_str(), segs[2].as_str()), ("catalog", "subject"), "{p}: {l}");
                assert!(domains().contains(&segs[1].as_str()), "{p}: {l}");
                assert_eq!(query.len(), 1, "{p}: {l}: the one query member is the key");
                assert_eq!(query[0].0, "key", "{p}: {l}");
                assert!(encodings.contains(&query[0].1), "{p}: {l}: the key is a subject's encoding");
            } else {
                assert!(query.is_empty(), "{p}: {l}: only a subject's page has a query");
            }
        }
    }
    assert!(seen_fn > 0, "the pages link to the sources' pages");
}

/// Every `source` value is a bound full name, as the entry has it (no facet value or rank added), a function whose page exists
/// or the class; and a call made after reading the catalogue with that name answers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c10_every_source_value_is_a_stable_bound_name() {
    let m = served_s("c10-sources", &[]).await;
    let known: BTreeSet<String> = s_all_sources().into_iter().collect();
    let mut n = 0;
    for s in s_subjects() {
        let facets = [s.domain, s.thing, s.measure, s.region];
        for rank in 1..=s.sources.len() as u32 {
            let v = source_of(&m, s.domain, s.thing, s.measure, s.region, rank)
                .await
                .unwrap_or_else(|| panic!("no source({rank})"));
            assert!(known.contains(&v), "{v} is not the name of an entry");
            // a stable name: no facet value, and no rank, appended as a path segment of the entry's name
            let own: Vec<&str> = v.split('/').collect();
            assert!(own.last().map_or(false, |l| !l.bytes().all(|b| b.is_ascii_digit())), "{v}: ends in a rank");
            let given = s.sources[rank as usize - 1].as_str();
            assert_eq!(v, given, "{} {} {}: source({rank})", s.domain, s.thing, s.measure);
            let _ = facets;
            // the name is bound: its page answers
            let d = describe(&m, &v).await;
            assert_eq!(d.status, 200, "{v}: {}", d.text);
            // a call after the catalogue: a relayed function is read by its name
            if v.contains("/relay/") {
                let a = cell(&m, &format!("{v}({DAY})")).await;
                assert_eq!((a.status, a.outcome()), (200, "ok"), "{v}: {}", a.text);
            }
            n += 1;
        }
    }
    assert_eq!(n, s_subjects().iter().map(|s| s.sources.len()).sum::<usize>());
}

// ---- the cache --------------------------------------------------------------------------------------------------------

/// Before the first build every page answers `503` with `Retry-After` and `no-store`, at once, and builds nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c9_before_the_first_build_the_pages_answer_503() {
    let m = served_s("c9-cold", &[("DATUMESH_CATALOG_FIRST_BUILD_DELAY_SECS", "3600")]).await;
    let s = &s_subjects()[4];
    for path in
        ["/catalog".to_string(), "/catalog/finance".to_string(), subject_page(s.domain, s.thing, s.measure, s.region)]
    {
        let t = Instant::now();
        let r = page(&m, &path).await;
        assert_cold(&r, &path);
        assert!(t.elapsed() < Duration::from_secs(1), "{path}: answered at once, took {:?}", t.elapsed());
    }
    assert_eq!(builds(&m, "request").await, 0);
    assert_eq!(builds(&m, "background").await, 0);
    assert_cold(&page(&m, "/catalog").await, "again");
}

/// After the build the pages are served from it: quick, the same every time, and nothing is built on a request.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c9_after_the_first_build_requests_read_it_and_build_nothing() {
    let m = served_s("c9-warm", &[]).await;
    let first = until_built(&m, "/catalog").await;
    assert_eq!(first.status, 200, "{}", first.text());
    assert!(builds(&m, "background").await >= 1);
    let s = &s_subjects()[4];
    for _ in 0..10 {
        for path in [
            "/catalog".to_string(),
            "/catalog/finance".to_string(),
            subject_page(s.domain, s.thing, s.measure, s.region),
        ] {
            let t = Instant::now();
            let r = page(&m, &path).await;
            assert_eq!(r.status, 200, "{path}: {}", r.text());
            assert!(t.elapsed() < Duration::from_secs(1), "{path}: {:?}", t.elapsed());
        }
    }
    assert_eq!(page(&m, "/catalog").await.body, first.body, "the same page every time");
    assert_eq!(builds(&m, "request").await, 0, "no page was built on a request");
}

/// No work grows with the data on a request: a domain of 125 subjects (and 125 subjects' pages) is as quick as one, and still
/// nothing is built on a request.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c9_no_work_grows_with_the_data_on_a_request() {
    let lite = Lite::new("c9-wide");
    let t: Vec<String> = (1..=5).map(|i| format!("t{i}")).collect();
    let me: Vec<String> = (1..=5).map(|i| format!("m{i}")).collect();
    let r: Vec<String> = (1..=5).map(|i| format!("r{i}")).collect();
    let (t, me, r): (Vec<&str>, Vec<&str>, Vec<&str>) = (
        t.iter().map(String::as_str).collect(),
        me.iter().map(String::as_str).collect(),
        r.iter().map(String::as_str).collect(),
    );
    let entry = (host_name("wide/f"), fac(&["wide"], &t, &me, &r, &["daily"]));
    crworld::publish(&lite, "w.jsonl", &entries_lines(&[entry]), NOW).unwrap_or_else(|e| panic!("{e}"));
    let b = build(&lite, T3);
    assert!(cells(&b) >= 125, "{b:?}");
    let m = serve(lite, &[]).await;
    let root = until_built(&m, "/catalog").await;
    assert_eq!(root.status, 200, "{}", root.text());
    assert!(plain(&root.text()).contains("125"), "{}", plain(&root.text()));
    assert!(builds(&m, "background").await >= 1);
    for _ in 0..10 {
        for path in ["/catalog".to_string(), "/catalog/wide".to_string(), subject_page("wide", "t3", "m2", "r5")] {
            let t = Instant::now();
            let p = page(&m, &path).await;
            assert_eq!(p.status, 200, "{path}: {}", p.text());
            assert!(t.elapsed() < Duration::from_secs(1), "{path}: {:?}", t.elapsed());
        }
    }
    assert_eq!(builds(&m, "request").await, 0);
}

/// The pages are rebuilt on a schedule, never on a request: an entry added after the first build is on `/catalog` within a few
/// seconds of the schedule (1 s here), and the request counter stays 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c9_the_pages_are_rebuilt_on_a_schedule_never_on_a_request() {
    let m = served_s("c9-refresh", &[("DATUMESH_CATALOG_REFRESH_SECS", "1")]).await;
    let first = until_built(&m, "/catalog").await;
    assert_eq!(first.status, 200, "{}", first.text());
    assert!(!plain(&first.text()).contains("newdom"));
    let name = host_name("fresh/f");
    let lines = entries_lines(&[(name, fac(&["newdom"], &["x"], &["m1"], &["world"], &["daily"]))]);
    let f = common::pub031::write(&m.lite, "fresh.jsonl", &lines);
    common::pub031::run(&m.lite, &f, T4).unwrap_or_else(|e| panic!("{e}"));
    build(&m.lite, T5);
    let _ = page(&m, "/catalog").await;
    assert_eq!(builds(&m, "request").await, 0);
    let end = Instant::now() + Duration::from_secs(20);
    let mut seen = false;
    while Instant::now() < end {
        let r = page(&m, "/catalog").await;
        assert_eq!(r.status, 200, "{}", r.text());
        if plain(&r.text()).contains("newdom") {
            seen = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(seen, "the scheduled build did not add the domain in 20 s");
    assert!(builds(&m, "background").await >= 2, "built at start and again on the schedule");
    assert_eq!(builds(&m, "request").await, 0);
}
