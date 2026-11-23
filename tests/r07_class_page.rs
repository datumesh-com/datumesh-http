//! The 0.7 round (CONTRACT-round-0.7.md), R9 (and R7's page paths): a page for people for each class. Failing tests.
//!
//! `GET /fn/<class name>` serves a class page: its label and description, keys with their types, methods each linking to its
//! `/fn/<class>/<m>` page (the defining class's under K5, for an inherited method), the parent and the subclasses by name, facets,
//! the subjects it is a source of with its rank, the object count from the stored counter, the lookup's example labels, and the
//! steward. `?format=json` gives the same data. The pages are rebuilt by the catalogue thread when `catalog-build` finished and only
//! when it changed something: the build compares the generation the job writes (the latest `tx` of the `catalog/*` cells) with the
//! one it built from. A cold page before the first build is `503` with `Retry-After` and `no-store`.
//!
//! Interfaces pinned here (the implementer follows them; the report lists them):
//! * the page is `text/html`; with `?format=json` it is `application/json` with the members
//!   `{"name", "label", "description", "keys": [{"name", "type"}], "methods": [{"name", "page"}], "parent"?: "<class name>",
//!   "subclasses": ["<class name>", ...], "facets": {...}, "subjects": [{"subject": "<key encoding>", "label", "rank": N}, ...],
//!   "objects": N, "labels": ["...", ...], "steward": {"namespace", "process"}}`; `parent` is absent for a root class, `facets`
//!   `{}` for a class without; `labels` is `[]` for a class without a lookup;
//! * a method's `page` is the path or the URL of its `/fn/<defining class>/<method>` page;
//! * the HTML shows the same things and links: each method to its page, the parent and each subclass to `/fn/<class>`, each
//!   subject to its page `/catalog/<domain>/subject?key=...`;
//! * the rebuilds are counted by `datumesh_catalog_builds_total{by="background"}`, which counts a build that was made (not a
//!   check that found the generation unchanged); `by="request"` stays 0.

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/r07world.rs"]
mod r07world;

use std::time::{Duration, Instant};

use catjob::*;
use catworld::*;
use common::lite::*;
use common::pub031::{T1, T2};
use common::readpath::*;
use crworld::*;
use r07world::*;
use serde_json::{json, Value};

async fn scratch_served(label: &str, vars: &[(&str, &str)]) -> Served {
    let lite = scratch(label);
    build(&lite, T3);
    serve(lite, vars).await
}

async fn family_served(label: &str) -> Served {
    let lite = family(
        label,
        &[fam("fam/parent", None, "", "rate", NOW), fam("fam/child", Some(0), "fixing", "fixed", T1)],
        &[],
    );
    build(&lite, T3);
    serve(lite, &[]).await
}

fn json_of(p: &Page, what: &str) -> Value {
    assert_eq!(p.status, 200, "{what}: {}", p.text());
    assert!(
        p.header("content-type").unwrap_or_default().starts_with("application/json"),
        "{what}: {:?}",
        p.header("content-type")
    );
    p.json()
}

fn html_of(p: &Page, what: &str) -> String {
    assert_eq!(p.status, 200, "{what}: {}", p.text());
    assert!(
        p.header("content-type").unwrap_or_default().starts_with("text/html"),
        "{what}: {:?}",
        p.header("content-type")
    );
    p.text()
}

async fn class_json(m: &Served, name: &str) -> Value {
    let p = until_built(m, &format!("/fn/{name}?format=json")).await;
    json_of(&p, name)
}

fn methods_of(v: &Value) -> Vec<(String, String)> {
    v["methods"]
        .as_array()
        .unwrap_or_else(|| panic!("no methods: {v}"))
        .iter()
        .map(|m| {
            (
                m["name"].as_str().unwrap().to_string(),
                local(m["page"].as_str().unwrap_or_else(|| panic!("no page: {m}"))),
            )
        })
        .collect()
}

fn names(v: &Value) -> Vec<String> {
    v.as_array().unwrap_or_else(|| panic!("not a list: {v}")).iter().map(|x| x.as_str().unwrap().to_string()).collect()
}

// ---- the page --------------------------------------------------------------------------------------------------------

