//! Test 10 of the brief: `POST /verify` verifies every receipt vector of the spec and every
//! receipt the engine issues; tampered receipts fail with the right code.

mod common;

use std::collections::BTreeSet;

use common::*;
use datumesh_core::json::{self, Value};
use datumesh_core::{Receipt, ReceiptBuilder};

const A: &str = "mesh-a.example";

fn result(r: &Resp) -> (bool, Option<String>, Option<String>, Option<String>) {
    let v = r.json();
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    (v.get("ok") == Some(&Value::Bool(true)), s("level"), s("code"), s("hash"))
}

fn hexs(b: &[u8]) -> String {
    datumesh_core::lexical::to_hex(b)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn receipt_vectors_and_engine_receipts() {
    let a = Node::start(A, |c| c.rate.enabled = false).await;
    seed(&a, 10);

    // Every receipt vector, at level signed, with the vector's key documents in the mesh's
    // key cache (a failed retrieval recorded as such).
    let mut files = Vec::new();
    for dir in ["receipt", "invalid"] {
        for e in std::fs::read_dir(spec_dir().join("vectors").join(dir)).unwrap() {
            files.push(e.unwrap().path());
        }
    }
    files.sort();
    let (mut valid, mut invalid, skipped): (usize, usize, Vec<String>) = (0, 0, Vec::new());
    let mut hosts: BTreeSet<String> = BTreeSet::new();
    for p in files {
        let v = json::parse(&std::fs::read(&p).unwrap()).unwrap();
        if v.get("kind").and_then(Value::as_str) != Some("receipt") {
            continue;
        }
        let id = v.get("id").and_then(Value::as_str).unwrap().to_string();
        for h in &hosts {
            a.mesh.keys().remove(h);
        }
        hosts.clear();
        if let Some(ks) = v.get("keys").and_then(Value::as_object) {
            for (host, env) in ks {
                let https = env.get("https");
                let status = https
                    .and_then(|h| h.get("status"))
                    .and_then(Value::as_number_text)
                    .and_then(|s| s.parse::<u16>().ok())
                    .unwrap_or(0);
                let body = https.and_then(|h| h.get("body")).and_then(Value::as_str);
                a.mesh.keys().insert_response(host, status, body.map(str::as_bytes));
                hosts.insert(host.clone());
            }
        }
        let input = v.get("input").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| {
            let h = v.get("input_hex").and_then(Value::as_str).unwrap();
            let raw: Vec<u8> = (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect();
            String::from_utf8_lossy(&raw).into_owned()
        });
        let raw_input = v.get("input_hex").is_some();
        // The offered form: the vector's recompute object for step 7.
        let mut body = vec![("receipt".to_string(), Value::String(input.clone()))];
        if let Some(rc) = v.get("recompute") {
            body.push(("recompute".into(), rc.clone()));
        }
        if let Some(b) = v.get("blob_hex") {
            body.push(("blob_hex".into(), b.clone()));
        }
        let r = if raw_input {
            // Bytes that are not UTF-8 text go as they are, at level signed.
            let h = v.get("input_hex").and_then(Value::as_str).unwrap();
            let raw: Vec<u8> = (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect();
            a.post("/verify?level=signed", raw).await
        } else {
            a.post("/verify?form=offered", Value::Object(body).to_jcs()).await
        };
        assert_eq!(r.status, 200, "{id}: {}", r.text());
        let (ok, level, code, hash) = result(&r);
        if v.get("valid") == Some(&Value::Bool(true)) {
            assert!(ok, "{id}: {}", r.text());
            assert_eq!(level.as_deref(), v.get("level").and_then(Value::as_str), "{id}");
            assert_eq!(hash.as_deref(), v.get("sha256").and_then(Value::as_str), "{id}");
            assert_eq!(code.as_deref(), v.get("code").and_then(Value::as_str), "{id}");
            if v.get("blob_hex").is_some() {
                assert_eq!(r.json().get("blob_checked"), Some(&Value::Bool(true)), "{id}");
            }
            valid += 1;
        } else {
            let want = v.get("error").and_then(Value::as_str).unwrap();
            assert!(!ok, "{id} verified, want {want}: {}", r.text());
            assert_eq!(code.as_deref(), Some(want), "{id}: {}", r.text());
            invalid += 1;
        }
    }
    for h in &hosts {
        a.mesh.keys().remove(h);
    }

    // Every receipt the engine issued in this test, at level recomputed.
    let mut engine = Vec::new();
    for path in [
        format!("/cell/{A}/double?ccy=USD&date=2026-03-01"),
        format!("/cell/{A}/double?ccy=GBP&date=2026-03-01&asof={T2}"),
        format!("/cell/{A}/total?date=2026-03-01"),
        format!("/cell/{A}/slow?tag=v"),
    ] {
        let r = a.get(&path).await;
        assert_eq!(r.outcome(), "ok", "{path}: {}", r.text());
        engine.push(r.json().get("receipt").unwrap().to_jcs());
    }
    for bytes in &engine {
        let r = a.post("/verify", bytes.clone()).await;
        let (ok, level, _, hash) = result(&r);
        assert!(ok, "{}", r.text());
        assert_eq!(level.as_deref(), Some("recomputed"));
        assert_eq!(hash.as_deref(), Some(Receipt::from_json(bytes).unwrap().hash()));
    }
    // A stored receipt the mesh holds, recomputed from the store.
    let stored =
        a.get(&format!("/cell/{A}/rate?ccy=USD&date=2026-03-01")).await.json().get("receipt").unwrap().to_jcs();
    assert_eq!(result(&a.post("/verify", stored.clone()).await).1.as_deref(), Some("recomputed"));

    // Tampered receipts.
    let good = String::from_utf8(engine[0].clone()).unwrap();
    let rec = Receipt::from_json(&engine[0]).unwrap();
    let sig = rec.sig().to_string();
    let flipped = format!("{}{}", &sig[..127], if sig.ends_with('0') { '1' } else { '0' });
    let forged = ReceiptBuilder::new()
        .cell(rec.cell().clone())
        .value(n("9.99"))
        .valid(rec.valid().clone())
        .inputs(rec.inputs().iter().cloned())
        .asof(rec.asof())
        .tx(rec.tx())
        .issued(rec.issued())
        .publisher(rec.publisher())
        .signer_host(rec.signer_host())
        .licence(rec.licence())
        .summary(rec.summary())
        .sign(&a.key)
        .unwrap();
    let unknown_host = ReceiptBuilder::new()
        .cell(rec.cell().clone())
        .value(rec.value().clone())
        .valid(rec.valid().clone())
        .asof(rec.asof())
        .tx(rec.tx())
        .issued(rec.issued())
        .publisher(rec.publisher())
        .signer_host("nowhere.example")
        .licence(rec.licence())
        .summary(rec.summary())
        .sign(&a.key)
        .unwrap();
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        (
            "value changed",
            good.replacen(&format!("\"value\":\"{}\"", rec.value().lexical()), "\"value\":\"2.17\"", 1).into_bytes(),
            "E_SIGNATURE",
        ),
        ("signature flipped", good.replacen(&sig, &flipped, 1).into_bytes(), "E_SIGNATURE"),
        ("truncated", good.as_bytes()[..good.len() - 2].to_vec(), "E_PARSE_JSON"),
        (
            "duplicate member",
            good.replacen("{", &format!("{{\"tx\":\"{}\",", rec.tx()), 1).into_bytes(),
            "E_PARSE_DUPLICATE_KEY",
        ),
        ("unknown key id", good.replacen(rec.key_id(), "0123456789abcdef", 1).into_bytes(), "E_KEY_UNKNOWN"),
        ("unknown signer host", receipt_json(&unknown_host), "E_KEY_UNAVAILABLE"),
        ("a value the inputs do not give", receipt_json(&forged), "E_VALUE_MISMATCH"),
        ("not a receipt", b"{\"proto\":\"datumesh/1\"}".to_vec(), "E_SCHEMA"),
    ];
    let mut tampered = 0;
    for (what, body, want) in &cases {
        let r = a.post("/verify", body.clone()).await;
        let (ok, _, code, _) = result(&r);
        assert!(!ok, "{what}: {}", r.text());
        assert_eq!(code.as_deref(), Some(*want), "{what}: {}", r.text());
        tampered += 1;
    }
    // Blob content checked by step 7.7 when given.
    let _ = hexs(b"");

    let out = format!(
        "verify: {valid} valid and {invalid} invalid receipt vectors as the spec says; {} skipped ({skipped:?}); {} engine receipts and 1 stored at recomputed; {tampered} tampered receipts with their codes\n",
        skipped.len(),
        engine.len()
    );
    std::fs::write(out_dir().join("verify.txt"), &out).unwrap();
    eprint!("{out}");
    assert!(valid > 30 && invalid > 60 && skipped.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_is_rate_limited_per_ip() {
    let a = Node::start(A, |c| c.rate.verify_per_ip = 3).await;
    for _ in 0..3 {
        assert_eq!(a.post("/verify", b"{}".to_vec()).await.status, 200);
    }
    let r = a.post("/verify", b"{}".to_vec()).await;
    assert_eq!(r.status, 429);
    assert_eq!(r.outcome(), "rate_limited");
    assert!(r.header("retry-after").is_some());
}
