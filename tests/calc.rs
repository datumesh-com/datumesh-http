//! Build 2 Part 3 addition, `calc` at the HTTP surface (CONTRACT-build2-part3-calc C1, C3, C6; D27):
//! `POST /x/calc`, the mesh document's `endpoints.x_calc`.
//!
//! The surface these tests specify:
//!
//! * the mesh document lists `endpoints.x_calc`, `https://<host>/x/calc`; OpenAPI has `POST /x/calc`;
//! * the request is `{"formula": "<body>", "asof"?: instant}`; any other member, a missing
//!   `formula`, is `bad_request` `E_SCHEMA`; a body that is not strict JSON is `E_PARSE_JSON`;
//!   a bad `asof` is `E_CANON_INSTANT`; a GET is 405;
//! * the answer is HTTP 200 with `value`, `type`, `formula` (as given), `canonical` (the printed
//!   canonical AST of 15.6, no final LF), `asof`, `lineage` and `kind: "calculation"`, and no
//!   `receipt`, no `trust`. The members may sit at the top level or under a `calculation` member
//!   (the contract does not say; `ans` takes either);
//! * `lineage`: an array of the cells read; each entry has `call`, `receipt` (the hash of a signed
//!   receipt the mesh serves at `/receipt/<hash>`), `publisher` and `trust` (names as `explain`);
//! * nothing is written (the store snapshot is unchanged), nothing is bound;
//! * errors carry the formula library's and the engine's code, as `error` or as `fault`;
//!   a cycle is the `error` envelope of 5.5 (`E_EVAL_UNAVAILABLE` with `cycle`);
//! * the formula body is at most 4096 bytes; the evaluation budget is the mesh's, past it the
//!   answer is `E_EVAL_BUDGET` with no `pending` job; the rate limit is
//!   `DATUMESH_RATE_CALC_PER_IP`.

#[path = "common/calculus.rs"]
mod calculus;
mod common;

use calculus::*;
use common::lite::*;
use common::pub031::{function_line, snapshot};
use common::readpath::*;
use serde_json::{json, Value};

impl Served {
    async fn calc(&self, formula: &str) -> Reply {
        self.post_raw("/x/calc", serde_json::to_vec(&json!({ "formula": formula })).unwrap()).await
    }

    async fn calc_asof(&self, formula: &str, asof: &str) -> Reply {
        self.post_raw("/x/calc", serde_json::to_vec(&json!({"formula": formula, "asof": asof})).unwrap()).await
    }
}

async fn mesh(label: &str) -> Served {
    serve_lite(label, &whatif_lines(&calc()), None, |_| {}).await
}

/// The members of the answer, at the top level or under `calculation`.
fn ans(r: &Reply) -> &Value {
    match r.json.get("calculation") {
        Some(c) => c,
        None => &r.json,
    }
}

fn text(r: &Reply) -> String {
    String::from_utf8_lossy(&r.body).to_string()
}

fn value(r: &Reply) -> String {
    assert_eq!(r.status, 200, "{}", text(r));
    ans(r)["value"].as_str().unwrap_or_else(|| panic!("no value: {}", text(r))).to_string()
}

/// The code of an error answer: `error`, or `fault`.
fn code(r: &Reply) -> String {
    r.json["error"].as_str().or(r.json["fault"].as_str()).unwrap_or_else(|| panic!("no code: {}", text(r))).to_string()
}

fn lineage(r: &Reply) -> Vec<&Value> {
    ans(r)["lineage"].as_array().unwrap_or_else(|| panic!("no lineage: {}", text(r))).iter().collect()
}

fn entry_for<'a>(r: &'a Reply, call: &str) -> &'a Value {
    let want = norm(call);
    lineage(r)
        .into_iter()
        .find(|e| e["call"].as_str().map(norm) == Some(want.clone()))
        .unwrap_or_else(|| panic!("no lineage entry for {call}: {}", text(r)))
}

fn reading(c: &Calc, site: &str) -> String {
    format!("{}(site: \"{site}\")", c.reading)
}

