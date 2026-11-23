//! http 0.5.1, CONTRACT-0.5.1 H2: the catalogue thread and the catalogue job read in pages. Written first; FAIL until then.
//!
//! * "They never ask the store for more than one page": 6,144 subjects (24 catalogue entries of 256 subjects) are written by
//!   the job, then the job is run again over the unchanged catalogue and the thread builds the view, with a PostgreSQL probe
//!   (`common/h051.rs`) that records the MOST rows of `cells` ONE statement visits. A page is 1,000 subjects; reading
//!   `label`, `sources` and `source(1)` of a page's subjects by key is at most 3 cells a subject, so no statement may
//!   visit more than 3,500 rows of cells. The 0.5.0 reads take every stored cell of a method in one statement
//!   (`STORED_CELLS_READ`, 1 << 20): at least 6,144 rows, and 12,000 for `source` with its cells' versions.
//!   The job's rerun writes nothing and the view lists the same 6,000 subjects.
//! * `DATUMESH_CATALOG_MAX` (default 100,000) caps the subjects of the thread's view: with 100 the view lists 100 of the 512
//!   subjects a job wrote, and the cut is counted in a metric. With the default nothing is cut.
//!
//! Assumptions (the contract is silent): the cap's metric is a counter whose name has `catalog` and one of `trunc`, `cut`
//! or `capped`, and a value above 0 once the cap cut a build; the job's own cap (the same knob, read by the job's process)
//! is not tested here: `catalog_build_with` takes no configuration, so the test could only set the environment of the whole
//! test process. The job's output being unchanged is what the existing catalogue tests pin (they run on this branch).

#[path = "common/catjob.rs"]
mod catjob;
#[path = "common/catworld.rs"]
mod catworld;
mod common;
#[path = "common/crworld.rs"]
mod crworld;
#[path = "common/h0410.rs"]
mod h0410;
#[path = "common/h051.rs"]
mod h051;

use catjob::*;
use catworld::*;
use common::lite::*;
use crworld::*;
use serde_json::Value;

/// `n` entries of 256 subjects each (8 things by 8 measures by 4 regions: the most an entry gives, and the lists of a facet
/// have at most 8 names), each in a domain of its own.
fn entries(n: usize) -> Vec<String> {
    let things: Vec<String> = (0..8).map(|i| format!("t{i}")).collect();
    let measures: Vec<String> = (0..8).map(|i| format!("m{i}")).collect();
    let regions: Vec<String> = (0..4).map(|i| format!("r{i}")).collect();
    let (t, m, r): (Vec<&str>, Vec<&str>, Vec<&str>) = (
        things.iter().map(String::as_str).collect(),
        measures.iter().map(String::as_str).collect(),
        regions.iter().map(String::as_str).collect(),
    );
    let mut v = Vec::new();
    for e in 0..n {
        let name = host_name(&format!("w/e{e:02}"));
        v.push(fn_line(&name));
        v.push(cat_line(&name, fac(&[&format!("d{e:02}")], &t, &m, &r, &["static"])));
    }
    v
}

fn subjects_in(root: &Value) -> u64 {
    root["domains"].as_array().unwrap().iter().map(|d| d["subjects"].as_u64().unwrap()).sum()
}

const PAGE_BOUND: u64 = 3_500;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h2_the_job_and_the_thread_never_read_more_than_a_page_at_a_time() {
    let fx = h0410::Fx::pg("h051-pages");
    fx.publish(&entries(24), NOW);
    let first = run_job(&fx, T3);
    assert_eq!(first.subjects, 6144, "{first:?}");
    assert!(first.cells > 6144, "{first:?}");
    h051::arm_statement_max(&fx, &["cells"]);
    // the job again over the unchanged catalogue: it reads what is stored, in pages, and writes nothing
    let again = run_job(&fx, T4);
    assert_eq!((again.cells, again.subjects), (0, 6144), "{again:?}");
    let job = h051::statement_max(&fx);
    assert!(job > 100, "the probe counted nothing (largest statement: {job} rows)");
    assert!(job <= PAGE_BOUND, "the job asked for a statement that visited {job} rows of cells; a page is 1,000 subjects, at most {PAGE_BOUND}");
    // the thread's first build
    h051::statement_reset(&fx);
    let w = fx.serve(h051::cfg(&[], &[])).await;
    let root = loop_root(&w).await;
    assert_eq!(subjects_in(&root), 6144, "{root}");
    let thread = h051::statement_max(&w.fx);
    assert!(thread > 100, "the probe counted nothing for the thread ({thread})");
    assert!(
        thread <= PAGE_BOUND,
        "the catalogue thread asked for a statement that visited {thread} rows of cells; at most {PAGE_BOUND}"
    );
}

fn run_job(fx: &h0410::Fx, clock: &'static str) -> datumesh_http::catalog_build::CatalogBuilt {
    let (store, key) = (fx.store.clone(), fx.key.clone());
    common::on_thread(move || {
        datumesh_http::catalog_build::catalog_build_with(&*store, &key, HOST, LICENCE, &common::pub031::opts(clock))
            .unwrap_or_else(|e| panic!("catalog-build: {e}"))
    })
}

async fn loop_root(w: &h0410::W) -> Value {
    let r = w.describe_until_found(&format!("{HOST}/catalog")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    r.json()
}

// ---- DATUMESH_CATALOG_MAX ----------------------------------------------------------------------------------------------

/// The cut counters of the metrics: the lines whose name has `catalog` and `trunc`, `cut` or `capped`.
fn cut_counted(metrics: &str) -> f64 {
    metrics
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter(|l| {
            let name = l.split(['{', ' ']).next().unwrap_or("");
            name.contains("catalog") && ["trunc", "cut", "capped"].iter().any(|k| name.contains(k))
        })
        .filter_map(|l| l.rsplit(' ').next().and_then(|v| v.parse::<f64>().ok()))
        .sum()
}

fn world(label: &str) -> Lite {
    let lite = Lite::new(label);
    publish(&lite, "e.jsonl", &entries(2), NOW).unwrap_or_else(|e| panic!("{e}"));
    let b = build(&lite, T3);
    assert_eq!(b.subjects, 512, "{b:?}");
    lite
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h2_the_cap_cuts_the_view_and_is_counted() {
    let m = serve(world("h2-cap"), &[("DATUMESH_CATALOG_MAX", "100")]).await;
    let r = until_built(&m, &format!("/describe/{HOST}/catalog")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(subjects_in(&r.json()), 100, "the view lists the cap's worth of subjects: {}", r.text());
    let metrics = page(&m, "/metrics").await.text();
    assert!(
        cut_counted(&metrics) >= 1.0,
        "the cut is counted (a counter named catalog ... trunc|cut|capped):\n{metrics}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h2_under_the_cap_nothing_is_cut() {
    for vars in [vec![], vec![("DATUMESH_CATALOG_MAX", "512")]] {
        let m = serve(world("h2-nocap"), &vars).await;
        let r = until_built(&m, &format!("/describe/{HOST}/catalog")).await;
        assert_eq!(r.status, 200, "{}", r.text());
        assert_eq!(subjects_in(&r.json()), 512, "{vars:?}: {}", r.text());
        let metrics = page(&m, "/metrics").await.text();
        assert_eq!(cut_counted(&metrics), 0.0, "{vars:?}: nothing was cut:\n{metrics}");
    }
}
