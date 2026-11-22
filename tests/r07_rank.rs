//! The 0.7 round (CONTRACT-round-0.7.md), R5: rank, a subclass above its parent. Failing tests.
//!
//! "Within a level, an entry whose class is a subclass (at any depth) of another entry's class in the same subject ranks above
//! it. Then the first binding, then the name." The rule text (`/catalog/rule`, `catalog/rule()`) says so.
//!
//! Every world here has a parent bound before its subclass, so that today's order (binding time) puts the parent first and
//! these tests fail on it; the name of the subclass sorts before the parent's only where a test says so.
//!
//! The levels are those of C5: a subject with no relay among its entries has all its entries at level 1; with a relay, the
//! relay and the views built on it are level 2, and a name that reads no relay is level 3.

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/r07world.rs"]
mod r07world;

use catjob::*;
use catworld::*;
use common::lite::*;
use common::pub031::{class_line, function_line, BEFORE, T1, T2};
use common::readpath::*;
use crworld::*;
use r07world::*;
use serde_json::json;

async fn served(lite: Lite) -> Served {
    build(&lite, T3);
    serve(lite, &[]).await
}

fn p() -> String {
    host_name("fam/parent")
}
fn c() -> String {
    host_name("fam/child")
}
fn g() -> String {
    host_name("fam/grandchild")
}

/// A parent bound first, its subclass later: the subclass is rank 1, though the binding order alone would give the parent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r5_a_subclass_ranks_above_its_parent_within_a_level() {
    let lite = family(
        "r5-child",
        &[fam("fam/parent", None, "", "rate", NOW), fam("fam/child", Some(0), "fixing", "fixed", T1)],
        &[],
    );
    let m = served(lite).await;
    assert_eq!(sources_list(&m, FAM).await, [c(), p()]);
    let one = source_of(&m, FAM.0, FAM.1, FAM.2, FAM.3, 1).await;
    assert_eq!(one.as_deref(), Some(c().as_str()), "source(1) is the subclass");
}

/// The name does not decide it: with the subclass's name sorting after the parent's and the same binding instant, the subclass is
/// still first.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r5_the_subclass_rule_comes_before_the_binding_time_and_the_name() {
    let lite = family(
        "r5-same-time",
        &[fam("fam/a_parent", None, "", "rate", NOW), fam("fam/z_child", Some(0), "fixing", "fixed", NOW)],
        &[],
    );
    let m = served(lite).await;
    assert_eq!(sources_list(&m, FAM).await, [host_name("fam/z_child"), host_name("fam/a_parent")]);
}

/// A grandchild is above its parent and its grandparent (the rule holds at any depth); the child is above the parent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r5_a_grandchild_ranks_above_both() {
    let lite = family(
        "r5-grandchild",
        &[
            fam("fam/parent", None, "", "rate", NOW),
            fam("fam/child", Some(0), "", "fixed", T1),
            fam("fam/grandchild", Some(1), "extra", "more", T2),
        ],
        &[],
    );
    let m = served(lite).await;
    assert_eq!(sources_list(&m, FAM).await, [g(), c(), p()]);
}

/// Any depth: the grandchild is above the grandparent even when the child in between is not a source of the subject.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r5_the_grandchild_is_above_the_grandparent_without_the_child_between() {
    let lite = family(
        "r5-skip",
        &[
            fam("fam/parent", None, "", "rate", NOW),
            fam("fam/child", Some(0), "", "fixed", T1),
            fam("fam/grandchild", Some(1), "extra", "more", T2),
        ],
        &[],
    );
    // the child's entry loses its facets: it is no source of the subject
    let drop = common::pub031::write(&lite, "drop.jsonl", &[catalog_line_of(&c(), "class", json!({}))]);
    common::pub031::run(&lite, &drop, T2).unwrap_or_else(|e| panic!("{e}"));
    let m = served(lite).await;
    assert_eq!(sources_list(&m, FAM).await, [g(), p()]);
}

/// Classes that are not related keep the binding order, then the name (today's rule): the later-bound one with the earlier name
/// is after.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r5_unrelated_classes_keep_the_binding_order() {
    let lite = family(
        "r5-unrelated",
        &[fam("fam/zebra", None, "", "rate", NOW), fam("fam/aardvark", None, "", "rate", T1)],
        &[],
    );
    let m = served(lite).await;
    assert_eq!(sources_list(&m, FAM).await, [host_name("fam/zebra"), host_name("fam/aardvark")]);
}

/// A mix (the unrelated class has a method of another name, so its class hash differs from the parent's): an unrelated class bound first stays first; the subclass is above its parent but not above what it is unrelated to:
/// `[unrelated (bound first), child, parent]`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r5_an_unrelated_class_keeps_its_place_between_the_family() {
    let lite = family(
        "r5-mixed",
        &[
            fam("fam/other", None, "", "other", NOW),
            fam("fam/parent", None, "", "rate", T1),
            fam("fam/child", Some(1), "fixing", "fixed", T2),
        ],
        &[],
    );
    let m = served(lite).await;
    assert_eq!(sources_list(&m, FAM).await, [host_name("fam/other"), c(), p()]);
}

// ---- levels ----------------------------------------------------------------------------------------------------------

fn num_key() -> &'static str {
    r#"{"n":{"type":"number","unit":"1"}}"#
}
fn k2_key() -> &'static str {
    r#"{"k2":{"type":"number","unit":"1"}}"#
}
const NUM_ARG: &str = r#"{"type":"number","unit":"1","valid":false}"#;