/// A class page's sections, in HTML: the label and description, the keys with their types, the methods linking to their pages,
/// the facets, the subject it is a source of (linking to the subject's page), and the steward.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r9_a_class_page_has_its_sections_and_links() {
    let m = scratch_served("r9-html", &[]).await;
    let c = company();
    let p = until_built(&m, &class_path(&c)).await;
    let body = html_of(&p, &c);
    let text = plain(&body);
    for want in [
        "A synthetic name",
        "A synthetic description.",
        "cik",
        "string",
        "filed",
        "finance",
        "annual",
        HOST,
        "classes/change",
    ] {
        assert!(text.contains(want) || body.contains(want), "{want:?} is not on the page: {text}");
    }
    let links: Vec<String> = hrefs(&body).into_iter().map(|h| local(&h)).collect();
    let filed = format!("/fn/{c}/filed");
    assert!(links.iter().any(|l| unpct(l) == filed), "no link to the method page {filed}: {links:?}");
    let s = subject_page("finance", "company", "fundamentals", "us");
    assert!(links.contains(&s), "no link to the subject's page {s}: {links:?}");
    assert!(text.contains("fundamentals · company · United States · finance"), "the subject's label: {text}");
}

/// The same data as JSON.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r9_the_json_has_the_same_data() {
    let m = scratch_served("r9-json", &[]).await;
    let c = company();
    let v = class_json(&m, &c).await;
    assert_eq!(v["name"], c.as_str(), "{v}");
    assert_eq!(v["label"], "A synthetic name", "{v}");
    assert_eq!(v["description"], "A synthetic description.", "{v}");
    let keys = v["keys"].as_array().unwrap_or_else(|| panic!("{v}"));
    assert_eq!(keys.len(), 1, "{v}");
    assert_eq!((keys[0]["name"].as_str(), keys[0]["type"].as_str()), (Some("cik"), Some("string")), "{v}");
    assert_eq!(methods_of(&v), [("filed".to_string(), format!("/fn/{c}/filed"))], "{v}");
    assert!(v.get("parent").is_none_or(Value::is_null), "a root class has no parent: {v}");
    assert_eq!(v["subclasses"], json!([]), "{v}");
    assert_eq!(v["facets"]["domain"], json!(["finance"]), "{v}");
    assert_eq!(v["facets"]["frequency"], json!(["annual"]), "{v}");
    assert_eq!(
        v["subjects"],
        json!([{"subject": sk("finance", "company", "fundamentals", "us"), "label": "fundamentals · company · United States · finance", "rank": 1}]),
        "{v}"
    );
    assert_eq!(v["objects"], 1, "the object count is the stored counter of its first method: {v}");
    assert_eq!(v["labels"], json!([]), "no lookup, no labels: {v}");
    assert_eq!(v["steward"], json!({"namespace": HOST, "process": format!("https://{HOST}/classes/change")}), "{v}");
    // the members, and nothing else
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "description",
            "facets",
            "keys",
            "label",
            "labels",
            "methods",
            "name",
            "objects",
            "steward",
            "subclasses",
            "subjects"
        ],
        "{v}"
    );
}

/// The subclass and the parent, by name; an inherited method links to the page of the class that defines it (K5); the subject
/// lists each class with its rank (the subclass is rank 1 under R5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r9_parent_subclasses_and_inherited_methods() {
    let m = family_served("r9-family").await;
    let (p, c) = (host_name("fam/parent"), host_name("fam/child"));
    let pv = class_json(&m, &p).await;
    let cv = class_json(&m, &c).await;
    assert!(pv.get("parent").is_none_or(Value::is_null), "{pv}");
    assert_eq!(names(&pv["subclasses"]), std::slice::from_ref(&c), "{pv}");
    assert_eq!(cv["parent"], p.as_str(), "{cv}");
    assert_eq!(cv["subclasses"], json!([]), "{cv}");
    assert_eq!(methods_of(&pv), [("rate".to_string(), format!("/fn/{p}/rate"))], "{pv}");
    let mc = methods_of(&cv);
    assert!(mc.contains(&("fixed".to_string(), format!("/fn/{c}/fixed"))), "its own method: {mc:?}");
    assert!(
        mc.contains(&("rate".to_string(), format!("/fn/{p}/rate"))),
        "an inherited method links to the defining class: {mc:?}"
    );
    assert_eq!(mc.len(), 2, "{mc:?}");
    // the keys: the subclass's own and the inherited
    let ck: Vec<&str> = cv["keys"].as_array().unwrap().iter().map(|k| k["name"].as_str().unwrap()).collect();
    assert_eq!(ck, ["base", "quote", "fixing"], "{cv}");
    // the rank in the subject
    let subject = sk(FAM.0, FAM.1, FAM.2, FAM.3);
    assert_eq!(cv["subjects"][0]["subject"], subject.as_str(), "{cv}");
    assert_eq!(cv["subjects"][0]["rank"], 1, "{cv}");
    assert_eq!(pv["subjects"][0]["rank"], 2, "{pv}");
    // and in HTML
    let hp = html_of(&until_built(&m, &class_path(&p)).await, &p);
    let hc = html_of(&page(&m, &class_path(&c)).await, &c);
    let lp: Vec<String> = hrefs(&hp).into_iter().map(|h| unpct(&local(&h))).collect();
    let lc: Vec<String> = hrefs(&hc).into_iter().map(|h| unpct(&local(&h))).collect();
    assert!(lp.contains(&class_path(&c)), "the parent page links to the subclass: {lp:?}");
    assert!(lc.contains(&class_path(&p)), "the subclass page links to the parent: {lc:?}");
    assert!(lc.contains(&format!("/fn/{p}/rate")), "{lc:?}");
}

