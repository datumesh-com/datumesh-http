//! spec-0.4.2: every formula and excel vector of the spec answers by pinned hash
//! (`/cell/<name>@<fn_hash>(args)`), once `import_vectors` has loaded the vectors' descriptors,
//! receipts, bindings and versions; and the answer is the vector's (its value, its fault, or
//! the mesh's failure for its other codes). The count goes to target-out/vectors.txt.

mod common;

use std::collections::BTreeMap;

use common::*;
use datumesh_core::json::{self, Value};
use datumesh_core::{format_reference, Arg, Args, Reference, Scalar};

const A: &str = "mesh-a.example";

fn descriptor_of(v: &Value) -> Option<Value> {
    if v.get("kind").and_then(Value::as_str) == Some("excel") {
        // SPEC 11: an excel vector's function is its printed body under its signature.
        return Some(Value::Object(vec![
            ("code".into(), Value::Object(vec![("body".into(), v.get("body")?.clone()), ("lang".into(), Value::String("formula".into()))])),
            ("inputs".into(), Value::Array(vec![])),
            ("signature".into(), v.get("signature")?.clone()),
        ]));
    }
    let f = v.get("cell")?.get("fn")?.as_str()?;
    v.get("world")?.get("functions")?.get(f)?.get("descriptor").cloned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_formula_vector_by_pinned_hash() {
    let a = Node::start(A, |c| {
        c.rate.enabled = false;
        c.budget = std::time::Duration::from_secs(30);
    })
    .await;
    let dir = spec_dir().join("vectors");
    let report = bl(|| datumesh_http::vectors::import(&*a.store, &dir, &now()));
    eprintln!("import: {}", String::from_utf8_lossy(&report.to_jcs()));
    let mut files = Vec::new();
    for g in ["formula-eval", "excel"] {
        for e in std::fs::read_dir(dir.join(g)).unwrap() {
            files.push(e.unwrap().path());
        }
    }
    files.sort();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut otherwise = Vec::new();
    let (mut total, mut answered, mut as_said) = (0, 0, 0);
    for p in files {
        let v = json::parse(&std::fs::read(&p).unwrap()).unwrap();
        let id = v.get("id").and_then(Value::as_str).unwrap().to_string();
        total += 1;
        let d = descriptor_of(&v).unwrap();
        let fn_hash = datumesh_core::FunctionDescriptor::from_value(&d).unwrap().hash();
        let cell = v.get("cell").unwrap();
        let (ns, f) = cell.get("fn").and_then(Value::as_str).unwrap().rsplit_once('/').unwrap();
        let mut a_: Args = Args::new();
        for (k, x) in cell.get("args").and_then(Value::as_object).unwrap() {
            a_.insert(k.clone(), Arg::Value(Scalar::from_json(&x.to_jcs()).unwrap()));
        }
        let mut r = Reference::new(ns, f, a_);
        r.fn_hash = Some(fn_hash.clone());
        r.asof = v.get("asof").and_then(Value::as_str).map(str::to_string);
        let uri = format_reference(&r);
        let resp = a.get(&format!("/cell/{}", &uri["cell://".len()..])).await;
        let env = resp.json();
        let o = env.get("outcome").and_then(Value::as_str).unwrap_or("").to_string();
        if o != "bad_request" && env.get("fn_hash").and_then(Value::as_str) == Some(fn_hash.as_str()) {
            answered += 1;
        }
        let fuel_limited = v.get("fuel").is_some();
        let ok = if v.get("valid") == Some(&Value::Bool(true)) {
            let want = v.get("value").unwrap();
            o == "ok" && env.get("receipt").and_then(|r| r.get("value")) == want.get("value") && env.get("receipt").and_then(|r| r.get("type")) == want.get("type")
        } else {
            let code = v.get("error").and_then(Value::as_str).unwrap();
            if datumesh_core::Fault::from_code(code).is_some() {
                o == "not_covered" && env.get("fault").and_then(Value::as_str) == Some(code)
            } else if code == "E_FORMULA_NOT_COVERED" {
                o == "not_covered" && env.get("fault").is_none()
            } else {
                o == "error" && env.get("error").and_then(Value::as_str) == Some("E_EVAL_UNAVAILABLE")
            }
        };
        let class = match (ok, fuel_limited) {
            (true, _) => "as the vector says",
            (false, true) => "otherwise: the vector's own fuel (a request has none)",
            (false, false) => "otherwise",
        };
        *counts.entry(class.to_string()).or_default() += 1;
        if ok {
            as_said += 1;
        } else if !fuel_limited {
            otherwise.push(format!("{id}: {o} {}", String::from_utf8_lossy(&resp.body).chars().take(160).collect::<String>()));
        }
    }
    let out = format!(
        "formula and excel vectors by pinned hash: {answered} of {total} answered (the pin echoed); {as_said} as the vector says; {counts:?}\n{}\n",
        otherwise.join("\n")
    );
    std::fs::write(out_dir().join("vectors.txt"), &out).unwrap();
    eprint!("{out}");
    assert_eq!(answered, total, "{out}");
    assert!(otherwise.is_empty(), "{out}");
}
