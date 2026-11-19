//! Build 2 Part 3, E1/E2 at the HTTP surface: `POST /whatif` (spec-0.6.0, 5.9).
//!
//! The surface these tests specify:
//!
//! * the mesh document lists `endpoints.whatif`, `https://<host>/whatif`, no placeholders;
//! * `POST /whatif` takes the JSON body of 5.9 (`cell`, optional `asof`, `overrides`, optional
//!   `solve` with `vary`, `target`, `equals`, `lo`, `hi`); any other member, or a missing `cell` or
//!   `overrides`, is `bad_request` `E_SCHEMA`; a body that is not strict JSON is `E_PARSE_JSON`;
//! * the answer to a good request is HTTP 200 and the envelope `{"outcome": "scenario", "scenario":
//!   {asof, cell, overrides, type, value[, solved]}}` of 5.9, with no `receipt`, no `trust`;
//!   core 0.3.9's `check_answer` accepts it for the request description with the canonical
//!   overrides; an absent `asof` is the mesh's now;
//! * `bad_request` (400) codes: `E_CANON_TYPE` (a value of another type than the cell's return),
//!   `E_SLICE_MEMBER` (a member not in the slice, or both added and removed),
//!   `E_CANON_ARG_DUPLICATE` (two overrides of one cell or one slice), `E_CANON_LIMIT` (more than 64
//!   overrides), `E_SCHEMA` (empty overrides without `solve`), `E_CANON_INSTANT`, `E_PARSE_URI`;
//! * `not_covered` (404): an unbound name (`E_NAME_UNBOUND`), a fault in the scope (`fault`
//!   `E_FORMULA_DIV_ZERO`, say) and, for `solve` with no root in the bounds,
//!   `E_WHATIF_NO_SOLUTION`;
//! * nothing is written (the store file is byte-identical), and a scenario is never served at
//!   `/cell` afterwards.

#[path = "common/calculus.rs"]
mod calculus;
mod common;

use calculus::*;
use common::lite::*;
use common::pub031::snapshot;
use common::readpath::*;
use serde_json::{json, Value};

async fn mesh(label: &str) -> Served {
    serve_lite(label, &whatif_lines(&calc()), None, |_| {}).await
}

fn dbl(c: &Calc, m: &Served) -> Value {
    cell_json(m, &c.double, &[("site", "string", "N")])
}

fn reading(c: &Calc, m: &Served, site: &str) -> Value {
    cell_json(m, &c.reading, &[("site", "string", site)])
}

fn call(c: &Calc, f: &str, args: &str) -> String {
    format!(
        "{}({args})",
        match f {
            "double" => &c.double,
            "inverse" => &c.inverse,
            "square" => &c.square,
            "all_sum" => &c.all_sum,
            "reading" => &c.reading,
            _ => panic!(),
        }
    )
}

fn body(cell: &str, overrides: Value) -> Value {
    json!({"cell": cell, "overrides": overrides})
}

fn n(v: &str) -> Value {
    json!({"type": "number", "value": v})
}

// ---- the mesh document ------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mesh_document_lists_the_whatif_endpoint() {
    let m = mesh("wi-doc").await;
    let md = m.get("/.well-known/datumesh-mesh.json").await;
    assert_eq!(md["endpoints"]["whatif"], format!("https://{HOST}/whatif"), "{md}");
    // core 0.3.9 reads it
    let kd = m.raw("/.well-known/datumesh.json").await;
    let mdb = m.raw("/.well-known/datumesh-mesh.json").await;
    let resolved = datumesh_core::resolve_mesh(
        HOST,
        datumesh_core::Retrieval::Response { status: 200, body: Some(&kd.1) },
        datumesh_core::Retrieval::Response { status: 200, body: Some(&mdb.1) },
    )
    .unwrap();
    assert_eq!(
        resolved.mesh.endpoints.get("whatif").map(String::as_str),
        Some(format!("https://{HOST}/whatif").as_str())
    );
}

// ---- value overrides --------------------------------------------------------------------------