/// The count of objects is the stored counter, and the lookup's example labels are real labels: the class `catalog/subject` has
/// the 11 subjects of the world as objects (its `exists()` cells), five methods, and labels that resolve.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r9_objects_methods_and_example_labels_of_the_job_class() {
    let m = scratch_served("r9-job-class", &[]).await;
    let c = subject_class();
    let v = class_json(&m, &c).await;
    assert_eq!(v["objects"], s_subjects().len(), "{v}");
    let ms = methods_of(&v);
    let mn: Vec<&str> = ms.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(mn, ["exists", "from_label", "label", "source", "sources"], "{v}");
    for (n, page) in &ms {
        assert_eq!(page, &format!("/fn/{c}/{n}"), "{v}");
    }
    let keys: Vec<&str> = v["keys"].as_array().unwrap().iter().map(|k| k["name"].as_str().unwrap()).collect();
    assert_eq!(keys, ["domain", "thing", "measure", "region"], "{v}");
    let labels = names(&v["labels"]);
    assert!(!labels.is_empty() && labels.len() <= 50, "example labels, a few: {labels:?}");
    let a = ask(&m, &path(&format!("{c}[\"{}\"].label()", labels[0]), None)).await;
    assert_eq!(a.outcome(), "ok", "the example label {:?} resolves: {}", labels[0], a.text);
    // the facet class is a class of its own, with its own page
    let f = class_json(&m, &facet_class()).await;
    assert_eq!(f["name"], facet_class().as_str(), "{f}");
    assert!(f["objects"].as_u64().unwrap() > 0, "{f}");
}

/// A name that is not a class has no class page: 404 after the first build; a function's page is the function's as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r9_other_names_are_as_before() {
    let m = scratch_served("r9-other", &[]).await;
    until_built(&m, &class_path(&company())).await;
    assert_eq!(page(&m, &class_path(&host_name("no/such_class"))).await.status, 404);
    assert_eq!(page(&m, &format!("{}?format=json", class_path(&host_name("no/such_class")))).await.status, 404);
    let f = host_name("relay/ecb.europa.eu/fx_reference");
    let p = page(&m, &class_path(&f)).await;
    assert_eq!(p.status, 200, "a function's page: {}", p.text());
    assert!(!plain(&p.text()).contains("subclasses"), "a function page is not a class page");
}

/// The class pages are built by the catalogue thread, not on a request: many requests build nothing and are quick.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r9_requests_read_the_build_and_build_nothing() {
    let m = scratch_served("r9-warm", &[]).await;
    let c = company();
    until_built(&m, &class_path(&c)).await;
    let first = page(&m, &class_path(&c)).await.body;
    for _ in 0..10 {
        for path in [class_path(&c), format!("{}?format=json", class_path(&c)), class_path(&subject_class())] {
            let t = Instant::now();
            let r = page(&m, &path).await;
            assert_eq!(r.status, 200, "{path}: {}", r.text());
            assert!(t.elapsed() < Duration::from_secs(1), "{path}: {:?}", t.elapsed());
        }
    }
    assert_eq!(page(&m, &class_path(&c)).await.body, first, "the same page every time");
    assert_eq!(builds(&m, "request").await, 0);
}

// ---- the cold page ---------------------------------------------------------------------------------------------------

