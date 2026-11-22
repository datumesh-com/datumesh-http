//! http 0.4.10, CONTRACT-round-0.7-P HP1 to HP4: describe never indexes on a request, and the object index is
//! built from the declared list. Written first; FAIL until then. (Store 0.5.3 is not needed: these tests
//! use the 0.5.2 API; the counting ones are in `h0410_counts.rs`.)
//!
//! What the tests pin where the contract leaves a detail open:
//!
//! * The reason of a hex id that the last build does not hold is the text `not in the object index yet` in the
//!   body of the `404` (HP1: "404 with the reason ..."), compared in lower case.
//! * A cell id is not an object id any more: HP2 "the `cells` map goes: an object's id comes from its keys, not
//!   from a cell id". `GET /describe/<64 or 12 hex of a method cell>` is `404`.
//! * A class is made known to the index by a configured alias to one of its methods (the build hears of those
//!   classes at its start). What a request that names a class does to the next build is not pinned.
//! * `DATUMESH_INDEX_CLASS_MAX` and `DATUMESH_INDEX_MAX` count objects; which objects a truncated class keeps is
//!   not pinned, only how many; the others are reached by key encoding.
//! * `datumesh_object_index_truncated_total`: a counter; its value (summed over labels, if it has any) is at
//!   least the number of classes that were cut in the last build, and it is absent or 0 when none was.

mod common;
#[path = "common/h0410.rs"]
mod h0410;

use common::pub031::T2;
use h0410::*;

const A: &str = "mesh-test.datumesh.com/h0410/idx/alpha";
const B: &str = "mesh-test.datumesh.com/h0410/idx/beta";

async fn serve_with(fx: Fx, vars: &[(&str, &str)], classes: &[(&str, &str)]) -> W {
    let aliases: Vec<(String, String)> =
        classes.iter().enumerate().map(|(i, (c, m))| (format!("idx{i}"), format!("{c}/{m}"))).collect();
    let a: Vec<(&str, &str)> = aliases.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    fx.serve(config(vars, &a)).await
}

/// A class `A` with the extent `members` (objects E00000 to E00005), `rate` (objects R00000 to R00003 and
/// E00000) and `note` (objects N00000 to N00003).
fn extent_class(label: &str) -> (Fx, Cls) {
    let fx = Fx::lite(label);
    let cls = define_class(&fx, A, &[("members", Kind::Extent), ("rate", Kind::Rate), ("note", Kind::Note)]);
    let mut lines = cells_of(&cls, "members", Kind::Extent, "E", 0..6);
    lines.extend(cells_of(&cls, "rate", Kind::Rate, "R", 0..4));
    lines.extend(cells_of(&cls, "rate", Kind::Rate, "E", 0..1));
    lines.extend(cells_of(&cls, "note", Kind::Note, "N", 0..4));
    fx.publish(&lines, T2);
    (fx, cls)
}

fn named(i: usize, p: &str) -> String {
    format!("{p}{i:05}")
}

