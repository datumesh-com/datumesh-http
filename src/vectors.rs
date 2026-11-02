//! `import-vectors`: the spec's vectors' cells into a store (see `deploy/README.md`).
//!
//! What it puts, and nothing else: the key documents of the receipt vectors (each retrieved
//! document) and of the formula vectors' hosts (the test keys of SPEC 11), merged per host;
//! every descriptor the vectors give (a receipt vector's, every formula and excel vector's
//! function, valid or not: a pinned request evaluates any of them); every receipt (each
//! verified by `put_receipt`); the manifests; the names the valid vectors bind (a name bound
//! to several functions keeps the first, reported), and the names only invalid vectors bind
//! (their first function); the stored cells' versions.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use datumesh_core::json::{self, Value};
use datumesh_core::{keys_from_response, FunctionDescriptor, Manifest, Receipt, SigningKey};
use datumesh_store::Store;

/// The formula vectors' conventions (SPEC 11): each host's key from the seed
/// SHA-256("datumesh formula test key: <host>"), valid from 2020-01-01.
const KEY_FROM: &str = "2020-01-01T00:00:00.000000Z";

fn files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut es: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
    es.sort();
    for p in es {
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|e| e == "json") {
            out.push(p);
        }
    }
}

fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

fn num(n: usize) -> Value {
    json::parse(n.to_string().as_bytes()).unwrap_or(Value::Null)
}

