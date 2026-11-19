//! Build 2 Part 3, E3 at the HTTP surface: a cycle names its path and offers the fixes (spec-0.6.0,
//! 5.5 `cycle`, 15.12).
//!
//! * a cycle is an `error` envelope (HTTP 500-599) with `error` `E_EVAL_UNAVAILABLE` and the member
//!   `cycle`: exactly `path` (canonical calls in read order, the first and the last equal) and
//!   `fixes` (the subset of `asof`, `lag`, `step`, `solver`, in that order): `asof` and `solver`
//!   always, `lag` when the cycle passes through a function with a `date` or `instant` argument,
//!   `step` when through one with a `number` argument;
//! * the check is keyed on the instant: A rebound to read B as of an instant T before the rebind,
//!   `asof(B, T)`, while B reads A, is not a cycle;
//! * an override can never make a cycle: `whatif` with the cycle's cell overridden answers.
//!
//! The functions: `a` and `b`, both over `site: string` and optionally a `d: date` and an
//! `n: number`. Run 1 binds `a` (reads the stored `reading`) and `b` (reads `a`). Run 2, later,
//! rebinds `a`: naively to `b(...)`, or to `asof(b(...), T)` with T between the runs.

mod common;
#[path = "common/calculus.rs"]
mod calculus;

use calculus::*;
use common::pub031::function_line;
use common::readpath::*;
use serde_json::{json, Value};

const C1: &str = "2026-11-01T00:00:00.000000Z";
const T: &str = "2026-11-10T00:00:00.000000Z";
const C2: &str = "2026-11-12T00:00:00.000000Z";
const LATER: &str = "2026-11-15T00:00:00.000000Z";

struct World {
    a: String,
    b: String,
    reading: String,
    date: bool,
    number: bool,
}

impl World {
    fn new(prefix: &str, date: bool, number: bool) -> World {
        let c = Calc::new(prefix);
        World { a: format!("{prefix}/a"), b: format!("{prefix}/b"), reading: c.reading, date, number }
    }

