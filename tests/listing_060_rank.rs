//! http 0.6.0, the listing release (CONTRACT-http-listing.md L4): the catalogue job reads listings. A listed name joins its
//! subject by the subject keys of its listing ("bound here" becomes "bound here or listed"), and the rule decides its rank,
//! unchanged: level 1 when the publisher's host (the listed host) is the source's domain (the relay path's domain) or a
//! sub-domain of it; a parent domain or any other host is level 3, below the relay (level 2).
//!
//! The listings are written with the store's `put_listing` (no network here). Failing on the test commit: the store's
//! listing calls are `Unsupported` there, and the job does not read listings.
//!
//! Within a level the order is the rule's: a listed name's "first bound" is its listing's `listed_at` (the test writer's
//! reading; the contract does not say), then the name.

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
use crworld::*;
use datumesh_store::SubjectKeys;
use listing060::*;

/// The relay's domain is `bank.source.example`; its parent `source.example` is not the source's domain.
const DOMAIN: &str = "bank.source.example";
const L_A: &str = "2026-11-15T12:00:01.000000Z";
const L_B: &str = "2026-11-15T12:00:02.000000Z";
const L_C: &str = "2026-11-15T12:00:03.000000Z";
const L_D: &str = "2026-11-15T12:00:04.000000Z";
const BUILD: &str = "2026-11-15T12:00:05.000000Z";

fn relay_of_domain() -> String {
    host_name(&format!("relay/{DOMAIN}/bank_rate"))
}

/// A relay of `bank.source.example` in GB, and four listed names in GB: the source's domain, a sub-domain, the parent
/// domain, another host. Listed in the order own, sub, parent, other.
fn world(label: &str) -> Lite {
    let lite = Lite::new(label);
    publish(&lite, "relay.jsonl", &relay_lines(&relay_of_domain(), gb_facets(), "Bank Rate"), T_RELAY)
        .unwrap_or_else(|e| panic!("relay: {e}"));
    put_listing(&lite, DOMAIN, &format!("{DOMAIN}/bank_rate"), vec![gb()], L_A);
    put_listing(&lite, &format!("stats.{DOMAIN}"), &format!("stats.{DOMAIN}/rates/bank_rate"), vec![gb()], L_B);
    put_listing(&lite, "source.example", "source.example/bank_rate", vec![gb()], L_C);
    put_listing(&lite, "other.example", "other.example/bank_rate", vec![gb()], L_D);
    lite
}

/// Level 1 for the source's domain and a sub-domain, then the relay (level 2), then level 3 for the parent domain and the
/// other host; within a level by the time listed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l4_listed_names_rank_by_the_rule_in_a_subject_with_a_relay() {
    let lite = world("l060-rank");
    let built = build(&lite, BUILD);
    assert!(built.cells > 0);
    let m = serve(lite, &[]).await;
    assert_eq!(
        sources_at(&m, GB).await,
        [
            format!("{DOMAIN}/bank_rate"),
            format!("stats.{DOMAIN}/rates/bank_rate"),
            relay_of_domain(),
            "source.example/bank_rate".to_string(),
            "other.example/bank_rate".to_string(),
        ]
    );
}

/// The example of L4: `<source domain>/...` in the subject of `<host>/relay/<source domain>/bank_rate` ranks above the relay;
/// `source(1)` is the listed name.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l4_the_sources_own_function_ranks_above_the_relay() {
    let lite = listing_lite("l060-rank-example");
    put_listing(&lite, SRC, &bank_rate(), vec![gb()], L_A);
    build(&lite, BUILD);
    let m = serve(lite, &[]).await;
    assert_eq!(sources_at(&m, GB).await, [bank_rate(), relay()]);
    let one = source_at(&m, GB, 1).await;
    assert_eq!(one.value(), Some(bank_rate().as_str()), "{}", one.text);
}

/// "Same source" means the same subject and nothing else (answer 2): a listed name in another subject (region `ea`) does
/// not join GB, and alone in its subject (no relay, no source domain) it is level 1 under its own host.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l4_a_listed_name_joins_only_its_own_subjects() {
    let lite = listing_lite("l060-rank-subject");
    let ea = SubjectKeys::new("finance", "rate", "interest_rate", "ea");
    put_listing(&lite, SRC, &format!("{SRC}/euro_rate"), vec![ea], L_A);
    build(&lite, BUILD);
    let m = serve(lite, &[]).await;
    assert_eq!(sources_at(&m, GB).await, [relay()], "GB is the relay's alone");
    assert_eq!(sources_at(&m, ("finance", "rate", "interest_rate", "ea")).await, [format!("{SRC}/euro_rate")]);
}

/// A listed name in two subjects joins both, by the keys of its listing (not by the cross product of its facets).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn l4_a_listed_name_joins_each_subject_of_its_listing_and_no_other() {
    let lite = listing_lite("l060-rank-two");
    let us_cpi = SubjectKeys::new("finance", "price", "cpi", "us");
    // facets would be domain finance, thing price+rate, measure cpi+interest_rate, region gb+us: four subjects; the
    // listing names two.
    put_listing(&lite, SRC, &format!("{SRC}/both"), vec![gb(), us_cpi], L_A);
    build(&lite, BUILD);
    let m = serve(lite, &[]).await;
    assert_eq!(sources_at(&m, GB).await, [format!("{SRC}/both"), relay()]);
    assert_eq!(sources_at(&m, ("finance", "price", "cpi", "us")).await, [format!("{SRC}/both")]);
    let cross = ask(&m, &path(&sc("finance", "price", "interest_rate", "gb", "exists()"), Some(common::readpath::MESH_NOW))).await;
    assert_ne!(cross.outcome(), "ok", "no subject from the cross product of the facets: {}", cross.text);
}
