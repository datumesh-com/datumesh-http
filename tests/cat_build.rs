//! The catalogue, phase 1 (CONTRACT-classes-round.md), C3, C4 and C5: the classes `catalog/subject` and `catalog/facet` and the
//! job `datumesh-mesh catalog-build` that writes them.
//!
//! The job reads the store's catalogue entries that have facets (as of its clock) and writes the two classes with their methods
//! inline (K7), as stored cells signed by the mesh's key with the licence it is given (`DATUMESH_LICENCE`), and the rule text as
//! the page `/catalog/rule` and the cell `catalog/rule()`. The interfaces are in `common/catworld.rs`.
//!
//! Choices where the contract is silent (the implementer follows them; each is in the report):
//! * **The label's form.** `label()` of a subject is its facet values with `_` as a space and the region written out, in the
//!   order measure, thing, region, domain, joined by ` · ` (U+00B7 with a space on each side), a value `none` left out
//!   (`ratio · tools`). Regions: `ea` euro area, `gb` United Kingdom, `us` United States, `world` world.
//! * **The lookup's labels.** The folded `label()`; and `<measure> <region>` and `<thing> <measure> <region>` (each value with
//!   spaces, the region written out, folded: `interest rate euro area`), each only when it names exactly one subject. A lookup
//!   cell's value is the subject's key encoding (16.5).
//! * **Levels** (C5). Per subject. A *relay* is a name `<host>/relay/<domain>/...` whose function has relayed cells; a *view* is
//!   a function or class that reads a relay name directly or through other views, by its descriptor's inputs (a class: by its
//!   methods' inputs). Relays and views are level 2. Of the others (non-relayed, not views), an entry whose publisher host
//!   equals a source domain of the subject, or is a sub-domain of one, is level 1; any other is level 3. The source domains of a
//!   subject are the domains of the relays and views in it; a subject with none has its non-relayed entries' own hosts, so every
//!   entry of it is level 1. (This is the reading of "for a non-relayed entry, the source domain is its own host" that leaves
//!   level 3 reachable; the report asks the user to confirm it.)
//! * **Within a level**: the instant the name was first bound (a class: the class name's binding), then the name ascending.
//! * `exists()` is `true` for each subject, a stored boolean cell; `catalog/facet`'s objects are the (facet, value) pairs of all
//!   five keys of the entries.
//! * A subject whose entry loses its facets: `exists()` false and its lookup cells (every label it had) `""`; a facet value no
//!   entry has: `exists()` false. The run that does it writes new versions only for what changed.

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;

use catjob::*;
use catworld::*;
use common::lite::*;
use common::pub031::{snapshot, write};
use common::readpath::*;
use common::*;
use crworld::*;
use datumesh_store::Store;
use serde_json::{json, Value};

/// Reads the sources of a subject in rank order until a rank has no cell.
async fn sources_of(m: &Served, d: &str, t: &str, me: &str, r: &str) -> Vec<String> {
    let mut out = Vec::new();
    for rank in 1..=20 {
        match source_of(m, d, t, me, r, rank).await {
            Some(s) => out.push(s),
            None => break,
        }
    }
    out
}

fn ok_value(a: &Ans, what: &str) -> String {
    assert_eq!((a.status, a.outcome()), (200, "ok"), "{what}: {}", a.text);
    a.value().unwrap_or_else(|| panic!("{what}: no value: {}", a.text)).to_string()
}

// ---- the classes, as stored cells signed by the mesh -------------------------------------------------------------------