/// Before the first build a class page, in HTML and in JSON, is `503` with `Retry-After` and `no-store`, at once, and nothing is
/// built. A function's page does not wait for the catalogue.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r9_a_cold_class_page_is_503() {
    let m = scratch_served("r9-cold", &[("DATUMESH_CATALOG_FIRST_BUILD_DELAY_SECS", "3600")]).await;
    let c = company();
    for path in [class_path(&c), format!("{}?format=json", class_path(&c)), class_path(&subject_class())] {
        let t = Instant::now();
        let r = page(&m, &path).await;
        assert_cold(&r, &path);
        assert!(t.elapsed() < Duration::from_secs(1), "{path}: answered at once, took {:?}", t.elapsed());
    }
    assert_eq!(builds(&m, "request").await, 0);
    assert_eq!(builds(&m, "background").await, 0);
    let f = page(&m, &class_path(&host_name("relay/ecb.europa.eu/fx_reference"))).await;
    assert_eq!(f.status, 200, "a function's page is not cold: {}", f.text());
}

// ---- the rebuilds ----------------------------------------------------------------------------------------------------

async fn wait_for(m: &Served, what: &str, mut ok: impl FnMut(&Page) -> bool, path: &str) {
    let end = Instant::now() + Duration::from_secs(25);
    while Instant::now() < end {
        let r = page(m, path).await;
        if r.status == 200 && ok(&r) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("{what}: not seen in 25 s");
}

/// A job that changed something is followed by a rebuild of the root, the `/catalog` pages and the class pages (exactly one), and
/// a build that had nothing new does not build again: with a refresh every second, nothing is built for as long as the
/// generation (the latest `tx` of the `catalog/*` cells) does not move, whether the job reran with nothing to write or an
/// unrelated name was published.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r9_rebuilds_only_when_the_job_changed_something() {
    let m = scratch_served("r9-rebuild", &[("DATUMESH_CATALOG_REFRESH_SECS", "1")]).await;
    let c = company();
    let first = until_built(&m, &class_path(&c)).await;
    assert_eq!(first.status, 200, "{}", first.text());
    let n0 = builds(&m, "background").await;
    assert!(n0 >= 1, "built once at start");
    let subjects = |p: &Page| p.json()["subjects"].as_array().map_or(0, Vec::len);
    assert_eq!(subjects(&page(&m, &format!("{}?format=json", class_path(&c))).await), 1);

    // nothing changed: three refresh intervals and no build
    tokio::time::sleep(Duration::from_millis(3500)).await;
    assert_eq!(builds(&m, "background").await, n0, "no build while the generation did not move");

    // a rerun of the job with nothing to write, and an unrelated publication: still none
    let again = build(&m.lite, T4);
    assert_eq!(cells(&again), 0, "{again:?}");
    let unrelated = common::pub031::write(&m.lite, "unrelated.jsonl", &[fn_line(&host_name("misc/unrelated"))]);
    common::pub031::run(&m.lite, &unrelated, T4).unwrap_or_else(|e| panic!("{e}"));
    tokio::time::sleep(Duration::from_millis(3500)).await;
    assert_eq!(
        builds(&m, "background").await,
        n0,
        "a job that changed nothing and an unrelated publication build nothing"
    );

    // a change: the class becomes a source of a second subject
    let facets = fac(&["finance"], &["company"], &["fundamentals"], &["gb", "us"], &["annual"]);
    let f = common::pub031::write(&m.lite, "change.jsonl", &[catalog_line_of(&c, "class", json!({"facets": facets}))]);
    common::pub031::run(&m.lite, &f, T5).unwrap_or_else(|e| panic!("{e}"));
    let run = build(&m.lite, T6);
    assert!(cells(&run) > 0, "{run:?}");
    let path = format!("{}?format=json", class_path(&c));
    wait_for(&m, "the class page lists the new subject", |p| subjects(p) == 2, &path).await;
    // the root and the pages followed in the same rebuild
    let root = page(&m, "/catalog/finance").await;
    assert!(plain(&root.text()).contains("United Kingdom"), "{}", plain(&root.text()));
    let d = describe(&m, &format!("{HOST}/catalog")).await;
    assert_eq!(d.status, 200, "{}", d.text);
    tokio::time::sleep(Duration::from_millis(3500)).await;
    assert_eq!(builds(&m, "background").await, n0 + 1, "one build for the change, none after it");
    assert_eq!(builds(&m, "request").await, 0);
    let _ = (T1, T2);
}