    fn sig(&self) -> String {
        let mut args = vec![r#""site":{"type":"string","valid":false}"#.to_string()];
        let mut order = vec![r#""site""#.to_string()];
        if self.date {
            args.push(r#""d":{"type":"date","valid":true}"#.into());
            order.push(r#""d""#.into());
        }
        if self.number {
            args.push(r#""n":{"type":"number","unit":"1","valid":false}"#.into());
            order.push(r#""n""#.into());
        }
        format!(
            r#"{{"args":{{{}}},"order":[{}],"returns":{{"type":"number","unit":"1"}}}}"#,
            args.join(","),
            order.join(",")
        )
    }

    /// The labelled arguments of a read of the other function.
    fn passing(&self) -> String {
        let mut v = vec!["site: site"];
        if self.date {
            v.push("d: d");
        }
        if self.number {
            v.push("n: n");
        }
        v.join(", ")
    }

    /// A body that uses every declared argument (a date and a number add nothing).
    fn uses(&self) -> String {
        let mut s = format!("{}(site: site)", self.reading);
        if self.date {
            s += " + day(d) * 0";
        }
        if self.number {
            s += " + n * 0";
        }
        s
    }

    fn run1(&self) -> Vec<String> {
        vec![
            function_line(&self.reading, &reading_sig(), None),
            site_cell(&self.reading, "N", "10"),
            formula_line(&self.a, &self.uses(), &self.sig(), None),
            formula_line(&self.b, &format!("{}({}) + 1", self.a, self.passing()), &self.sig(), None),
        ]
    }

    /// `a` rebound to read `b` now: the naive cycle.
    fn naive(&self) -> Vec<String> {
        vec![formula_line(&self.a, &format!("{}({})", self.b, self.passing()), &self.sig(), None)]
    }

    /// `a` rebound to read `b` as of T: the feedback pattern.
    fn feedback(&self) -> Vec<String> {
        vec![formula_line(&self.a, &format!("asof({}({}), {T})", self.b, self.passing()), &self.sig(), None)]
    }

    fn query(&self) -> String {
        let mut q = "site=N".to_string();
        if self.date {
            q += "&d=2026-03-02";
        }
        if self.number {
            q += "&n=1";
        }
        q
    }

    fn call(&self, f: &str) -> String {
        let mut parts = Vec::new();
        if self.date {
            parts.push("d: 2026-03-02");
        }
        if self.number {
            parts.push("n: 1");
        }
        parts.push("site: \"N\"");
        format!("{f}({})", parts.join(", "))
    }
}

async fn serve(label: &str, w: &World, second: Vec<String>) -> Served {
    serve_runs(label, &[(C1, w.run1()), (C2, second)], None).await
}

fn path_of(v: &Value) -> Vec<String> {
    v["cycle"]["path"].as_array().unwrap().iter().map(|p| norm(p.as_str().unwrap())).collect()
}

/// The naive rebind: the path, and the fixes by the arguments of the functions the cycle passes
/// through.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cycle_names_its_path_and_its_fixes() {
    let table: [(bool, bool, &[&str]); 4] = [
        (false, false, &["asof", "solver"]),
        (true, false, &["asof", "lag", "solver"]),
        (false, true, &["asof", "step", "solver"]),
        (true, true, &["asof", "lag", "step", "solver"]),
    ];
    for (i, (date, number, fixes)) in table.into_iter().enumerate() {
        let w = World::new(&format!("mesh-test.datumesh.com/scratch_b3/cy{i}"), date, number);
        let m = serve(&format!("cy-{i}"), &w, w.naive()).await;
        let r = m.get_any(&format!("/cell/{}?{}", w.a, w.query())).await;
        assert!((500..600).contains(&r.status), "{i}: {} {}", r.status, String::from_utf8_lossy(&r.body));
        let v = &r.json;
        assert_eq!(
            (v["outcome"].as_str(), v["error"].as_str()),
            (Some("error"), Some("E_EVAL_UNAVAILABLE")),
            "{i}: {v}"
        );
        let mut keys: Vec<&str> = v["cycle"].as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["fixes", "path"], "{i}: {v}");
        let want: Vec<String> = [w.call(&w.a), w.call(&w.b), w.call(&w.a)].iter().map(|c| norm(c)).collect();
        assert_eq!(path_of(v), want, "{i}: A -> B -> A");
        let got: Vec<&str> = v["cycle"]["fixes"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
        assert_eq!(got, fixes, "{i}");
        // the same cycle seen from B: B -> A -> B
        let r = m.get_any(&format!("/cell/{}?{}", w.b, w.query())).await;
        let want: Vec<String> = [w.call(&w.b), w.call(&w.a), w.call(&w.b)].iter().map(|c| norm(c)).collect();
        assert_eq!(path_of(&r.json), want, "{i}: B -> A -> B");
    }
}

/// A cycle at a requested asof, and in a batch: the same member.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cycle_at_an_asof_and_in_a_batch() {
    let w = World::new("mesh-test.datumesh.com/scratch_b3/cyb", false, false);
    let m = serve("cy-batch", &w, w.naive()).await;
    let r = m.get_any(&format!("/cell/{}?{}&asof={LATER}", w.a, w.query())).await;
    assert_eq!(r.json["error"], "E_EVAL_UNAVAILABLE", "{}", r.json);
    assert_eq!(r.json["cycle"]["path"].as_array().unwrap().len(), 3);
    let r = m.post_raw("/cells", json!([format!("cell://{}?{}", w.a, w.query())]).to_string().into_bytes()).await;
    assert_eq!(r.status, 200, "{}", r.json);
    let text = r.json["results"][0].to_string();
    assert!(text.contains("E_EVAL_UNAVAILABLE") && text.contains("cycle"), "{text}");
}

/// The feedback pattern is not a cycle: `a` reads `b` as of T, before the rebind; `b` reads the old
/// `a`. The receipt's lineage holds `b`'s receipt as of T.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_read_as_of_before_the_rebind_is_not_a_cycle() {
    let w = World::new("mesh-test.datumesh.com/scratch_b3/fb", false, false);
    let m = serve("cy-fb", &w, w.feedback()).await;
    for path in [format!("/cell/{}?{}", w.a, w.query()), format!("/cell/{}?{}&asof={LATER}", w.a, w.query())] {
        let v = m.get(&path).await;
        assert_eq!(v["outcome"], "ok", "{path}: {v}");
        // a(N) now = b(N) as of T = (reading 10) + 1
        assert_eq!(v["receipt"]["value"], "11", "{v}");
        let inputs: Vec<&str> =
            v["receipt"]["inputs"].as_array().unwrap().iter().map(|h| h.as_str().unwrap()).collect();
        assert!(!inputs.is_empty(), "{v}");
        let mut found = false;
        for h in inputs {
            let r = m.get(&format!("/receipt/{h}")).await;
            if r["cell"]["fn"] == "b" {
                found = true;
                assert!(r["asof"].as_str().unwrap() <= T, "b's receipt is as of T: {r}");
                assert_eq!(r["value"], "11");
            }
        }
        assert!(found, "b's receipt is in the lineage: {v}");
    }
    // b read now reads the rebound a, which reads b as of T: not a cycle either
    let v = m.get(&format!("/cell/{}?{}", w.b, w.query())).await;
    assert_eq!(v["outcome"], "ok", "{v}");
}

/// An override can never create a cycle: in the cyclic world, overriding `a` answers, and so does
/// reading `b` with `a` overridden (it is a constant there).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_override_breaks_the_cycle() {
    let w = World::new("mesh-test.datumesh.com/scratch_b3/cyo", false, false);
    let m = serve("cy-ov", &w, w.naive()).await;
    let a_cell = cell_json(&m, &w.a, &[("site", "string", "N")]);
    let ov = canonical_overrides(&[(a_cell, "5")]);
    let five = json!([{"cell": w.call(&w.a), "value": {"type": "number", "value": "5"}}]);
    let r = m.whatif(&json!({"cell": w.call(&w.b), "overrides": five})).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(scenario_value(&r), "6");
    let b_cell = cell_json(&m, &w.b, &[("site", "string", "N")]);
    check_scenario(&r, &b_cell, None, &ov, None);
    let r = m.whatif(&json!({"cell": w.call(&w.a), "overrides": five})).await;
    assert_eq!(scenario_value(&r), "5");
    // without the override the cycle is still there
    let v = m.get_any(&format!("/cell/{}?{}", w.a, w.query())).await.json;
    assert_eq!(v["error"], "E_EVAL_UNAVAILABLE");
}