/// The two classes are bound, with the keys, the order, the lookup and the extent the contract says, and their methods are as
/// C3 and C4 list them; `source`'s `rank` is a number with unit `1` and the literal default `"1"` (K13).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c3_the_subject_class_has_its_keys_and_methods() {
    let lite = vocab("c3-class");
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    let a = describe(&m, &subject_class()).await;
    assert_eq!((a.status, a.kind()), (200, Some("class")), "{}", a.text);
    let d = &a.v["descriptor"];
    let nostr = json!({"key_normalisation": "none", "type": "string"});
    assert_eq!(d["keys"], json!({"domain": nostr, "measure": nostr, "region": nostr, "thing": nostr}), "{}", a.text);
    assert_eq!(d["order"], json!(["domain", "thing", "measure", "region"]), "keys in that order: {}", a.text);
    assert_eq!(d["lookup"], "from_label", "{}", a.text);
    let methods = a.v["methods"].as_object().unwrap();
    let mut names: Vec<&str> = methods.keys().map(String::as_str).collect();
    names.sort();
    assert_eq!(names, ["exists", "from_label", "label", "source", "sources"], "{}", a.text);
    assert_eq!(d["extent"]["keys"], methods["exists"]["fn_hash"], "exists() is the extent.keys: {}", a.text);
    let rank = &methods["source"]["args"]["rank"];
    assert_eq!(
        (&rank["type"], &rank["unit"], &rank["default"]),
        (&json!("number"), &json!("1"), &json!("1")),
        "{}",
        a.text
    );
    assert_eq!(methods["source"]["returns"], "string", "{}", a.text);
    assert_eq!(methods["sources"]["returns"], json!({"type": "number", "unit": "1"}), "{}", a.text);
    assert_eq!(methods["label"]["returns"], "string", "{}", a.text);
    assert_eq!(methods["exists"]["returns"], "boolean", "{}", a.text);
    let fl = &methods["from_label"];
    assert_eq!(fl["args"]["label"]["key_normalisation"], "label_fold", "{}", a.text);
    assert_eq!(fl["returns"], json!({"classes": [subject_class()], "type": "string"}), "{}", a.text);
    // the steward is the mesh
    assert_eq!(d["steward"]["namespace"], HOST, "{}", a.text);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c4_the_facet_class_has_its_keys_and_methods() {
    let lite = vocab("c4-class");
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    let a = describe(&m, &facet_class()).await;
    assert_eq!((a.status, a.kind()), (200, Some("class")), "{}", a.text);
    let d = &a.v["descriptor"];
    let nostr = json!({"key_normalisation": "none", "type": "string"});
    assert_eq!(d["keys"], json!({"facet": nostr, "value": nostr}), "{}", a.text);
    assert_eq!(d["order"], json!(["facet", "value"]), "{}", a.text);
    let methods = a.v["methods"].as_object().unwrap();
    let mut names: Vec<&str> = methods.keys().map(String::as_str).collect();
    names.sort();
    assert_eq!(names, ["exists", "label", "subjects"], "{}", a.text);
    assert_eq!(d["extent"]["keys"], methods["exists"]["fn_hash"], "{}", a.text);
    assert_eq!(methods["label"]["returns"], "string", "{}", a.text);
    assert_eq!(methods["subjects"]["returns"], json!({"type": "number", "unit": "1"}), "{}", a.text);
    assert_eq!(methods["exists"]["returns"], "boolean", "{}", a.text);
    assert!(d.get("lookup").is_none(), "the facet class has no lookup: {}", a.text);
}

/// Every cell the job wrote is a stored cell (no inputs), signed by the mesh's key (the answer checks as a client checks it,
/// against the mesh's key document), under the licence the job was given.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c5_the_cells_are_stored_signed_by_the_mesh_with_the_licence() {
    let lite = vocab("c5-signed");
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    let keys = common::pub031::keys(&m.lite);
    let calls = [
        sc("finance", "rate", "interest_rate", "ea", "label()"),
        sc("finance", "rate", "interest_rate", "ea", "source(1)"),
        sc("finance", "rate", "interest_rate", "ea", "sources()"),
        sc("finance", "rate", "interest_rate", "ea", "exists()"),
        fc("measure", "interest_rate", "label()"),
        fc("measure", "interest_rate", "subjects()"),
        fc("measure", "interest_rate", "exists()"),
        format!("{}/from_label(label: \"interest rate · rate · euro area · finance\")", subject_class()),
        format!("{}()", rule_fn()),
    ];
    for call in calls {
        let a = cell(&m, &call).await;
        assert_eq!((a.status, a.outcome()), (200, "ok"), "{call}: {}", a.text);
        assert_eq!(a.v["receipt"]["licence"], LICENCE, "{call}: {}", a.text);
        assert!(a.inputs().is_empty(), "{call}: a stored cell has no inputs: {}", a.text);
        let req = datumesh_core::Request::new(&a.cell_id(), None).unwrap();
        datumesh_core::check_answer(a.status, a.text.as_bytes(), &keys, &req)
            .unwrap_or_else(|e| panic!("{call}: {}", e.code_str()));
    }
}

// ---- the subject's methods and their values ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c3_label_is_the_facet_values_with_the_region_written_out() {
    let lite = vocab("c3-label");
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    for (d, t, me, r, label) in v_subjects() {
        let a = cell(&m, &sc(d, t, me, r, "label()")).await;
        assert_eq!(ok_value(&a, label), label, "{d} {t} {me} {r}");
    }
}