/// An excel vector's function (SPEC 11): `{"code": {"body": <body>, "lang": "formula"},
/// "inputs": <the derived inputs>, "signature": <signature>}`, the inputs the names its body
/// reads.
pub fn excel_descriptor(v: &Value) -> Option<Value> {
    let body = v.get("body")?.as_str()?;
    let ast = datumesh_formula::canonical_ast(body, None).ok()?;
    // The reads of the canonical AST (its JCS form: objects with "node": "read" and "fn").
    fn reads(v: &Value, out: &mut BTreeSet<String>) {
        match v {
            Value::Object(m) => {
                if v.get("node").and_then(Value::as_str) == Some("read") {
                    if let Some(f) = v.get("fn").and_then(Value::as_str) {
                        out.insert(f.to_string());
                    }
                }
                m.iter().for_each(|(_, x)| reads(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| reads(x, out)),
            _ => {}
        }
    }
    let mut names = BTreeSet::new();
    reads(&json::parse(&ast.to_jcs()).ok()?, &mut names);
    Some(Value::Object(vec![
        ("code".into(), Value::Object(vec![("body".into(), s(body)), ("lang".into(), s("formula"))])),
        ("inputs".into(), Value::Array(names.into_iter().map(|n| s(&n)).collect())),
        ("signature".into(), v.get("signature")?.clone()),
    ]))
}

/// Imports the vectors under `dir` (a spec's `vectors/`) into `store` (see the module docs);
/// a report as JSON: what was put, and what was refused or skipped.
pub fn import<S: Store + ?Sized>(store: &S, dir: &Path, now: &str) -> Value {
    let mut paths = Vec::new();
    files(dir, &mut paths);
    let vs: Vec<Value> = paths.iter().filter_map(|p| json::parse(&std::fs::read(p).ok()?).ok()).collect();
    let valid = |v: &Value| v.get("valid") == Some(&Value::Bool(true));
    let kind = |v: &Value| v.get("kind").and_then(Value::as_str).unwrap_or("").to_string();

    // Key documents: every valid receipt vector's retrieved documents and the formula
    // vectors' hosts' test keys, merged per host by key id.
    let mut keys: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    let mut world_hosts = BTreeSet::new();
    for v in &vs {
        if kind(v) == "receipt" && valid(v) {
            for (host, env) in v.get("keys").and_then(Value::as_object).unwrap_or(&[]) {
                let https = env.get("https");
                if https.and_then(|h| h.get("status")).and_then(Value::as_number_text) != Some("200") {
                    continue;
                }
                let Some(body) = https.and_then(|h| h.get("body")).and_then(Value::as_str) else { continue };
                if keys_from_response(host, 200, Some(body.as_bytes())).is_err() {
                    continue;
                }
                let doc = json::parse(body.as_bytes()).unwrap_or(Value::Null);
                if let Some(Value::Array(ks)) = doc.get("keys") {
                    for k in ks {
                        if let Some(id) = k.get("id").and_then(Value::as_str) {
                            keys.entry(host.clone()).or_default().entry(id.to_string()).or_insert_with(|| k.clone());
                        }
                    }
                }
            }
        }
        if matches!(kind(v).as_str(), "formula-eval" | "excel") {
            if let Some(w) = v.get("world") {
                for (name, _) in w.get("functions").and_then(Value::as_object).unwrap_or(&[]) {
                    world_hosts.insert(name.split('/').next().unwrap_or("").to_string());
                }
                if let Some(Value::Array(rs)) = w.get("receipts") {
                    for r in rs {
                        if let Some(h) = r.get("computed_by").and_then(Value::as_str) {
                            world_hosts.insert(h.split('#').next().unwrap_or("").to_string());
                        }
                    }
                }
            }
        }
    }
    for host in &world_hosts {
        let key =
            SigningKey::from_seed(&datumesh_core::sha256(format!("datumesh formula test key: {host}").as_bytes()));
        let k = Value::Object(vec![
            ("alg".into(), s("ed25519")),
            ("id".into(), s(key.key_id())),
            ("public".into(), s(&datumesh_core::lexical::to_hex(&key.public_key()))),
            ("valid_from".into(), s(KEY_FROM)),
        ]);
        keys.entry(host.clone()).or_default().entry(key.key_id().to_string()).or_insert(k);
    }
    let mut namespaces = 0;
    for (host, ks) in &keys {
        let doc = Value::Object(vec![
            ("host".into(), s(host)),
            ("keys".into(), Value::Array(ks.values().take(64).cloned().collect())),
            ("proto".into(), s("datumesh/1")),
        ])
        .to_jcs();
        if store.namespace(host).ok().flatten().map(|n| n.keydoc) != Some(doc.clone())
            && store.put_namespace(host, &doc, now).is_ok()
        {
            namespaces += 1;
        }
    }

    // Descriptors, receipts, manifests; candidate bindings and versions.
    let mut descriptors: BTreeMap<String, String> = BTreeMap::new(); // fn_hash -> lang
    let mut bindings: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new(); // name -> (since, fn_hash)
    let mut invalid_bindings: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut versions: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new(); // cell id -> (tx, hash, fn_hash)
    let (mut receipts, mut refused, mut manifests) = (0usize, Vec::new(), 0usize);
    let mut blobs = 0usize;
    let put_descriptor = |d: &Value, descriptors: &mut BTreeMap<String, String>| -> Option<String> {
        let fd = FunctionDescriptor::from_value(d).ok()?;
        let h = store.put_descriptor(&fd.canonical()).ok()?;
        descriptors.insert(h.clone(), fd.lang.clone());
        Some(h)
    };
    let mut put_receipt =
        |bytes: &[u8], id: &str, versions: &mut BTreeMap<String, Vec<(String, String, String)>>| match (
            store.put_receipt(bytes),
            Receipt::from_json(bytes),
        ) {
            (Ok(h), Ok(r)) => {
                receipts += 1;
                versions.entry(r.cell().id()).or_default().push((r.tx().to_string(), h, r.cell().fn_hash.clone()));
            }
            (Err(e), _) => refused.push(format!("{id}: {e}")),
            _ => {}
        };
    for v in &vs {
        let id = v.get("id").and_then(Value::as_str).unwrap_or("?").to_string();
        match kind(v).as_str() {
            "receipt" if valid(v) => {
                let Some(text) = v.get("input").and_then(Value::as_str) else { continue };
                let rc = v.get("recompute");
                if let (Some(d), Ok(r)) = (rc.and_then(|r| r.get("descriptor")), Receipt::from_json(text.as_bytes())) {
                    if let Some(h) = put_descriptor(d, &mut descriptors) {
                        if h == r.cell().fn_hash {
                            bindings.entry(r.cell().function_name()).or_default().push((r.tx().to_string(), h));
                        }
                    }
                }
                for (h, t) in rc.and_then(|r| r.get("inputs")).and_then(Value::as_object).unwrap_or(&[]) {
                    if let Some(t) = t.as_str() {
                        put_receipt(t.as_bytes(), &format!("{id} input {h}"), &mut versions);
                    }
                }
                for (_, t) in rc.and_then(|r| r.get("manifests")).and_then(Value::as_object).unwrap_or(&[]) {
                    if t.as_str().is_some_and(|t| store.put_manifest(t.as_bytes()).is_ok()) {
                        manifests += 1;
                    }
                }
                put_receipt(text.as_bytes(), &id, &mut versions);
            }
            "slice" if valid(v) => {
                if let Some(t) = v.get("input").and_then(Value::as_str) {
                    if Manifest::from_json(t.as_bytes()).is_ok() && store.put_manifest(t.as_bytes()).is_ok() {
                        manifests += 1;
                    }
                }
            }
            "formula-eval" | "excel" => {
                // Every vector's functions (so a pinned request evaluates it), bound by name
                // from the valid ones only; an excel vector's function is its printed body
                // under its signature (SPEC 11), known by hash only.
                if kind(v) == "excel" {
                    if let Some(d) = excel_descriptor(v) {
                        put_descriptor(&d, &mut descriptors);
                    }
                }
                let Some(w) = v.get("world") else { continue };
                for (_, hex) in w.get("blobs").and_then(Value::as_object).unwrap_or(&[]) {
                    let bytes: Option<Vec<u8>> = hex.as_str().and_then(|h| {
                        (0..h.len()).step_by(2).map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok()).collect()
                    });
                    if let Some(b) = bytes {
                        if store.put_blob(&b).is_ok() {
                            blobs += 1;
                        }
                    }
                }
                for (name, f) in w.get("functions").and_then(Value::as_object).unwrap_or(&[]) {
                    let (Some(d), Some(b)) = (f.get("descriptor"), f.get("bound").and_then(Value::as_str)) else {
                        continue;
                    };
                    if let Some(h) = put_descriptor(d, &mut descriptors) {
                        let to = if valid(v) { &mut bindings } else { &mut invalid_bindings };
                        to.entry(name.clone()).or_default().push((b.to_string(), h));
                    }
                }
                if let Some(Value::Array(rs)) = w.get("receipts") {
                    for r in rs {
                        put_receipt(&r.to_jcs(), &format!("{id} world"), &mut versions);
                    }
                }
            }
            _ => {}
        }
    }

    // Bindings: a name with one function is bound from its earliest instant; a name the
    // vectors bind to several functions keeps the first (one name means one function at a
    // time) and the others are reported.
    let (mut bound, mut skipped) = (0usize, Vec::new());
    for (name, cs) in invalid_bindings {
        bindings.entry(name).or_insert(cs);
    }
    for (name, cs) in &bindings {
        let hashes: BTreeSet<&String> = cs.iter().map(|(_, h)| h).collect();
        let (since, h) = cs.iter().filter(|(_, h)| *h == cs[0].1).min().cloned().unwrap_or_default();
        if hashes.len() > 1 {
            skipped.push(format!("{name}: {} functions, the first bound", hashes.len()));
        }
        match store.resolve_binding(name, "9999-12-31T23:59:59.999999Z") {
            Ok(Some(b)) if b.fn_hash == h => {}
            Ok(None) => {
                if store.bind_name(name, &h, &since).is_ok() {
                    bound += 1;
                }
            }
            _ => skipped.push(format!("{name}: already bound otherwise")),
        }
    }
    // Versions of stored functions' cells, in tx order (a second receipt at one tx is not a
    // version).
    let mut stored_versions = 0usize;
    for (cell_id, mut vs) in versions {
        vs.sort();
        vs.dedup_by(|a, b| a.0 == b.0);
        for (tx, hash, fn_hash) in vs {
            if descriptors.get(&fn_hash).map(String::as_str) != Some("stored") {
                continue;
            }
            match store.latest(&cell_id) {
                Ok(Some(l)) if l.tx >= tx => continue,
                _ => {}
            }
            if store.put_version(&cell_id, &tx, &hash).is_ok() {
                stored_versions += 1;
            }
        }
    }
    Value::Object(vec![
        ("bindings".into(), num(bound)),
        ("blobs".into(), num(blobs)),
        ("bindings_skipped".into(), Value::Array(skipped.iter().map(|x| s(x)).collect())),
        ("descriptors".into(), num(descriptors.len())),
        ("manifests".into(), num(manifests)),
        ("namespaces".into(), num(namespaces)),
        ("receipts".into(), num(receipts)),
        ("receipts_refused".into(), Value::Array(refused.iter().map(|x| s(x)).collect())),
        ("versions".into(), num(stored_versions)),
    ])
}