/// A value override changes a dependent; the answer is a scenario that core accepts, with the
/// overrides canonical and the mesh's now as `asof`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_value_override_changes_a_dependent() {
    let c = calc();
    let m = mesh("wi-value").await;
    let ov = canonical_overrides(&[(reading(&c, &m, "N"), "100")]);
    let r = m.whatif(&body(&call(&c, "double", "site: \"N\""), ov.clone())).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(scenario_value(&r), "200");
    let s = &r.json["scenario"];
    assert_eq!(s["type"], "number");
    assert_eq!(s["asof"], MESH_NOW);
    assert!(r.json.get("receipt").is_none() && r.json.get("trust").is_none(), "{}", r.json);
    let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["asof", "cell", "overrides", "type", "value"]);
    assert_eq!(s["cell"], dbl(&c, &m));
    check_scenario(&r, &dbl(&c, &m), None, &ov, None);

    // with an asof sent, the scenario states it, and core checks it
    let mut b = body(&call(&c, "double", "site: \"N\""), ov.clone());
    b["asof"] = json!(MESH_NOW);
    let r = m.whatif(&b).await;
    assert_eq!(scenario_value(&r), "200");
    check_scenario(&r, &dbl(&c, &m), Some(MESH_NOW), &ov, None);
}

/// The cell may be written as a URI's `<rest>`, not only as a call.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_cell_may_be_a_uri_rest() {
    let c = calc();
    let m = mesh("wi-rest").await;
    let ov = canonical_overrides(&[(reading(&c, &m, "N"), "7")]);
    let r = m.whatif(&body(&format!("{}?site=N", c.double), ov)).await;
    assert_eq!(scenario_value(&r), "14", "{}", String::from_utf8_lossy(&r.body));
}

/// A non-dependent cell answers as it does, an override of the cell itself answers its value.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_non_dependent_is_unchanged_and_an_overridden_cell_answers_its_value() {
    let c = calc();
    let m = mesh("wi-nondep").await;
    let ov = canonical_overrides(&[(reading(&c, &m, "S"), "1")]);
    let r = m.whatif(&body(&call(&c, "double", "site: \"N\""), ov)).await;
    assert_eq!(scenario_value(&r), "20");
    let ov = canonical_overrides(&[(reading(&c, &m, "N"), "7")]);
    let r = m.whatif(&body(&call(&c, "reading", "site: \"N\""), ov)).await;
    assert_eq!(scenario_value(&r), "7");
    // two overrides, canonical order by id, answered in that order
    let ov = canonical_overrides(&[(reading(&c, &m, "N"), "1"), (reading(&c, &m, "S"), "2")]);
    let r = m.whatif(&body(&call(&c, "all_sum", "scale: 1"), ov.clone())).await;
    assert_eq!(scenario_value(&r), "3");
    let cell = cell_json(&m, &c.all_sum, &[("scale", "number", "1")]);
    check_scenario(&r, &cell, None, &ov, None);
}

// ---- slice overrides --------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slice_overrides_add_and_remove_members() {
    let c = calc();
    let m = mesh("wi-slice").await;
    let slice = slice_json(&m, &c.reading, &[], &["site"]);
    let (n_cell, s_cell, w_cell) = (reading(&c, &m, "N"), reading(&c, &m, "S"), reading(&c, &m, "W"));
    let cell = cell_json(&m, &c.all_sum, &[("scale", "number", "1")]);
    let slice_text = format!("{}(site: *)", c.reading);
    let mk = |add: &[(Value, &str)], remove: &[Value]| {
        let ov = Value::Array(vec![canonical_slice_override(&slice, add, remove)]);
        let b = body(
            &call(&c, "all_sum", "scale: 1"),
            json!([{
                "slice": slice_text,
                "add": add.iter().map(|(cell, v)| json!({"cell": format!("{}(site: \"{}\")", c.reading, cell["args"]["site"]["value"].as_str().unwrap()), "value": n(v)})).collect::<Vec<_>>(),
                "remove": remove.iter().map(|cell| json!(format!("{}(site: \"{}\")", c.reading, cell["args"]["site"]["value"].as_str().unwrap()))).collect::<Vec<_>>(),
            }]),
        );
        (b, ov)
    };
    // baseline: 10 + 30
    let r = m.whatif(&body(&call(&c, "all_sum", "scale: 1"), json!([]))).await;
    assert_eq!(r.json["outcome"], "bad_request", "empty overrides need solve: {}", r.json);
    // remove N
    let (b, ov) = mk(&[], std::slice::from_ref(&n_cell));
    let r = m.whatif(&b).await;
    assert_eq!(scenario_value(&r), "30", "{}", String::from_utf8_lossy(&r.body));
    check_scenario(&r, &cell, None, &ov, None);
    // add W = 5
    let (b, ov) = mk(&[(w_cell.clone(), "5")], &[]);
    let r = m.whatif(&b).await;
    assert_eq!(scenario_value(&r), "45");
    check_scenario(&r, &cell, None, &ov, None);
    // both: remove N, add W = 5, remove S
    let (b, ov) = mk(&[(w_cell.clone(), "5")], &[n_cell.clone(), s_cell.clone()]);
    let r = m.whatif(&b).await;
    assert_eq!(scenario_value(&r), "5");
    check_scenario(&r, &cell, None, &ov, None);
    // an added cell, read alone, answers its override value
    let (b, _) = mk(&[(w_cell.clone(), "5")], &[]);
    let mut alone = b.clone();
    alone["cell"] = json!(call(&c, "reading", "site: \"W\""));
    let r = m.whatif(&alone).await;
    assert_eq!(scenario_value(&r), "5", "{}", String::from_utf8_lossy(&r.body));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slice_member_errors() {
    let c = calc();
    let m = mesh("wi-slice-bad").await;
    let slice_text = format!("{}(site: *)", c.reading);
    let q = |site: &str| format!("{}(site: \"{site}\")", c.reading);
    let cell = call(&c, "all_sum", "scale: 1");
    let cases: Vec<(&str, Value)> = vec![
        ("remove a cell not in the slice", json!([{"slice": slice_text, "add": [], "remove": [q("ZZ")]}])),
        (
            "add and remove the same cell",
            json!([{"slice": slice_text, "add": [{"cell": q("W"), "value": n("5")}], "remove": [q("W")]}]),
        ),
        (
            "a member of another function",
            json!([{"slice": slice_text, "add": [{"cell": call(&c, "double", "site: \"W\""), "value": n("5")}], "remove": []}]),
        ),
    ];
    for (what, ov) in cases {
        let r = m.whatif(&body(&cell, ov)).await;
        assert_eq!(
            (r.status, r.json["outcome"].as_str(), r.json["error"].as_str()),
            (400, Some("bad_request"), Some("E_SLICE_MEMBER")),
            "{what}: {}",
            r.json
        );
    }
}