/// `source(rank)`: the default `1` is the literal of K13: the call without the argument is the cell of rank 1, and `resolved`
/// records a `default`. `sources()` counts the sources; a rank beyond has no cell.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c3_source_defaults_to_rank_one_and_sources_counts() {
    let lite = vocab("c3-source");
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    let (d, t, me, r) = ("finance", "rate", "interest_rate", "ea");
    let one = cell(&m, &sc(d, t, me, r, "source(1)")).await;
    let none = cell(&m, &sc(d, t, me, r, "source()")).await;
    assert_eq!(ok_value(&none, "source()"), ok_value(&one, "source(1)"));
    assert_eq!(none.cell_id(), one.cell_id(), "the default is bound in the cell");
    assert!(none.resolution_kinds().contains(&"default".to_string()), "{:?}", none.resolutions());
    // the two relays of the subject (estr and policy_rate), bound together: by name
    let first = host_name("relay/ecb.europa.eu/estr");
    let second = host_name("relay/ecb.europa.eu/policy_rate");
    assert_eq!(ok_value(&one, "source(1)"), first);
    assert_eq!(ok_value(&cell(&m, &sc(d, t, me, r, "source(2)")).await, "source(2)"), second);
    assert_eq!(ok_value(&cell(&m, &sc(d, t, me, r, "sources()")).await, "sources()"), "2");
    assert_eq!(
        ok_value(&cell(&m, &sc("finance", "currency", "exchange_rate", "ea", "sources()")).await, "sources()"),
        "1"
    );
    let beyond = cell(&m, &sc(d, t, me, r, "source(3)")).await;
    assert_eq!((beyond.status, beyond.outcome()), (404, "not_covered"), "{}", beyond.text);
}

/// `exists()` is `true` for each subject and no cell for what is not one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c3_exists_is_true_for_each_subject() {
    let lite = vocab("c3-exists");
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    for (d, t, me, r, label) in v_subjects() {
        let a = cell(&m, &sc(d, t, me, r, "exists()")).await;
        assert_eq!((a.status, a.outcome(), truth(&a)), (200, "ok", Some(true)), "{label}: {}", a.text);
    }
    let no = cell(&m, &sc("finance", "rate", "interest_rate", "us", "exists()")).await;
    assert_eq!((no.status, no.outcome()), (404, "not_covered"), "{}", no.text);
}

/// The subjects are the cross product of an entry's domain, thing, measure and region values; `frequency` is a facet only.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c3_the_subjects_are_the_cross_product_and_frequency_is_not_a_key() {
    let lite = Lite::new("c3-cross");
    let name = host_name("scratch/cross");
    let facets = fac(&["a_dom", "b_dom"], &["t1", "t2"], &["m1"], &["ea", "gb"], &["daily", "monthly"]);
    crworld::publish(&lite, "x.jsonl", &entries_lines(&[(name.clone(), facets)]), NOW)
        .unwrap_or_else(|e| panic!("{e}"));
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    let mut n = 0;
    for d in ["a_dom", "b_dom"] {
        for t in ["t1", "t2"] {
            for r in ["ea", "gb"] {
                let a = cell(&m, &sc(d, t, "m1", r, "exists()")).await;
                assert_eq!(truth(&a), Some(true), "{d} {t} {r}: {}", a.text);
                assert_eq!(ok_value(&cell(&m, &sc(d, t, "m1", r, "source(1)")).await, "source"), name);
                n += 1;
            }
        }
    }
    assert_eq!(n, 8);
    // not in the product
    for (d, t, me, r) in [("a_dom", "t1", "m1", "us"), ("c_dom", "t1", "m1", "ea"), ("a_dom", "t3", "m1", "ea")] {
        let a = cell(&m, &sc(d, t, me, r, "exists()")).await;
        assert_eq!(a.outcome(), "not_covered", "{d} {t} {me} {r}: {}", a.text);
    }
    // the vocabulary counts the product
    assert_eq!(ok_value(&cell(&m, &fc("region", "ea", "subjects()")).await, "region ea"), "4");
    assert_eq!(ok_value(&cell(&m, &fc("domain", "a_dom", "subjects()")).await, "domain a_dom"), "4");
    assert_eq!(ok_value(&cell(&m, &fc("measure", "m1", "subjects()")).await, "measure m1"), "8");
    // frequency is a facet: it has objects, not a key
    for f in ["daily", "monthly"] {
        let a = cell(&m, &fc("frequency", f, "exists()")).await;
        assert_eq!(truth(&a), Some(true), "frequency {f}: {}", a.text);
    }
}