// ---- HP2: the extent.keys objects only ------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_index_holds_the_extent_keys_objects_and_no_others() {
    let (fx, cls) = extent_class("hp2-extent");
    let w = serve_with(fx, &[], &[(A, "members")]).await;
    let first = w.describe_until_found(&cls.id(&named(0, "E"), "Q")).await;
    assert_eq!(first.status, 200, "{}", first.text());
    for i in 0..6 {
        let r = w.get(&format!("/describe/{}", cls.id(&named(i, "E"), "Q"))).await;
        assert_eq!(r.status, 200, "E{i}: {}", r.text());
        let by_key = w.get(&format!("/describe/{}", common::world::call_path(&cls.encoding(&named(i, "E"), "Q")))).await;
        assert_eq!(r.body, by_key.body, "E{i}: the id and the key encoding answer the same");
    }
    // the objects of the other methods are not in the index: 404 with the reason; by key encoding they answer
    for (p, n) in [("R", 4), ("N", 4)] {
        for i in 0..n {
            let id = cls.id(&named(i, p), "Q");
            let r = w.get(&format!("/describe/{id}")).await;
            assert_eq!(r.status, 404, "{p}{i}: {}", r.text());
            assert!(says_not_in_index(&r.text()), "{p}{i}: the reason: {}", r.text());
            let short = w.get(&format!("/describe/{}", &id[..12])).await;
            assert_eq!(short.status, 404, "{p}{i} short: {}", short.text());
            let by_key = w.get(&format!("/describe/{}", common::world::call_path(&cls.encoding(&named(i, p), "Q")))).await;
            assert_eq!(by_key.status, 200, "{p}{i} by key: {}", by_key.text());
        }
    }
    // an id that is nobody's
    let r = w.get(&format!("/describe/{}", "ab".repeat(32))).await;
    assert_eq!(r.status, 404, "{}", r.text());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_class_without_extent_keys_is_indexed_from_its_first_method_in_declared_order() {
    let fx = Fx::lite("hp2-first");
    // the methods are declared in name order: `alpha` is the first, `omega` the second
    let cls = define_class(&fx, A, &[("alpha", Kind::Level), ("omega", Kind::Note)]);
    let mut lines = cells_of(&cls, "alpha", Kind::Level, "P", 0..3);
    lines.extend(cells_of(&cls, "omega", Kind::Note, "Q", 0..3));
    fx.publish(&lines, T2);
    let w = serve_with(fx, &[], &[(A, "alpha")]).await;
    let r = w.describe_until_found(&cls.id(&named(0, "P"), "Q")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    for i in 0..3 {
        let r = w.get(&format!("/describe/{}", cls.id(&named(i, "P"), "Q"))).await;
        assert_eq!(r.status, 200, "P{i}: {}", r.text());
        let r = w.get(&format!("/describe/{}", cls.id(&named(i, "Q"), "Q"))).await;
        assert_eq!(r.status, 404, "Q{i} is of the second method: {}", r.text());
        assert!(says_not_in_index(&r.text()), "{}", r.text());
    }
}

/// More than one page of 10,000 (`slice_values_page`): the objects on both sides of the boundary are held.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_build_reads_the_list_in_pages_and_loses_none() {
    let fx = Fx::lite("hp2-pages");
    let cls = define_class(&fx, A, &[("members", Kind::Extent)]);
    let n = 10_030;
    fx.publish(&cells_of(&cls, "members", Kind::Extent, "E", 0..n), T2);
    let w = serve_with(fx, &[], &[(A, "members")]).await;
    let r = w.describe_until_found(&cls.id(&named(0, "E"), "Q")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    // the first, the last, and every thousandth in between, on the last build (a build is one pass)
    let mut missing = Vec::new();
    for i in (0..n).step_by(1000).chain([n - 1, 9_999, 10_000, 10_001]) {
        let r = w.get(&format!("/describe/{}", cls.id(&named(i, "E"), "Q"))).await;
        if r.status != 200 {
            missing.push((i, r.status));
        }
    }
    assert!(missing.is_empty(), "objects not in the index: {missing:?}");
}

// ---- HP2: the caps and the metric -------------------------------------------------------------------------

async fn found_count(w: &W, cls: &Cls, prefix: &str, n: usize) -> usize {
    let mut k = 0;
    for i in 0..n {
        let r = w.get(&format!("/describe/{}", cls.id(&named(i, prefix), "Q"))).await;
        match r.status {
            200 => k += 1,
            404 => assert!(says_not_in_index(&r.text()), "{prefix}{i}: {}", r.text()),
            s => panic!("{prefix}{i}: {s}: {}", r.text()),
        }
    }
    k
}

async fn wait_for_metric(w: &W, want: f64) -> f64 {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let v = w.metric("datumesh_object_index_truncated_total").await;
        if v.is_some_and(|v| v >= want) || std::time::Instant::now() > end {
            return v.unwrap_or(-1.0);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_class_past_the_class_cap_is_cut_and_the_metric_says_so() {
    let fx = Fx::lite("hp2-class-cap");
    let cls = define_class(&fx, A, &[("members", Kind::Extent)]);
    fx.publish(&cells_of(&cls, "members", Kind::Extent, "E", 0..12), T2);
    let w = serve_with(fx, &[("DATUMESH_INDEX_CLASS_MAX", "5")], &[(A, "members")]).await;
    // the build has run once one object is found
    let mut built = false;
    for i in 0..12 {
        let r = w.describe_until_found(&cls.id(&named(i, "E"), "Q")).await;
        if r.status == 200 {
            built = true;
            break;
        }
    }
    assert!(built, "no object of the class is in the index");
    assert_eq!(found_count(&w, &cls, "E", 12).await, 5, "the class cap is 5");
    // the others are reached by key encoding
    for i in 0..12 {
        let r = w.get(&format!("/describe/{}", common::world::call_path(&cls.encoding(&named(i, "E"), "Q")))).await;
        assert_eq!(r.status, 200, "E{i} by key: {}", r.text());
    }
    let v = wait_for_metric(&w, 1.0).await;
    assert!(v >= 1.0, "datumesh_object_index_truncated_total after a cut class: {v}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_index_as_a_whole_is_capped_too() {
    let fx = Fx::lite("hp2-total-cap");
    let a = define_class(&fx, A, &[("members", Kind::Extent)]);
    let b = define_class(&fx, B, &[("members", Kind::Extent)]);
    let mut lines = cells_of(&a, "members", Kind::Extent, "E", 0..7);
    lines.extend(cells_of(&b, "members", Kind::Extent, "E", 0..7));
    fx.publish(&lines, T2);
    let w = serve_with(
        fx,
        &[("DATUMESH_INDEX_CLASS_MAX", "5"), ("DATUMESH_INDEX_MAX", "8")],
        &[(A, "members"), (B, "members")],
    )
    .await;
    let mut built = false;
    for i in 0..7 {
        if w.describe_until_found(&a.id(&named(i, "E"), "Q")).await.status == 200 {
            built = true;
            break;
        }
    }
    assert!(built, "the build ran");
    // 5 of the first class (its cap), then the 3 the total has room for in the second
    let (ka, kb) = (found_count(&w, &a, "E", 7).await, found_count(&w, &b, "E", 7).await);
    assert_eq!(ka + kb, 8, "the index holds at most 8 objects: {ka} + {kb}");
    assert!(ka <= 5 && kb <= 5, "{ka} {kb}");
    let v = wait_for_metric(&w, 2.0).await;
    assert!(v >= 2.0, "both classes were cut: {v}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_class_within_the_caps_is_not_counted_as_cut() {
    let (fx, cls) = extent_class("hp2-no-cut");
    let w = serve_with(fx, &[("DATUMESH_INDEX_CLASS_MAX", "50"), ("DATUMESH_INDEX_MAX", "100")], &[(A, "members")]).await;
    assert_eq!(w.describe_until_found(&cls.id(&named(0, "E"), "Q")).await.status, 200);
    assert_eq!(found_count(&w, &cls, "E", 6).await, 6);
    let v = w.metric("datumesh_object_index_truncated_total").await;
    assert!(v.is_none_or(|v| v == 0.0), "nothing was cut: {v:?}");
}

// ---- HP1: a class describe indexes nothing ----------------------------------------------------------------

/// The class `B` is not heard of at the start. After a describe of it, the object of its list is still not in
/// the last build: 404 with the reason (0.4.9 indexed the class inside the request and answered 200).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describing_a_class_does_not_put_its_objects_into_the_index() {
    let fx = Fx::lite("hp1-named");
    let a = define_class(&fx, A, &[("members", Kind::Extent)]);
    let b = define_class(&fx, B, &[("members", Kind::Extent)]);
    let mut lines = cells_of(&a, "members", Kind::Extent, "E", 0..2);
    lines.extend(cells_of(&b, "members", Kind::Extent, "E", 0..2));
    fx.publish(&lines, T2);
    let w = serve_with(fx, &[], &[(A, "members")]).await;
    assert_eq!(w.describe_until_found(&a.id(&named(0, "E"), "Q")).await.status, 200, "the first build ran");
    // the two calls are made in process, back to back
    let (s, body) = w.describe_now(B);
    assert_eq!(s, 200, "{body}");
    assert!(body.contains(&b.hash), "the class answer: {body}");
    let (s, body) = w.describe_now(&b.id(&named(0, "E"), "Q"));
    assert_eq!(s, 404, "{body}");
    assert!(says_not_in_index(&body), "{body}");
    // an object named by its key encoding is answered without the index
    let (s, body) = w.describe_now(&b.encoding(&named(0, "E"), "Q"));
    assert_eq!(s, 200, "{body}");
}

// ---- HP2 and HP4: ids ---------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_short_id_answers_as_the_full_id_and_a_cell_id_is_not_an_object_id() {
    let (fx, cls) = extent_class("hp4-short");
    let w = serve_with(fx, &[], &[(A, "members")]).await;
    assert_eq!(w.describe_until_found(&cls.id(&named(0, "E"), "Q")).await.status, 200);
    for i in 0..6 {
        let id = cls.id(&named(i, "E"), "Q");
        let full = w.get(&format!("/describe/{id}")).await;
        let short = w.get(&format!("/describe/{}", &id[..12])).await;
        assert_eq!((short.status, &short.body), (200, &full.body), "E{i}: {}", short.text());
        let doc = short.json();
        assert_eq!(doc["short_id"], id[..12], "{doc}");
        assert_eq!(doc["object_id"], id, "{doc}");
    }
    // twelve hex digits that begin no object id
    let r = w.get("/describe/000000000000").await;
    assert_eq!(r.status, 404, "{}", r.text());
    // the id of a cell of the extent function names no object
    let cid = cls.cell_id("members", Kind::Extent, &named(0, "E"), "Q");
    for id in [cid.clone(), cid[..12].to_string()] {
        let r = w.get(&format!("/describe/{id}")).await;
        assert_eq!(r.status, 404, "a cell id: {}", r.text());
    }
}