// ---- solve (E2) -------------------------------------------------------------------------------

fn solve_body(c: &Calc, target: &str, equals: &str, lo: &str, hi: &str) -> Value {
    json!({"cell": target, "overrides": [], "solve": {
        "vary": call(c, "reading", "site: \"N\""), "target": target, "equals": equals, "lo": lo, "hi": hi}})
}

fn close(a: &str, b: f64, tol: f64) -> bool {
    (a.parse::<f64>().unwrap() - b).abs() <= tol
}

/// Solve on a linear function (`double(N) = 50` gives `reading(N) = 25`), on a non-linear one
/// (`square(N) = 2`), the same answer every time, and a target that no value reaches.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn solve_finds_a_root() {
    let c = calc();
    let m = mesh("wi-solve").await;
    let vary = reading(&c, &m, "N");
    let vary_id = core_id(&vary);
    let target = call(&c, "double", "site: \"N\"");
    let r = m.whatif(&solve_body(&c, &target, "50", "0", "100")).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let s = &r.json["scenario"];
    assert_eq!(s["solved"]["vary"], vary, "{}", r.json);
    assert!(close(s["solved"]["value"]["value"].as_str().unwrap(), 25.0, 1e-9), "{}", r.json);
    assert_eq!(s["solved"]["value"]["type"], "number");
    assert!(close(s["value"].as_str().unwrap(), 50.0, 1e-9), "{}", r.json);
    check_scenario(&r, &dbl(&c, &m), None, &json!([]), Some(&vary_id));
    // deterministic: the same bytes again
    let again = m.whatif(&solve_body(&c, &target, "50", "0", "100")).await;
    assert_eq!(r.body, again.body);

    // non-linear: the square root of 2
    let sq = call(&c, "square", "site: \"N\"");
    let r = m.whatif(&solve_body(&c, &sq, "2", "0", "10")).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let x = r.json["scenario"]["solved"]["value"]["value"].as_str().unwrap().to_string();
    assert!(close(&x, 2f64.sqrt(), 1e-9), "{x}");
    let cell = cell_json(&m, &c.square, &[("site", "string", "N")]);
    check_scenario(&r, &cell, None, &json!([]), Some(&vary_id));

    // solve with an override alongside: the other site is not the one varied
    let ov = canonical_overrides(&[(reading(&c, &m, "S"), "1")]);
    let mut b = solve_body(&c, &target, "50", "0", "100");
    b["overrides"] = ov.clone();
    let r = m.whatif(&b).await;
    assert!(close(r.json["scenario"]["solved"]["value"]["value"].as_str().unwrap(), 25.0, 1e-9), "{}", r.json);
    check_scenario(&r, &dbl(&c, &m), None, &ov, Some(&vary_id));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn solve_without_a_root_is_not_covered_no_solution() {
    let c = calc();
    let m = mesh("wi-nosol").await;
    let target = call(&c, "double", "site: \"N\"");
    // double(N) lies in [0, 200] for reading(N) in [0, 100]: 1000 is out of reach
    let r = m.whatif(&solve_body(&c, &target, "1000", "0", "100")).await;
    assert_eq!(
        (r.status, r.json["outcome"].as_str(), r.json["fault"].as_str()),
        (404, Some("not_covered"), Some("E_WHATIF_NO_SOLUTION")),
        "{}",
        r.json
    );
    let vary_id = core_id(&reading(&c, &m, "N"));
    let a = check_scenario_not_covered(&r, &dbl(&c, &m), &vary_id);
    assert_eq!(a.fault.map(|f| f.as_str().to_string()), Some("E_WHATIF_NO_SOLUTION".into()));
}