// ---- from_label --------------------------------------------------------------------------------------------------------

/// The lookup: the folded label, and `<measure> <region>` and `<thing> <measure> <region>` when each names one subject. The
/// value is the subject's key encoding, and a request's label is folded by the mesh (K8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c3_from_label_reads_the_three_forms_when_unique() {
    let lite = vocab("c3-lookup");
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    let lookup = |label: &str| format!("/cell/{}/from_label?label={}", subject_class(), common::world::qenc(label));
    let ea_rate = sk("finance", "rate", "interest_rate", "ea");
    let cases: Vec<(&str, String)> = vec![
        // the folded label()
        ("interest rate · rate · euro area · finance", ea_rate.clone()),
        ("exchange rate · currency · euro area · finance", sk("finance", "currency", "exchange_rate", "ea")),
        ("interest rate · rate · united kingdom · finance", sk("finance", "rate", "interest_rate", "gb")),
        ("yield · yield · united states · finance", sk("finance", "yield", "yield", "us")),
        ("rainfall · region · world · earth", sk("earth", "region", "rainfall", "world")),
        ("ratio · tools", sk("tools", "none", "ratio", "none")),
        // <measure> <region>
        ("exchange rate euro area", sk("finance", "currency", "exchange_rate", "ea")),
        ("interest rate united kingdom", sk("finance", "rate", "interest_rate", "gb")),
        ("yield united states", sk("finance", "yield", "yield", "us")),
        ("rainfall world", sk("earth", "region", "rainfall", "world")),
        // <thing> <measure> <region>
        ("currency exchange rate euro area", sk("finance", "currency", "exchange_rate", "ea")),
        ("rate interest rate euro area", ea_rate.clone()),
        ("yield interest rate euro area", sk("finance", "yield", "interest_rate", "ea")),
        ("rate interest rate united kingdom", sk("finance", "rate", "interest_rate", "gb")),
    ];
    for (label, want) in cases {
        let a = ask(&m, &lookup(label)).await;
        assert_eq!(ok_value(&a, label), want, "{label}");
    }
    // "interest rate euro area" names two subjects (rate and yield things): it is not stored, so it is unbound
    for label in ["interest rate euro area", "nothing at all"] {
        let a = ask(&m, &path(&format!("{}[\"{label}\"].label()", subject_class()), None)).await;
        assert_eq!(
            (a.status, a.outcome(), a.error()),
            (404, "not_covered", Some("E_NAME_UNBOUND")),
            "{label}: {}",
            a.text
        );
    }
    // a bracketed label is folded by the mesh, and reads the object's method
    for label in [
        "Interest Rate · RATE · Euro  Area · Finance",
        "rate interest rate euro area",
        "  Rate Interest Rate   Euro Area ",
    ] {
        let a = ask(&m, &path(&format!("{}[\"{label}\"].label()", subject_class()), None)).await;
        assert_eq!(ok_value(&a, label), "interest rate · rate · euro area · finance", "{label}");
    }
}

// ---- catalog/facet -----------------------------------------------------------------------------------------------------

/// The vocabulary: each (facet, value) of the entries is an object (`exists()` true) with its label (the value with `_` as a
/// space, regions written out) and the number of subjects that have it (the four keys; `frequency` is not counted here).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c4_the_facet_objects_are_the_vocabulary() {
    let lite = vocab("c4-vocab");
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    let labels = [
        ("domain", "finance", "finance"),
        ("domain", "earth", "earth"),
        ("thing", "rate", "rate"),
        ("measure", "interest_rate", "interest rate"),
        ("measure", "exchange_rate", "exchange rate"),
        ("region", "ea", "euro area"),
        ("region", "gb", "United Kingdom"),
        ("region", "us", "United States"),
        ("region", "world", "world"),
        ("frequency", "on_change", "on change"),
        ("frequency", "daily", "daily"),
    ];
    for (f, v, label) in labels {
        let a = cell(&m, &fc(f, v, "label()")).await;
        assert_eq!(ok_value(&a, &format!("{f} {v}")), label, "{f} {v}");
        let e = cell(&m, &fc(f, v, "exists()")).await;
        assert_eq!(truth(&e), Some(true), "{f} {v}: {}", e.text);
    }
    // the counts of subjects (world V: five finance subjects, three in the euro area, ...)
    for (f, v, n) in [
        ("domain", "finance", "5"),
        ("domain", "earth", "1"),
        ("thing", "rate", "2"),
        ("thing", "yield", "2"),
        ("measure", "interest_rate", "3"),
        ("measure", "exchange_rate", "1"),
        ("region", "ea", "3"),
        ("region", "gb", "1"),
        ("region", "us", "1"),
        ("region", "world", "1"),
    ] {
        let a = cell(&m, &fc(f, v, "subjects()")).await;
        assert_eq!(ok_value(&a, &format!("{f} {v}")), n, "{f} {v}");
    }
    // a value no entry has is no object
    let a = cell(&m, &fc("region", "de", "exists()")).await;
    assert_eq!((a.status, a.outcome()), (404, "not_covered"), "{}", a.text);
}