// ---- the mesh document and OpenAPI ------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mesh_document_lists_the_x_calc_endpoint() {
    let m = mesh("calc-doc").await;
    let md = m.get("/.well-known/datumesh-mesh.json").await;
    assert_eq!(md["endpoints"]["x_calc"], format!("https://{HOST}/x/calc"), "{md}");
    // the spec's own endpoints are still there
    assert_eq!(md["endpoints"]["whatif"], format!("https://{HOST}/whatif"), "{md}");
    assert_eq!(md["endpoints"]["x_bundles"], format!("https://{HOST}/bundles"), "{md}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn openapi_documents_post_x_calc() {
    let m = mesh("calc-openapi").await;
    let api = m.get("/openapi.json").await;
    let p = &api["paths"]["/x/calc"];
    assert!(p.get("post").is_some(), "OpenAPI lacks POST /x/calc: {}", api["paths"]);
    assert!(p.get("get").is_none());
    let doc = p["post"].to_string();
    assert!(doc.contains("calculation"), "the POST is described: {doc}");
}

// ---- the answer -------------------------------------------------------------------------------

/// A formula over a stored cell: value, type, the formula as given and canonical, the mesh's now
/// as `asof`, `kind: calculation`, and no receipt, no trust.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_formula_over_a_stored_cell_answers_value_type_and_kind() {
    let c = calc();
    let m = mesh("calc-stored").await;
    let f = format!("{} * 100", reading(&c, "N"));
    let r = m.calc(&f).await;
    assert_eq!(value(&r), "1000");
    let a = ans(&r);
    assert_eq!(a["type"], "number", "{}", text(&r));
    assert_eq!(a["kind"], "calculation");
    assert_eq!(a["formula"], f.as_str());
    let canonical = datumesh_formula::canonical_ast(&f, None).unwrap().print();
    assert_eq!(a["canonical"], canonical.as_str(), "{}", text(&r));
    assert_eq!(a["asof"], MESH_NOW);
    // no receipt and no trust, at the top level or in the answer
    for v in [&r.json, a] {
        assert!(v.get("receipt").is_none() && v.get("trust").is_none(), "{}", text(&r));
    }
    // the lineage: the one cell read, with a call, a receipt hash, a publisher and a trust
    let l = lineage(&r);
    assert_eq!(l.len(), 1, "{}", text(&r));
    let e = entry_for(&r, &reading(&c, "N"));
    for k in ["receipt", "publisher", "trust"] {
        assert!(e[k].as_str().is_some_and(|s| !s.is_empty()), "{k}: {e}");
    }
    assert_eq!(e["publisher"], HOST);
    // the receipt is the signed one the mesh serves, for that cell and value
    let rc = m.get(&format!("/receipt/{}", e["receipt"].as_str().unwrap())).await;
    assert_eq!(rc["value"], "10", "{rc}");
    assert_eq!(rc["cell"]["fn"], "reading");
}

/// Slice aggregates over stored cells (the formula library's `sum`, `mean`, `count`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slice_aggregates_work() {
    let c = calc();
    let m = mesh("calc-agg").await;
    let s = format!("{}(site: *)", c.reading);
    for (f, want) in [
        (format!("mean({s})"), "20"),
        (format!("sum({s})"), "40"),
        (format!("count({s})"), "2"),
        (format!("max({s}) - min({s})"), "20"),
    ] {
        let r = m.calc(&f).await;
        assert_eq!(value(&r), want, "{f}: {}", text(&r));
        assert_eq!(ans(&r)["type"], "number");
        assert_eq!(ans(&r)["kind"], "calculation");
        assert!(!lineage(&r).is_empty(), "{f}: the slice's cells are in the lineage: {}", text(&r));
    }
    // the lineage of a slice aggregate names the members read (or the slice): N and S are in it
    let r = m.calc(&format!("sum({s})")).await;
    let all = serde_json::to_string(&lineage(&r)).unwrap();
    assert!(all.contains("\"N\"") && all.contains("\"S\""), "{all}");
}

/// A formula over computed inputs. A computed cell the mesh has served has a signed receipt, and
/// the lineage names it with trust `computed`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_formula_over_computed_inputs() {
    let c = calc();
    let m = mesh("calc-computed").await;
    let f = format!("{}(site: \"N\") + 1", c.double);
    let r = m.calc(&f).await;
    assert_eq!(value(&r), "21", "{}", text(&r));
    // after the mesh has stored double(N) (a GET computes and keeps it), it is a lineage entry
    let v = m.get(&format!("/cell/{}?site=N", c.double)).await;
    assert_eq!(v["receipt"]["value"], "20");
    let r = m.calc(&f).await;
    assert_eq!(value(&r), "21");
    let e = entry_for(&r, &format!("{}(site: \"N\")", c.double));
    assert_eq!(e["trust"], "computed", "{e}");
    assert_eq!(e["publisher"], HOST);
    let rc = m.get(&format!("/receipt/{}", e["receipt"].as_str().unwrap())).await;
    assert_eq!(rc["value"], "20");
    // a mix of stored and computed
    let r = m.calc(&format!("{}(site: \"S\") + {}(site: \"N\")", c.reading, c.double)).await;
    assert_eq!(value(&r), "50", "{}", text(&r));
}