fn sig(args: &[(&str, &str)], order: &[&str]) -> String {
    let a: Vec<String> = args.iter().map(|(k, t)| format!("\"{k}\":{t}")).collect();
    let o: Vec<String> = order.iter().map(|k| format!("\"{k}\"")).collect();
    format!(r#"{{"args":{{{}}},"order":[{}],"returns":{NUM1}}}"#, a.join(","), o.join(","))
}

/// The spec's example: `sec/company` (a class with a method that reads the relay class's method, so a view of it, level 2) above
/// `relay/sec.gov/company` (the relay, level 2) on the same subject. The relay class was bound first.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r5_sec_company_is_above_relay_sec_gov_company_at_level_two() {
    let lite = Lite::new("r5-sec");
    let relay = host_name("relay/sec.gov/company");
    let sec = host_name("sec/company");
    let filed = format!("{relay}/filed");
    let view = format!("{sec}/view");
    let facets = fam_facets();
    publish(
        &lite,
        "a.jsonl",
        &[function_line(&filed, &sig(&[("n", NUM_ARG), ("date", DATE_V)], &["n", "date"]), None)],
        BEFORE,
    )
    .unwrap_or_else(|e| panic!("relay function: {e}"));
    let rd = common::world::class_json(num_key(), &["n"], &[("filed", &fn_hash(&lite, &filed))], HOST, "");
    publish(
        &lite,
        "relay.jsonl",
        &[class_line(&relay, &rd, None), catalog_line_of(&relay, "class", json!({"facets": facets.clone()}))],
        NOW,
    )
    .unwrap_or_else(|e| panic!("relay class: {e}"));
    let parent_hash = datumesh_core::ClassDescriptor::from_json(rd.as_bytes()).unwrap().hash();
    publish(
        &lite,
        "view.jsonl",
        &[formula_line(
            &view,
            &format!("{filed}(n: n, date: date) * k2"),
            &sig(&[("n", NUM_ARG), ("k2", NUM_ARG), ("date", DATE_V)], &["n", "k2", "date"]),
        )],
        T1,
    )
    .unwrap_or_else(|e| panic!("view: {e}"));
    let cd = common::world::class_json(
        k2_key(),
        &["k2"],
        &[("view", &fn_hash(&lite, &view))],
        HOST,
        &format!(r#""parent":"{parent_hash}","#),
    );
    publish(
        &lite,
        "sec.jsonl",
        &[class_line(&sec, &cd, None), catalog_line_of(&sec, "class", json!({"facets": facets}))],
        T2,
    )
    .unwrap_or_else(|e| panic!("sec/company: {e}"));
    let m = served(lite).await;
    assert_eq!(sources_list(&m, FAM).await, [sec, relay]);
}

/// A subclass does not leave its level: the parent reads the relay (level 2), the subclass reads nothing (level 3), so the
/// subclass is below, after the relay and its parent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r5_a_subclass_in_a_lower_level_stays_below_the_parent() {
    let lite = Lite::new("r5-levels");
    let relay = host_name("relay/ecb.europa.eu/fam_rate");
    let book = host_name("fam/book");
    let sub = host_name("fam/sub");
    let rate = format!("{book}/rate");
    let own = format!("{sub}/own");
    let facets = fam_facets();
    let mut a = entries_lines(&[(relay.clone(), facets.clone())]);
    a.push(function_line(&own, &sig(&[("n", NUM_ARG), ("k2", NUM_ARG)], &["n", "k2"]), None));
    publish(&lite, "a.jsonl", &a, BEFORE).unwrap_or_else(|e| panic!("relay: {e}"));
    publish(
        &lite,
        "f.jsonl",
        &[formula_line(
            &rate,
            &format!("{relay}(date: date) * n"),
            &sig(&[("n", NUM_ARG), ("date", DATE_V)], &["n", "date"]),
        )],
        BEFORE,
    )
    .unwrap_or_else(|e| panic!("view: {e}"));
    let bd = common::world::class_json(num_key(), &["n"], &[("rate", &fn_hash(&lite, &rate))], HOST, "");
    publish(
        &lite,
        "book.jsonl",
        &[class_line(&book, &bd, None), catalog_line_of(&book, "class", json!({"facets": facets.clone()}))],
        NOW,
    )
    .unwrap_or_else(|e| panic!("book: {e}"));
    let h = datumesh_core::ClassDescriptor::from_json(bd.as_bytes()).unwrap().hash();
    let sd = common::world::class_json(
        k2_key(),
        &["k2"],
        &[("own", &fn_hash(&lite, &own))],
        HOST,
        &format!(r#""parent":"{h}","#),
    );
    publish(
        &lite,
        "sub.jsonl",
        &[class_line(&sub, &sd, None), catalog_line_of(&sub, "class", json!({"facets": facets}))],
        T1,
    )
    .unwrap_or_else(|e| panic!("sub: {e}"));
    let m = served(lite).await;
    assert_eq!(sources_list(&m, FAM).await, [relay, book, sub]);
}

// ---- the rule text ---------------------------------------------------------------------------------------------------

/// The rule text says so, on the page and in the cell: the words "subclass" and "parent" are in both, and the two are the same
/// words.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn r5_the_rule_text_says_a_subclass_ranks_above_its_parent() {
    let lite = family(
        "r5-rule",
        &[fam("fam/parent", None, "", "rate", NOW), fam("fam/child", Some(0), "fixing", "fixed", T1)],
        &[],
    );
    let m = served(lite).await;
    let r = until_built(&m, "/catalog/rule").await;
    assert_eq!(r.status, 200, "{}", r.text());
    let text = plain(&r.text()).to_lowercase();
    for w in ["subclass", "parent"] {
        assert!(text.contains(w), "the rule page has no {w:?}: {text}");
    }
    let a = cell(&m, &format!("{}()", rule_fn())).await;
    assert_eq!((a.status, a.outcome()), (200, "ok"), "{}", a.text);
}