// ---- the rank ----------------------------------------------------------------------------------------------------------

async fn ranked_served(label: &str) -> Served {
    let lite = ranked(label);
    build(&lite, T3);
    serve(lite, &[]).await
}

/// The whole order of world R: level 1 (the sub-domain, bound at 12:00:01, then the source's own domain, bound at 12:00:02),
/// level 2 (the relay bound at 11:00; the two views bound at 11:59:59, double before quad by name; the class view bound at
/// 12:00:00), level 3 (the third party, bound first of all at 10:00).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c5_the_rank_is_level_then_first_binding_time_then_name() {
    let m = ranked_served("c5-rank").await;
    let got = sources_of(&m, "finance", "rate", "interest_rate", "ea").await;
    assert_eq!(got, ranked_names(), "the sources of the subject, rank 1 first");
    assert_eq!(ok_value(&cell(&m, &sc("finance", "rate", "interest_rate", "ea", "sources()")).await, "sources()"), "7");
}

/// The level beats the binding time: the third party's name was bound first of all and is last; the source's own domain was
/// bound last of all and is first.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c5_the_level_beats_the_binding_time() {
    let m = ranked_served("c5-level").await;
    let got = sources_of(&m, "finance", "rate", "interest_rate", "ea").await;
    let at = |n: &str| got.iter().position(|x| x == n).unwrap_or_else(|| panic!("{n} is not a source: {got:?}"));
    assert_eq!(at(&agg()), got.len() - 1, "level 3 last: {got:?}");
    assert!(
        at(&own_domain()) < at(&relay_policy()) && at(&sub_domain()) < at(&relay_policy()),
        "level 1 before the relay: {got:?}"
    );
    assert!(at(&relay_policy()) < at(&agg()), "{got:?}");
}

/// Within a level the first binding time comes before the name: the relay (`.../relay/...`) sorts after the views
/// (`.../finance/...`) by name and is before them because it was bound earlier; the class view `policy_book` sorts before
/// `policy_double` and is after it, bound later.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c5_within_a_level_the_binding_time_beats_the_name() {
    let m = ranked_served("c5-time").await;
    let got = sources_of(&m, "finance", "rate", "interest_rate", "ea").await;
    let at = |n: &str| got.iter().position(|x| x == n).unwrap();
    assert!(at(&relay_policy()) < at(&view_double()), "{got:?}");
    assert!(at(&view_double()) < at(&view_book()), "{got:?}");
    assert!(at(&view_quad()) < at(&view_book()), "{got:?}");
}

/// A tie on the binding time is broken by the name, ascending: `policy_double` and `policy_quad` were bound in one file at
/// one instant.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c5_a_tie_on_binding_time_is_broken_by_name() {
    let lite = ranked("c5-tie");
    assert_eq!(lite.bound_tx(&view_double()), lite.bound_tx(&view_quad()), "the fixture: one binding instant");
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    let got = sources_of(&m, "finance", "rate", "interest_rate", "ea").await;
    let at = |n: &str| got.iter().position(|x| x == n).unwrap();
    assert_eq!(at(&view_quad()), at(&view_double()) + 1, "{got:?}");
}

/// The views of a relay: a function that reads it (double), one that reads a view (quad: through another view), and a class
/// whose method reads it (policy_book) are all level 2, so all are before the third party and after the own-domain sources.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c5_views_are_level_two_directly_through_views_and_in_a_class() {
    let m = ranked_served("c5-views").await;
    let got = sources_of(&m, "finance", "rate", "interest_rate", "ea").await;
    let at = |n: &str| got.iter().position(|x| x == n).unwrap();
    for v in [view_double(), view_quad(), view_book()] {
        assert!(at(&own_domain()) < at(&v) && at(&sub_domain()) < at(&v), "{v} after level 1: {got:?}");
        assert!(at(&v) < at(&agg()), "{v} before level 3: {got:?}");
    }
}