/// The same formula in another spelling answers the same value; `formula` is as given.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_canonical_form_is_the_same_for_equivalent_spellings() {
    let c = calc();
    let m = mesh("calc-canon").await;
    let a = format!("{}*2", reading(&c, "N"));
    let b = format!("  {} *   2 ", reading(&c, "N"));
    let (ra, rb) = (m.calc(&a).await, m.calc(&b).await);
    assert_eq!((value(&ra), value(&rb)), ("20".into(), "20".into()));
    assert_eq!(ans(&ra)["formula"], a.as_str());
    assert_eq!(ans(&rb)["formula"], b.as_str());
    assert_eq!(ans(&ra)["canonical"], ans(&rb)["canonical"]);
}

// ---- nothing written --------------------------------------------------------------------------

/// The store is unchanged by calculations of every kind, a calculation is never served at `/cell`,
/// and no name is bound.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_is_written() {
    let c = calc();
    let m = mesh("calc-nowrite").await;
    m.raw("/health").await;
    let before = snapshot(&m.lite);
    let s = format!("{}(site: *)", c.reading);
    for f in [
        format!("{} * 100", reading(&c, "N")),
        format!("{}(site: \"S\") + 1", c.double), // double(S) is not stored: computed in the scope only
        format!("mean({s})"),
    ] {
        assert_eq!(m.calc(&f).await.status, 200, "{f}");
    }
    assert!(m.calc("1 +").await.status >= 400);
    assert_eq!(snapshot(&m.lite), before, "a calc wrote something");
    let bindings = m.get("/bindings").await.to_string();
    assert!(!bindings.contains("calc_"), "a name was bound: {bindings}");
    // an answer is no receipt: its value is not a version of anything
    let r = m.get_any(&format!("/versions/{}?site=N", c.reading)).await;
    assert!(!String::from_utf8_lossy(&r.body).contains("1000"));
}

// ---- asof -------------------------------------------------------------------------------------

const C1: &str = "2026-11-01T00:00:00.000000Z";
const T: &str = "2026-11-10T00:00:00.000000Z";
const C2: &str = "2026-11-12T00:00:00.000000Z";

/// `reading(N)` is 10 from C1 and 20 from C2: a calc as of T sees 10, the default (the mesh's now)
/// sees 20, and the answer states the asof it used.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn asof_reads_the_data_as_of_that_instant() {
    let c = calc();
    let run1 = data_lines(&c);
    let run2 = vec![site_cell(&c.reading, "N", "20")];
    let m = serve_runs("calc-asof", &[(C1, run1), (C2, run2)], None).await;
    let f = format!("{} * 100", reading(&c, "N"));
    let r = m.calc_asof(&f, T).await;
    assert_eq!(value(&r), "1000", "{}", text(&r));
    assert_eq!(ans(&r)["asof"], T);
    let r = m.calc(&f).await;
    assert_eq!(value(&r), "2000", "{}", text(&r));
    assert_eq!(ans(&r)["asof"], MESH_NOW);
    // an asof of the mesh's now is the same as none
    let r = m.calc_asof(&f, MESH_NOW).await;
    assert_eq!(value(&r), "2000");
    // the lineage's receipt is the one in force at that instant
    let r = m.calc_asof(&f, T).await;
    let e = entry_for(&r, &reading(&c, "N"));
    let rc = m.get(&format!("/receipt/{}", e["receipt"].as_str().unwrap())).await;
    assert_eq!(rc["value"], "10", "{rc}");
}

