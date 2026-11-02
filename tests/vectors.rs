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
        return datumesh_http::vectors::excel_descriptor(v);
    }
    let f = v.get("cell")?.get("fn")?.as_str()?;
    v.get("world")?.get("functions")?.get(f)?.get("descriptor").cloned()
}

/// The vector's pinned URI path (`/cell/...`) and its function's hash.
fn request_of(v: &Value) -> (String, String) {
    let d = descriptor_of(v).unwrap();
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
    (format!("/cell/{}", &uri["cell://".len()..]), fn_hash)
}

/// Whether the envelope is the vector's answer: its value, its fault, `not_covered` for
/// E_FORMULA_NOT_COVERED, or the mesh's evaluation failure for its other codes.
fn as_said(v: &Value, env: &Value) -> bool {
    let o = env.get("outcome").and_then(Value::as_str).unwrap_or("");
    if v.get("valid") == Some(&Value::Bool(true)) {
        let want = v.get("value").unwrap();
        o == "ok"
            && env.get("receipt").and_then(|r| r.get("value")) == want.get("value")
            && env.get("receipt").and_then(|r| r.get("type")) == want.get("type")
    } else {
        let code = v.get("error").and_then(Value::as_str).unwrap();
        if datumesh_core::Fault::from_code(code).is_some() {
            o == "not_covered" && env.get("fault").and_then(Value::as_str) == Some(code)
        } else if code == "E_FORMULA_NOT_COVERED" {
            o == "not_covered" && env.get("fault").is_none()
        } else {
            o == "error" && env.get("error").and_then(Value::as_str) == Some("E_EVAL_UNAVAILABLE")
        }
    }
}

async fn mesh() -> Node {
    Node::start(A, |c| {
        c.rate.enabled = false;
        // The vectors' hosts are this mesh's own here (as on the test mesh): nothing is
        // fetched from elsewhere.
        c.cross_mesh = false;
        c.budget = std::time::Duration::from_secs(30);
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_formula_vector_by_pinned_hash() {
    let a = mesh().await;
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
    let mut alone = Vec::new();
    let (mut total, mut answered, mut said) = (0, 0, 0);
    for p in &files {
        let v = json::parse(&std::fs::read(p).unwrap()).unwrap();
        let id = v.get("id").and_then(Value::as_str).unwrap().to_string();
        total += 1;
        let (path, fn_hash) = request_of(&v);
        let resp = a.get(&path).await;
        let env = resp.json();
        let o = env.get("outcome").and_then(Value::as_str).unwrap_or("").to_string();
        if o != "bad_request" && env.get("fn_hash").and_then(Value::as_str) == Some(fn_hash.as_str()) {
            answered += 1;
        }
        if as_said(&v, &env) {
            said += 1;
            *counts.entry("as the vector says".into()).or_default() += 1;
        } else if v.get("fuel").is_some() {
            *counts.entry("otherwise: the vector's own fuel (a request has none)".into()).or_default() += 1;
        } else {
            alone.push((p.clone(), id, o, String::from_utf8_lossy(&resp.body).chars().take(160).collect::<String>()));
        }
    }
    drop(a);
    // A vector whose world (the functions its names are bound to, its cells' values) differs
    // from another vector's on one mesh: on a mesh holding its world alone, it answers as it says.
    let scratch = out_dir().join("vectors-alone");
    for (p, id, o, body) in alone {
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(scratch.join("v")).unwrap();
        std::fs::copy(&p, scratch.join("v").join(p.file_name().unwrap())).unwrap();
        let b = mesh().await;
        bl(|| datumesh_http::vectors::import(&*b.store, &scratch, &now()));
        let v = json::parse(&std::fs::read(&p).unwrap()).unwrap();
        let (path, _) = request_of(&v);
        let env = b.get(&path).await.json();
        if as_said(&v, &env) {
            *counts
                .entry("otherwise with every vector; as it says on a mesh with its world alone".into())
                .or_default() += 1;
            otherwise.push(format!("{id} (world shared with other vectors; alone: as it says): {o} {body}"));
        } else {
            *counts.entry("otherwise".into()).or_default() += 1;
            otherwise.push(format!("{id} (FAILS alone too): {o} {body}"));
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    let out = format!(
        "formula and excel vectors by pinned hash: {answered} of {total} answered (the pin echoed); {said} as the vector says; {counts:?}\n{}\n",
        otherwise.join("\n")
    );
    std::fs::write(out_dir().join("vectors.txt"), &out).unwrap();
    eprint!("{out}");
    assert_eq!(answered, total, "{out}");
    assert!(!otherwise.iter().any(|x| x.contains("FAILS alone")), "{out}");
}