/// A sub-domain of the source domain (`data.ecb.europa.eu` of `ecb.europa.eu`) is a source under its own domain: level 1.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c5_a_publisher_under_a_parent_domain_is_level_one() {
    let m = ranked_served("c5-subdomain").await;
    let got = sources_of(&m, "finance", "rate", "interest_rate", "ea").await;
    assert_eq!(&got[..2], [sub_domain(), own_domain()], "{got:?}");
}

/// A subject whose only source is a relay has it as rank 1; another subject's sources do not mix in.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c5_a_relay_alone_is_rank_one_of_its_subject() {
    let m = ranked_served("c5-alone").await;
    let got = sources_of(&m, "finance", "rate", "interest_rate", "gb").await;
    assert_eq!(got, [relay_bank()]);
}

/// The ranks of world S (the scratch mesh): `ratio/pe` was bound before `calendar/day`, both non-relayed, so it ranks first
/// though its name sorts after.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c5_the_scratch_world_ranks_as_its_binding_times_say() {
    let lite = scratch("c5-scratch");
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    for s in s_subjects() {
        let got = sources_of(&m, s.domain, s.thing, s.measure, s.region).await;
        assert_eq!(got, s.sources, "{} {} {} {}", s.domain, s.thing, s.measure, s.region);
    }
}

// ---- reruns ------------------------------------------------------------------------------------------------------------

/// A rerun with nothing changed writes nothing: no cell, and not a byte of the store, at the same instant and at a later one.
#[test]
fn c5_a_rerun_with_nothing_changed_writes_nothing() {
    let lite = vocab("c5-rerun");
    let first = build(&lite, T3);
    assert!(cells(&first) > 0, "the first run writes cells: {first:?}");
    let before = snapshot(&lite);
    for clock in [T3, T4, T5] {
        let again = build(&lite, clock);
        assert_eq!(cells(&again), 0, "a rerun at {clock}: {again:?}");
        assert_eq!(snapshot(&lite), before, "a rerun at {clock} wrote something");
    }
    // the catalogue lines republished unchanged change nothing either, and neither does the next run
    let f = write(&lite, "same.jsonl", &entries_lines(&v_entries()));
    common::pub031::run(&lite, &f, T4).unwrap_or_else(|e| panic!("{e}"));
    let again = build(&lite, T5);
    assert_eq!(cells(&again), 0, "{again:?}");
}

/// A change writes new versions (and only for what changed): an entry's region changes from `gb` to `us`; the new subject
/// exists, the old one does not, and the next run writes nothing again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c5_a_change_writes_new_versions_and_a_second_run_nothing() {
    let lite = vocab("c5-change");
    build(&lite, T3);
    let bank = host_name("relay/bankofengland.co.uk/bank_rate");
    let f = write(
        &lite,
        "change.jsonl",
        &[cat_line(&bank, fac(&["finance"], &["rate"], &["interest_rate"], &["us"], &["daily"]))],
    );
    common::pub031::run(&lite, &f, T4).unwrap_or_else(|e| panic!("{e}"));
    let before_change = snapshot(&lite);
    let second = build(&lite, T5);
    assert!(cells(&second) > 0, "{second:?}");
    assert_ne!(snapshot(&lite), before_change);
    let after = snapshot(&lite);
    assert_eq!(cells(&build(&lite, T6)), 0);
    assert_eq!(snapshot(&lite), after);
    let m = serve(lite, &[]).await;
    let new = cell(&m, &sc("finance", "rate", "interest_rate", "us", "exists()")).await;
    assert_eq!(truth(&new), Some(true), "{}", new.text);
    assert_eq!(ok_value(&cell(&m, &sc("finance", "rate", "interest_rate", "us", "source(1)")).await, "source"), bank);
    let old = cell(&m, &sc("finance", "rate", "interest_rate", "gb", "exists()")).await;
    assert_eq!(truth(&old), Some(false), "{}", old.text);
}