// ---- errors -----------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn formula_errors_carry_their_codes() {
    let c = calc();
    let m = mesh("calc-errors").await;
    let cases: Vec<(&str, String, &str)> = vec![
        ("a syntax error", "1 +".into(), "E_FORMULA_SYNTAX"),
        ("a type error", "1 + \"a\"".into(), "E_FORMULA_TYPE"),
        ("a division by zero", format!("{} / 0", reading(&c, "N")), "E_FORMULA_DIV_ZERO"),
        ("an absent cell", format!("{} + 1", reading(&c, "ZZ")), "E_FORMULA_ABSENT"),
        ("an empty slice", format!("mean({}(site: \"ZZ\", day: *))", c.log), "E_FORMULA_EMPTY"),
    ];
    for (what, f, want) in cases {
        let r = m.calc(&f).await;
        assert!((400..600).contains(&r.status), "{what}: {} {}", r.status, text(&r));
        if want.is_empty() {
            assert!(code(&r).starts_with("E_"), "{what}: {}", text(&r));
        } else {
            assert_eq!(code(&r), want, "{what}: {}", text(&r));
        }
        for v in [&r.json, ans(&r)] {
            assert!(v.get("receipt").is_none() && v.get("kind").is_none_or(|k| k != "calculation"), "{what}");
        }
    }
    // free names are not allowed: a calc has no arguments of its own
    let r = m.calc("x + 1").await;
    assert!((400..600).contains(&r.status), "{}", text(&r));
    assert!(code(&r).starts_with("E_FORMULA_"), "{}", text(&r));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bad_requests_have_their_codes() {
    let m = mesh("calc-bad").await;
    let cases: Vec<(&str, Value, &str)> = vec![
        ("no formula", json!({}), "E_SCHEMA"),
        ("an unknown member", json!({"formula": "1 + 1", "extra": 1}), "E_SCHEMA"),
        ("a formula that is not a string", json!({"formula": 7}), "E_SCHEMA"),
        ("a bad asof", json!({"formula": "1 + 1", "asof": "yesterday"}), "E_CANON_INSTANT"),
    ];
    for (what, b, want) in cases {
        let r = m.post_raw("/x/calc", serde_json::to_vec(&b).unwrap()).await;
        assert_eq!(
            (r.status, r.json["outcome"].as_str(), r.json["error"].as_str()),
            (400, Some("bad_request"), Some(want)),
            "{what}: {}",
            text(&r)
        );
    }
    let r = m.post_raw("/x/calc", b"{not json".to_vec()).await;
    assert_eq!(
        (r.status, r.json["outcome"].as_str(), r.json["error"].as_str()),
        (400, Some("bad_request"), Some("E_PARSE_JSON"))
    );
    let r = m.post_raw("/x/calc", br#"{"formula": "1", "formula": "2"}"#.to_vec()).await;
    assert_eq!((r.status, r.json["error"].as_str()), (400, Some("E_PARSE_DUPLICATE_KEY")));
    assert_eq!(m.get_any("/x/calc").await.status, 405);
}

/// A cycle among the functions the formula reads is the cycle envelope of 5.5.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cycle_names_its_path() {
    let p = "mesh-test.datumesh.com/scratch_b3/cyc";
    let (a, b, reading_fn) = (format!("{p}/a"), format!("{p}/b"), format!("{p}/reading"));
    let sig =
        r#"{"args":{"site":{"type":"string","valid":false}},"order":["site"],"returns":{"type":"number","unit":"1"}}"#;
    let run1 = vec![
        function_line(&reading_fn, &reading_sig(), None),
        site_cell(&reading_fn, "N", "10"),
        formula_line(&a, &format!("{reading_fn}(site: site)"), sig, None),
        formula_line(&b, &format!("{a}(site: site) + 1"), sig, None),
    ];
    // a is rebound to read b: a -> b -> a
    let run2 = vec![formula_line(&a, &format!("{b}(site: site)"), sig, None)];
    let m = serve_runs("calc-cycle", &[(C1, run1), (C2, run2)], None).await;
    let r = m.calc(&format!("{b}(site: \"N\") * 2")).await;
    assert!((500..600).contains(&r.status), "{} {}", r.status, text(&r));
    assert_eq!((r.json["outcome"].as_str(), r.json["error"].as_str()), (Some("error"), Some("E_EVAL_UNAVAILABLE")));
    let path = r.json["cycle"]["path"].as_array().unwrap_or_else(|| panic!("no cycle: {}", text(&r)));
    assert!(path.len() >= 3 && path.first() == path.last(), "{path:?}");
    let fixes: Vec<&str> = r.json["cycle"]["fixes"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert_eq!(fixes, ["asof", "solver"]);
}

// ---- limits -----------------------------------------------------------------------------------

/// The body is at most 4096 bytes: exactly 4096 answers, one more is refused and nothing is
/// evaluated.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_formula_body_is_at_most_4_kib() {
    let m = mesh("calc-size").await;
    let pad = |n: usize| format!("1 + 2{}", " ".repeat(n - 5));
    let ok = pad(4096);
    assert_eq!(ok.len(), 4096);
    let r = m.calc(&ok).await;
    assert_eq!(value(&r), "3", "{}", text(&r));
    let big = pad(4097);
    assert_eq!(big.len(), 4097);
    let r = m.calc(&big).await;
    assert!(r.status == 400 || r.status == 413, "{} {}", r.status, text(&r));
    assert!(r.json.get("value").is_none(), "{}", text(&r));
    assert!(r.json.get("error").is_some() || r.json.get("outcome").is_some(), "{}", text(&r));
    // far past the limit is refused the same way
    let r = m.calc(&pad(200_000)).await;
    assert!(r.status == 400 || r.status == 413, "{}", r.status);
}