fn check_scenario_not_covered(r: &Reply, cell: &Value, vary_id: &str) -> datumesh_core::Answer {
    check_scenario(r, cell, None, &json!([]), Some(vary_id))
}

// ---- bad requests -----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bad_requests_have_their_codes() {
    let c = calc();
    let m = mesh("wi-bad").await;
    let target = call(&c, "double", "site: \"N\"");
    let q = |site: &str| format!("{}(site: \"{site}\")", c.reading);
    let ok_ov = json!([{"cell": q("N"), "value": n("1")}]);
    let many: Vec<Value> = (0..65).map(|i| json!({"cell": q(&format!("S{i:02}")), "value": n("1")})).collect();
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "a string for a number cell",
            body(&target, json!([{"cell": q("N"), "value": {"type": "string", "value": "x"}}])),
            "E_CANON_TYPE",
        ),
        (
            "a malformed number",
            body(&target, json!([{"cell": q("N"), "value": {"type": "number", "value": "01"}}])),
            "E_CANON_NUMBER",
        ),
        (
            "two overrides of one cell",
            body(&target, json!([{"cell": q("N"), "value": n("1")}, {"cell": q("N"), "value": n("2")}])),
            "E_CANON_ARG_DUPLICATE",
        ),
        ("more than 64 overrides", body(&target, Value::Array(many)), "E_CANON_LIMIT"),
        ("empty overrides without solve", body(&target, json!([])), "E_SCHEMA"),
        ("no overrides member", json!({"cell": target}), "E_SCHEMA"),
        ("no cell", json!({"overrides": ok_ov}), "E_SCHEMA"),
        ("an unknown member", json!({"cell": target, "overrides": ok_ov, "extra": 1}), "E_SCHEMA"),
        ("a bad asof", json!({"cell": target, "overrides": ok_ov, "asof": "yesterday"}), "E_CANON_INSTANT"),
        ("a cell that is not a call", body("not a call", ok_ov.clone()), "E_PARSE_URI"),
        (
            "an override cell that is not a call",
            body(&target, json!([{"cell": "(((", "value": n("1")}])),
            "E_PARSE_URI",
        ),
        (
            "an unknown argument in the cell",
            body(&format!("{}(site: \"N\", nope: 1)", c.double), ok_ov.clone()),
            "E_ARG_UNKNOWN",
        ),
        (
            "a missing argument in an override cell",
            body(&target, json!([{"cell": format!("{}()", c.reading), "value": n("1")}])),
            "E_ARG_MISSING",
        ),
    ];
    for (what, b, code) in cases {
        let r = m.whatif(&b).await;
        assert_eq!(
            (r.status, r.json["outcome"].as_str(), r.json["error"].as_str()),
            (400, Some("bad_request"), Some(code)),
            "{what}: {}",
            String::from_utf8_lossy(&r.body)
        );
    }
    // exactly 64 is fine
    let sixty_four: Vec<(Value, &str)> =
        (0..64).map(|i| (cell_json(&m, &c.reading, &[("site", "string", &format!("S{i:02}"))]), "1")).collect();
    let ov = canonical_overrides(&sixty_four);
    let r = m.whatif(&body(&target, ov.clone())).await;
    assert_eq!(scenario_value(&r), "20", "{}", String::from_utf8_lossy(&r.body));
    check_scenario(&r, &dbl(&c, &m), None, &ov, None);
    // not JSON; not a POST
    let r = m.post_raw("/whatif", b"{not json".to_vec()).await;
    assert_eq!(
        (r.status, r.json["outcome"].as_str(), r.json["error"].as_str()),
        (400, Some("bad_request"), Some("E_PARSE_JSON"))
    );
    let r = m.post_raw("/whatif", br#"{"cell": "x", "cell": "y", "overrides": []}"#.to_vec()).await;
    assert_eq!((r.status, r.json["error"].as_str()), (400, Some("E_PARSE_DUPLICATE_KEY")));
    assert_eq!(m.get_any("/whatif").await.status, 405);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unbound_name_and_a_fault_in_scope_are_not_covered() {
    let c = calc();
    let m = mesh("wi-nc").await;
    let ov = canonical_overrides(&[(reading(&c, &m, "N"), "1")]);
    let r = m.whatif(&body(&format!("{PFX}/ghost(site: \"N\")"), ov.clone())).await;
    assert_eq!((r.status, r.json["outcome"].as_str()), (404, Some("not_covered")), "{}", r.json);
    assert!(r.json.get("fault").is_none(), "{}", r.json);
    // 1 / reading with reading overridden to 0: a fault of the scenario, as for a get
    let zero = canonical_overrides(&[(reading(&c, &m, "N"), "0")]);
    let r = m.whatif(&body(&call(&c, "inverse", "site: \"N\""), zero.clone())).await;
    assert_eq!(
        (r.status, r.json["outcome"].as_str(), r.json["fault"].as_str()),
        (404, Some("not_covered"), Some("E_FORMULA_DIV_ZERO")),
        "{}",
        r.json
    );
    // the same cell without the override is an ordinary value
    let r =
        m.whatif(&body(&call(&c, "inverse", "site: \"N\""), canonical_overrides(&[(reading(&c, &m, "N"), "4")]))).await;
    assert_eq!(scenario_value(&r), "0.25");
}

// ---- no writes, never a fact ------------------------------------------------------------------

/// The store is byte-identical after scenarios (also a solve), and the cells are what they were:
/// `/cell` answers the real values, signed, never a scenario's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_is_written_and_a_scenario_is_never_a_cell() {
    let c = calc();
    let m = mesh("wi-nowrite").await;
    m.raw("/health").await;
    let before = snapshot(&m.lite);
    let ov = canonical_overrides(&[(reading(&c, &m, "N"), "100")]);
    for b in [
        body(&call(&c, "double", "site: \"N\""), ov.clone()),
        body(&call(&c, "double", "site: \"S\""), ov.clone()),
        solve_body(&c, &call(&c, "double", "site: \"N\""), "50", "0", "100"),
    ] {
        let r = m.whatif(&b).await;
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    }
    assert_eq!(snapshot(&m.lite), before, "a whatif wrote something");
    // /cell: the real value, a receipt signed by the mesh, as of the real data
    let (path_n, path_s) = (format!("/cell/{}?site=N", c.double), format!("/cell/{}?site=S", c.double));
    let v = m.get(&path_n).await;
    assert_eq!((v["outcome"].as_str(), v["receipt"]["value"].as_str()), (Some("ok"), Some("20")), "{v}");
    assert!(v.get("scenario").is_none());
    let v = m.get(&path_s).await;
    assert_eq!(v["receipt"]["value"], "60");
    let v = m.get(&format!("/cell/{}?site=N", c.reading)).await;
    assert_eq!(v["receipt"]["value"], "10");
    // not in the slice endpoint, not in a batch, not as a version
    let page = m.get(&format!("/slice/{}?site=*", c.reading)).await;
    let text = page.to_string();
    assert!(!text.contains("\"100\""), "{text}");
    let r = m.get_any(&format!("/versions/{}?site=N", c.double)).await;
    assert!(!String::from_utf8_lossy(&r.body).contains("\"200\""));
}

/// Scenarios in flight together keep to their own overrides.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_scenarios_do_not_leak() {
    let c = calc();
    let m = std::sync::Arc::new(mesh("wi-conc").await);
    let mut tasks = Vec::new();
    for i in 1..=24u32 {
        let m = m.clone();
        let c = c.clone();
        tasks.push(tokio::spawn(async move {
            let ov = canonical_overrides(&[(reading(&c, &m, "N"), &i.to_string())]);
            let r = m.whatif(&body(&call(&c, "double", "site: \"N\""), ov)).await;
            (i, scenario_value(&r))
        }));
    }
    for t in tasks {
        let (i, v) = t.await.unwrap();
        assert_eq!(v, (i * 2).to_string());
    }
    let v = m.get(&format!("/cell/{}?site=N", c.double)).await;
    assert_eq!(v["receipt"]["value"], "20");
}