/// A removed facet: the entry is republished without its facets, so its only subject disappears: `exists()` false, every label
/// of it withdrawn (its lookup cells hold `""`, so a bracketed label is `E_NAME_UNBOUND`; as of an instant before the run it
/// still resolves, the history stays), and the vocabulary that depended on it follows (`gb` has no object any more, the
/// counts of the others are new versions).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c5_a_removed_facet_sets_exists_false_and_withdraws_the_labels() {
    let lite = vocab("c5-removed");
    build(&lite, T3);
    let bank = host_name("relay/bankofengland.co.uk/bank_rate");
    let f = write(&lite, "drop.jsonl", &[catalog_line_of(&bank, "function", json!({}))]);
    common::pub031::run(&lite, &f, T4).unwrap_or_else(|e| panic!("{e}"));
    let run = build(&lite, T5);
    assert!(cells(&run) > 0, "{run:?}");
    let m = serve(lite, &[]).await;
    let (d, t, me, r) = ("finance", "rate", "interest_rate", "gb");
    let e = cell(&m, &sc(d, t, me, r, "exists()")).await;
    assert_eq!((e.status, e.outcome(), truth(&e)), (200, "ok", Some(false)), "{}", e.text);
    let labels = [
        "interest rate · rate · united kingdom · finance",
        "interest rate united kingdom",
        "rate interest rate united kingdom",
    ];
    for label in labels {
        let lookup = format!("/cell/{}/from_label?label={}", subject_class(), common::world::qenc(label));
        let a = ask(&m, &lookup).await;
        assert_eq!((a.outcome(), a.value()), ("ok", Some("")), "{label}: withdrawn, not deleted: {}", a.text);
        let b = ask(&m, &path(&format!("{}[\"{label}\"].label()", subject_class()), None)).await;
        assert_eq!((b.status, b.error()), (404, Some("E_NAME_UNBOUND")), "{label}: {}", b.text);
        // as of before the run, the label was there
        let was = ask(&m, &path(&format!("{}[\"{label}\"].label()", subject_class()), Some(T4))).await;
        assert_eq!(was.outcome(), "ok", "{label} as of {T4}: {}", was.text);
    }
    // the vocabulary: gb has no entry now; the counts moved
    let gb = cell(&m, &fc("region", "gb", "exists()")).await;
    assert_eq!(truth(&gb), Some(false), "{}", gb.text);
    assert_eq!(ok_value(&cell(&m, &fc("domain", "finance", "subjects()")).await, "domain finance"), "4");
    assert_eq!(ok_value(&cell(&m, &fc("measure", "interest_rate", "subjects()")).await, "measure interest_rate"), "2");
    // the other subjects are untouched
    let other = cell(&m, &sc("finance", "rate", "interest_rate", "ea", "exists()")).await;
    assert_eq!(truth(&other), Some(true));
}

// ---- the rule text -----------------------------------------------------------------------------------------------------

/// The rule is published as the page `/catalog/rule` and as the cell `catalog/rule()`, a text blob, with the same words.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c5_the_rule_is_a_page_and_a_cell_with_the_same_words() {
    let lite = vocab("c5-rule");
    build(&lite, T3);
    let m = serve(lite, &[]).await;
    let a = cell(&m, &format!("{}()", rule_fn())).await;
    assert_eq!((a.status, a.outcome()), (200, "ok"), "{}", a.text);
    let lexical = a.value().unwrap_or_else(|| panic!("no value: {}", a.text)).to_string();
    // SPEC 3: a blob's lexical form is `<sha256>:<length>:<media type>`.
    let mut parts = lexical.splitn(3, ':');
    let (sha, len, media) = (parts.next().unwrap(), parts.next(), parts.next());
    assert!(
        sha.len() == 64 && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        "a blob's value starts with its SHA-256 in lowercase hex: {lexical}"
    );
    let len: usize =
        len.and_then(|l| l.parse().ok()).unwrap_or_else(|| panic!("a blob's value has its length: {lexical}"));
    assert!(media.is_some_and(|m| m.contains('/')), "a blob's value ends with its media type: {lexical}");
    let (status, bytes) = m.raw(&format!("/blob/{sha}")).await;
    assert_eq!(status, 200);
    assert_eq!(bytes.len(), len, "the length in the value is the content's");
    assert_eq!(datumesh_core::sha256_hex(&bytes), sha, "the sha part is the SHA-256 of the words the page holds");
    let rule = String::from_utf8(bytes).unwrap();
    let rule_words = words(&rule);
    assert!(rule_words.len() >= 30, "the rule says how the order is made: {rule}");
    let p = page(&m, "/catalog/rule").await;
    assert_eq!(p.status, 200, "{}", p.text());
    assert!(p.header("content-type").unwrap_or_default().starts_with("text/html"), "{:?}", p.header("content-type"));
    let page_words = words(&plain(&p.text()));
    // the page holds the rule's words, in order, as one run
    let rule_plain = words(&plain(&rule));
    assert!(
        page_words.windows(rule_plain.len()).any(|w| w == rule_plain.as_slice()),
        "the page does not hold the cell's words.\n cell: {rule_plain:?}\n page: {page_words:?}"
    );
    // and the rule names its three levels
    let lower = rule.to_lowercase();
    for w in ["relay", "view", "first"] {
        assert!(lower.contains(w), "the rule does not mention {w:?}: {rule}");
    }
}