fn many_sites(n: usize) -> Vec<String> {
    let c = calc();
    let mut lines = whatif_lines(&c);
    lines.extend((0..n).map(|i| site_cell(&c.reading, &format!("P{i:05}"), "1")));
    lines
}

/// Past the budget the answer is `E_EVAL_BUDGET`, at once, with no pending job; with the default
/// budget the same formula answers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn past_the_budget_is_e_eval_budget_with_no_job() {
    let c = calc();
    let f = format!("sum({}(site: *)) + 1", c.reading);
    // a control with the default budget: 40 + 4000 + 1
    let ok = serve_lite("calc-budget-ok", &many_sites(4000), None, |_| {}).await;
    let r = ok.calc(&f).await;
    assert_eq!(value(&r), "4041", "{}", text(&r));

    let m = serve_lite("calc-budget", &many_sites(4000), None, |cfg| {
        cfg.budget = std::time::Duration::from_millis(1);
    })
    .await;
    let t0 = std::time::Instant::now();
    let r = m.calc(&f).await;
    assert!(t0.elapsed() < std::time::Duration::from_secs(5), "{:?}", t0.elapsed());
    assert_ne!(r.status, 202, "no pending job: {}", text(&r));
    assert!(r.status >= 400, "{} {}", r.status, text(&r));
    assert_eq!(code(&r), "E_EVAL_BUDGET", "{}", text(&r));
    assert!(r.json.get("job").is_none() && r.json.get("poll").is_none(), "{}", text(&r));
    assert!(r.json.get("value").is_none());
}

/// `DATUMESH_RATE_CALC_PER_IP` limits `POST /x/calc`: past it, 429 `rate_limited` with
/// `Retry-After`; other endpoints are not affected.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_calc_rate_limit_is_its_own() {
    let cfg = datumesh_http::Config::from_vars(|k| match k {
        "DATUMESH_HOST" => Some(HOST.to_string()),
        "DATUMESH_RATE_CALC_PER_IP" => Some("3".to_string()),
        _ => None,
    })
    .unwrap();
    let m = serve_lite("calc-rate", &whatif_lines(&calc()), Some(cfg), |_| {}).await;
    for i in 0..3 {
        let r = m.calc("1 + 1").await;
        assert_eq!(r.status, 200, "{i}: {}", text(&r));
    }
    let r = m
        .http
        .post(format!("{}/x/calc", m.base))
        .header("content-type", "application/json")
        .body(r#"{"formula": "1 + 1"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 429);
    assert!(r.headers().get("retry-after").is_some());
    let j: Value = serde_json::from_slice(&r.bytes().await.unwrap()).unwrap();
    assert!(j["outcome"] == "rate_limited" || j["error"] == "rate_limited", "{j}");
    // other endpoints keep their own limits
    assert_eq!(m.raw("/health").await.0, 200);
    let c = calc();
    assert_eq!(m.get_any(&format!("/cell/{}?site=N", c.reading)).await.status, 200);
}

/// The default is 30 a minute: 30 calcs in a row are all answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_default_calc_rate_limit_is_30_a_minute() {
    let m = mesh("calc-rate-default").await;
    for i in 0..30 {
        let r = m.calc("1 + 1").await;
        assert_eq!(r.status, 200, "{i}: {}", text(&r));
    }
    let r = m.calc("1 + 1").await;
    assert_eq!(r.status, 429, "the 31st in the minute: {}", text(&r));
}