// ---- the exception does not outlive the run ----------------------------------------------------------------------------

/// After the job (which writes under `<host>/catalog`), `publish-stored` still refuses a name there, a method of the job's own
/// class and a re-definition of it included (C1: the job flag is not a standing exception).
#[test]
fn c1_the_job_flag_is_not_a_standing_exception() {
    let lite = vocab("c1-after-job");
    build(&lite, T3);
    assert!(lite.store.resolve_class(&subject_class(), END).unwrap().is_some(), "the job wrote its class");
    for name in [
        format!("{HOST}/catalog/subject/extra"),
        format!("{HOST}/catalog/other"),
        format!("{HOST}/catalog/subject/source"),
    ] {
        let f = write(&lite, "x.jsonl", &[common::pub031::function_line(&name, &common::pub031::rate_sig(), None)]);
        let before = snapshot(&lite);
        let e = common::pub031::run(&lite, &f, T4).expect_err(&format!("{name} must be refused"));
        assert!(e.detail.contains("E_NAME_RESERVED"), "{name}: {e}");
        assert_eq!(snapshot(&lite), before, "{name}");
    }
}

// ---- the command -------------------------------------------------------------------------------------------------------

/// `datumesh-mesh catalog-build`: no flag; DATABASE_URL, DATUMESH_HOST, DATUMESH_KEY_FILE and DATUMESH_LICENCE; one JCS line
/// `{"cells":N,...}`. A first run writes, a rerun writes nothing. (A PostgreSQL store, the system clock.)
#[test]
fn c5_the_command_writes_and_reruns_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let db = TestDb::migrated();
    let store = db.store(4);
    let key = key_for(HOST);
    store.put_namespace(HOST, &datumesh_http::keydoc::with_key(HOST, None, &key, K_FROM).unwrap(), K_FROM).unwrap();
    let dir = out_dir().join(format!("catalog-build-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let world = dir.join("world.jsonl");
    std::fs::write(&world, entries_lines(&v_entries()).iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    let owned = vec![HOST.to_string()];
    bl(|| datumesh_http::publish::publish_stored_with(&store, &key, HOST, &owned, &world, &common::pub031::opts(NOW)))
        .unwrap_or_else(|e| panic!("world: {e}"));
    let key_file = dir.join("mesh.key");
    std::fs::write(&key_file, format!("{}\n", datumesh_core::lexical::to_hex(&seed_for(HOST)))).unwrap();
    std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let run = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_datumesh-mesh"))
            .args(["catalog-build"])
            .env("DATABASE_URL", db.url())
            .env("DATUMESH_HOST", HOST)
            .env("DATUMESH_KEY_FILE", &key_file)
            .env("DATUMESH_LICENCE", LICENCE)
            .output()
            .unwrap()
    };
    let out = run();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{stdout} {}", String::from_utf8_lossy(&out.stderr));
    let first: Value =
        serde_json::from_str(stdout.lines().last().unwrap_or("")).unwrap_or_else(|e| panic!("{e}: {stdout}"));
    assert!(first["cells"].as_u64().unwrap_or(0) > 0, "{stdout}");
    assert!(bl(|| store.resolve_class(&subject_class(), END)).unwrap().is_some(), "the class is bound");
    assert!(bl(|| store.resolve_class(&facet_class(), END)).unwrap().is_some(), "the class is bound");
    let out = run();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{stdout} {}", String::from_utf8_lossy(&out.stderr));
    let second: Value =
        serde_json::from_str(stdout.lines().last().unwrap_or("")).unwrap_or_else(|e| panic!("{e}: {stdout}"));
    assert_eq!(second["cells"], json!(0), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}
