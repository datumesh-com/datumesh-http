//! Test 1 of the brief: every endpoint has a contract test, every envelope the mesh emits
//! passes the spec's envelope schema (and the core's envelope check against the request),
//! and every outcome is reachable: ok, stale, not_covered (with and without fault),
//! unlicensed, payment_required, rate_limited, bad_request (every URI code, and
//! E_JOB_UNKNOWN), error, pending.

mod common;

use std::collections::BTreeSet;
use std::time::Duration;

use common::*;
use datumesh_core::json::Value;
use datumesh_core::{keys_from_response, resolve_mesh, Keys, Receipt, Retrieval};

const A: &str = "mesh-a.example";
const B: &str = "mesh-b.example";

struct Seen {
    outcomes: BTreeSet<String>,
    schemas: Schemas,
}

impl Seen {
    /// Asserts the response is a schema-valid envelope with `outcome`, which the core's
    /// check accepts for the request; records the outcome.
    #[track_caller]
    fn envelope(&mut self, r: &Resp, outcome: &str, keys: &Keys, cell_id: &str, asof: Option<&str>, job: Option<&str>) {
        self.schemas.assert_envelope(&r.body);
        assert_eq!(r.outcome(), outcome, "{}", r.text());
        assert_eq!(r.header("content-type").as_deref(), Some("application/json"));
        assert_eq!(check(r, keys, cell_id, asof, job), Ok(outcome.to_string()), "{} {}", r.status, r.text());
        self.outcomes.insert(outcome.to_string());
    }
}

fn path_of(url: &str, host: &str) -> String {
    url.strip_prefix(&format!("https://{host}")).unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_endpoint_and_outcome() {
    let mut b = Node::start(B, |c| {
        c.ttl.current = Duration::from_millis(500);
    })
    .await;
    let b_rate = format!("{B}/rate");
    b.stored(&b_rate, RATE_SIG);
    b.put_value(&b_rate, &args(&[("ccy", s("USD")), ("date", d("2026-03-01"))]), n("1.5"), T1);
    let b_addr = b.base.clone();
    let a = Node::start(A, |c| {
        c.budget = Duration::from_secs(1);
        c.ttl.current = Duration::from_millis(500);
        c.ttl.binding = Duration::from_millis(500);
        c.peers.insert(B.into(), b_addr.clone());
        c.aliases.insert("fx".into(), format!("{A}/rate"));
        c.catalog.insert(format!("{A}/rate"), "Daily reference exchange rates against the euro".into());
        policy(c, ALICE);
    })
    .await;
    seed(&a, 2000);
    let mut keys = a.keys();
    keys.insert_response(B, 200, Some(&key_doc(B, &b.key)));
    let mut seen = Seen { outcomes: BTreeSet::new(), schemas: schemas() };

    // ok: a stored cell (primary) and a computed one (verified).
    let usd = args(&[("ccy", s("USD")), ("date", d("2026-03-01"))]);
    let rate_id = a.cell_id(&format!("{A}/rate"), &usd);
    let r = a.get(&format!("/cell/{A}/rate?ccy=USD&date=2026-03-01")).await;
    assert_eq!(r.status, 200);
    seen.envelope(&r, "ok", &keys, &rate_id, None, None);
    assert_eq!(r.json().get("trust").and_then(Value::as_str), Some("primary"));
    let double_id = a.cell_id(&format!("{A}/double"), &usd);
    let r = a.get(&format!("/cell/{A}/double?ccy=USD&date=2026-03-01&asof={T2}")).await;
    seen.envelope(&r, "ok", &keys, &double_id, Some(T2), None);
    assert_eq!(r.json().get("trust").and_then(Value::as_str), Some("verified"));
    assert_eq!(r.json().get("receipt").and_then(|x| x.get("value")).and_then(Value::as_str), Some("2.16"));
    let double_receipt = r.json().get("receipt").unwrap().to_jcs();

    // not_covered: no value; no function; a fault.
    let eur = args(&[("ccy", s("EUR")), ("date", d("2026-03-01"))]);
    let r = a.get(&format!("/cell/{A}/rate?ccy=EUR&date=2026-03-01")).await;
    assert_eq!(r.status, 404);
    seen.envelope(&r, "not_covered", &keys, &a.cell_id(&format!("{A}/rate"), &eur), None, None);
    let r = a.get(&format!("/cell/{A}/nothing?ccy=EUR")).await;
    assert_eq!(r.status, 404);
    seen.schemas.assert_envelope(&r.body);
    assert_eq!(r.outcome(), "not_covered");
    let r = a.get(&format!("/cell/{A}/broken?ccy=USD&date=2026-03-01")).await;
    seen.envelope(&r, "not_covered", &keys, &a.cell_id(&format!("{A}/broken"), &usd), None, None);
    assert_eq!(r.json().get("fault").and_then(Value::as_str), Some("E_FORMULA_DIV_ZERO"));

    // unlicensed (no key, and a key that is not this mesh's), then ok with alice's key.
    let usd1 = args(&[("ccy", s("USD"))]);
    let secret_id = a.cell_id(&format!("{A}/secret"), &usd1);
    let r = a.get(&format!("/cell/{A}/secret?ccy=USD")).await;
    assert_eq!(r.status, 403);
    seen.envelope(&r, "unlicensed", &keys, &secret_id, None, None);
    assert!(r.header("cache-control").unwrap().contains("private"));
    let r =
        a.get_with(&format!("/cell/{A}/rate?ccy=USD&date=2026-03-01"), &[("authorization", "Bearer dm_nope")]).await;
    seen.envelope(&r, "unlicensed", &keys, &rate_id, None, None);
    let auth = format!("Bearer {ALICE}");
    let r = a.get_with(&format!("/cell/{A}/secret?ccy=USD"), &[("authorization", &auth)]).await;
    seen.envelope(&r, "ok", &keys, &secret_id, None, None);
    assert!(r.header("cache-control").unwrap().contains("private"), "a licensed answer is never shared");

    // payment_required (the stub).
    let r = a.get(&format!("/cell/{A}/paid?ccy=USD")).await;
    assert_eq!(r.status, 402);
    seen.envelope(&r, "payment_required", &keys, &a.cell_id(&format!("{A}/paid"), &usd1), None, None);

    // pending: a function taking 2 s with a budget of 1 s; its job's result.
    let tag = args(&[("tag", s("x"))]);
    let slow_id = a.cell_id(&format!("{A}/slow"), &tag);
    let t0 = std::time::Instant::now();
    let r = a.get(&format!("/cell/{A}/slow?tag=x")).await;
    assert!(t0.elapsed() < Duration::from_millis(1500), "pending within the budget: {:?}", t0.elapsed());
    assert_eq!(r.status, 202);
    seen.envelope(&r, "pending", &keys, &slow_id, None, None);
    let job = r.json().get("job").and_then(Value::as_str).unwrap().to_string();
    let poll = r.json().get("poll").and_then(Value::as_str).unwrap().to_string();
    assert_eq!(poll, format!("https://{A}/jobs/{job}"));
    let mut done = None;
    for _ in 0..60 {
        let r = a.get(&path_of(&poll, A)).await;
        match r.status {
            202 => seen.envelope(&r, "pending", &keys, &slow_id, None, Some(&job)),
            _ => {
                done = Some(r);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let r = done.expect("the job completes");
    seen.envelope(&r, "ok", &keys, &slow_id, None, Some(&job));
    // bad_request E_JOB_UNKNOWN at a job URL.
    let r = a.get(&format!("/jobs/{}", "0".repeat(64))).await;
    assert_eq!(r.status, 400);
    seen.envelope(&r, "bad_request", &keys, &slow_id, None, Some(&"0".repeat(64)));
    assert_eq!(r.json().get("error").and_then(Value::as_str), Some("E_JOB_UNKNOWN"));
    assert_eq!(
        check(&r, &keys, &slow_id, None, None),
        Err("E_ENVELOPE".into()),
        "E_JOB_UNKNOWN answers a job URL only"
    );

    // bad_request: every code parse can raise (SPEC 5.4).
    let types = format!("{A}/types");
    let ok_types = "b=true&d=2026-03-01&i=2026-03-01T00:00:00.000000Z&n=1&s=a&x=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855:0:text/plain";
    let with = |k: &str, v: &str| -> String {
        ok_types
            .split('&')
            .map(|p| if p.starts_with(&format!("{k}=")) { format!("{k}={v}") } else { p.to_string() })
            .collect::<Vec<_>>()
            .join("&")
    };
    let cases: Vec<(&str, String)> = vec![
        ("E_PARSE_URI", format!("{A}/rate?")),
        ("E_PARSE_URI", format!("{A}/rate?ccy")),
        ("E_CANON_NAME", format!("Mesh-A.example/rate?ccy=USD&date=2026-03-01")),
        ("E_CANON_NAME", format!("{A}/Rate?ccy=USD&date=2026-03-01")),
        ("E_CANON_ARG_NAME", format!("{A}/rate?Ccy=USD&date=2026-03-01")),
        ("E_CANON_ARG_DUPLICATE", format!("{A}/rate?ccy=USD&ccy=USD&date=2026-03-01")),
        ("E_CANON_ARG_ORDER", format!("{A}/rate?date=2026-03-01&ccy=USD")),
        ("E_CANON_PERCENT", format!("{A}/rate?ccy=%55SD&date=2026-03-01")),
        ("E_CANON_TYPE", format!("{types}?{}", with("b", "yes"))),
        ("E_CANON_NUMBER", format!("{types}?{}", with("n", "01"))),
        ("E_CANON_DATE", format!("{types}?{}", with("d", "2026-02-30"))),
        ("E_CANON_INSTANT", format!("{types}?{}", with("i", "2026-03-01T00:00:00Z"))),
        ("E_CANON_INSTANT", format!("{A}/rate?ccy=USD&date=2026-03-01&asof=2026-13-01T00:00:00.000000Z")),
        ("E_CANON_STRING", format!("{types}?{}", with("s", "Zu%CC%88rich"))),
        ("E_CANON_BLOB", format!("{types}?{}", with("x", "nothex"))),
        ("E_CANON_LIMIT", format!("{A}/rate?{}=1&ccy=USD&date=2026-03-01", "a".repeat(65))),
        ("E_ARG_UNKNOWN", format!("{A}/rate?ccy=USD&date=2026-03-01&zzz=1")),
        ("E_ARG_MISSING", format!("{A}/rate?ccy=USD")),
    ];
    let mut codes = BTreeSet::new();
    for (code, rest) in &cases {
        let r = a.get(&format!("/cell/{rest}")).await;
        assert_eq!(r.status, 400, "{rest}: {}", r.text());
        seen.envelope(&r, "bad_request", &keys, &rate_id, None, None);
        assert_eq!(r.json().get("error").and_then(Value::as_str), Some(*code), "{rest}");
        codes.insert(code.to_string());
    }
    let all: BTreeSet<String> =
        datumesh_http::envelope::BAD_REQUEST_CODES.iter().map(|c| c.as_str().to_string()).collect();
    assert_eq!(codes, all, "every URI code reached");

    // stale and error: B's cell read through A, then B down.
    let b_usd_id = {
        let fh = bl(|| b.store.resolve_name(&b_rate, &now()).unwrap().unwrap());
        datumesh_core::Cell { ns: B.into(), fn_name: "rate".into(), fn_hash: fh, args: usd.clone() }.id()
    };
    let r = a.get(&format!("/cell/{B}/rate?ccy=USD&date=2026-03-01")).await;
    seen.envelope(&r, "ok", &keys, &b_usd_id, None, None);
    assert_eq!(r.json().get("trust").and_then(Value::as_str), Some("primary"));
    b.stop().await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let r = a.get(&format!("/cell/{B}/rate?ccy=USD&date=2026-03-01")).await;
    assert_eq!(r.status, 200);
    seen.envelope(&r, "stale", &keys, &b_usd_id, None, None);
    let gbp = args(&[("ccy", s("GBP")), ("date", d("2026-03-01"))]);
    let b_gbp_id = {
        let fh = bl(|| a.store.resolve_name(&b_rate, &now()).unwrap().unwrap());
        datumesh_core::Cell { ns: B.into(), fn_name: "rate".into(), fn_hash: fh, args: gbp }.id()
    };
    let r = a.get(&format!("/cell/{B}/rate?ccy=GBP&date=2026-03-01")).await;
    assert_eq!(r.status, 503);
    seen.envelope(&r, "error", &keys, &b_gbp_id, None, None);
    assert_eq!(r.json().get("error").and_then(Value::as_str), Some("E_INPUT_UNAVAILABLE"));

    // rate_limited: a mesh allowing one request a minute.
    let tight = Node::start_with(
        Setup { db: Some(a.db.clone()), ..Setup::new(A) },
        |c| {
            c.rate.per_ip = 1;
            c.rate.ip_burst = 1;
        },
        |b| b,
    )
    .await;
    let _ = tight.get(&format!("/cell/{A}/rate?ccy=USD&date=2026-03-01")).await;
    let r = tight.get(&format!("/cell/{A}/rate?ccy=USD&date=2026-03-01")).await;
    assert_eq!(r.status, 429);
    seen.envelope(&r, "rate_limited", &keys, &rate_id, None, None);
    let retry = r.json().get("retry_after_seconds").and_then(Value::as_number_text).unwrap().to_string();
    assert_eq!(r.header("retry-after"), Some(retry));

    let want: BTreeSet<String> = [
        "ok",
        "stale",
        "not_covered",
        "unlicensed",
        "payment_required",
        "rate_limited",
        "bad_request",
        "error",
        "pending",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(seen.outcomes, want, "every outcome reached");

    // POST /cells: an array of envelopes, in order, each schema-valid.
    let body = format!(
        r#"["{A}/rate?ccy=USD&date=2026-03-01", {{"cell": "cell://{A}/rate?ccy=GBP&date=2026-03-01"}}, "{A}/rate(\"USD\", 2026-03-01)", "{A}/rate?ccy"]"#
    );
    let r = a.post("/cells", body.into_bytes()).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let Value::Array(items) = r.json() else { panic!("not an array") };
    let outcomes: Vec<String> =
        items.iter().map(|e| e.get("outcome").and_then(Value::as_str).unwrap().to_string()).collect();
    assert_eq!(outcomes, ["ok", "ok", "ok", "bad_request"]);
    for e in &items {
        seen.schemas.assert_envelope(&e.to_jcs());
    }
    assert_eq!(items[0].to_jcs(), items[2].to_jcs(), "a call and its URI give the same envelope");
    let r = a.post("/cells", b"{}".to_vec()).await;
    assert_eq!(r.status, 400);
    let r = a.post("/cells", format!("[{}]", vec!["\"x\""; 1001].join(",")).into_bytes()).await;
    assert_eq!(r.status, 400, "bounded batch size");

    // GET /slice (JSON); /manifest and /slice/<id>/<members> for the slice total read.
    let r = a.get(&format!("/slice/{A}/rate?ccy=*&date=2026-03-01")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let page = r.json();
    let Some(Value::Array(members)) = page.get("members") else { panic!("members") };
    assert_eq!(members.len(), 2);
    let r = a.get(&format!("/cell/{A}/total?date=2026-03-01")).await;
    assert_eq!(r.outcome(), "ok", "{}", r.text());
    let total = Receipt::from_json(&r.json().get("receipt").unwrap().to_jcs()).unwrap();
    assert_eq!(total.value().lexical(), "1.93");
    let entry = total.inputs()[0].clone();
    let parts: Vec<&str> = entry.split(':').collect();
    let m1 = a.get(&format!("/manifest/{}/{}", parts[1], parts[2])).await;
    assert_eq!(m1.status, 200);
    assert_eq!(datumesh_core::Manifest::from_json(&m1.body).unwrap().entry(), entry);
    let m2 = a.get(&format!("/slice/{}/{}", parts[1], parts[2])).await;
    assert_eq!(m2.body, m1.body);
    assert!(m1.header("cache-control").unwrap().contains("immutable"));

    // GET /receipt, /descriptor, /blob: bytes, immutable.
    let dr = Receipt::from_json(&double_receipt).unwrap();
    let r = a.get(&format!("/receipt/{}", dr.hash())).await;
    assert_eq!(r.status, 200);
    assert_eq!(Receipt::from_json(&r.body).unwrap().hash(), dr.hash());
    assert!(r.header("cache-control").unwrap().contains("immutable"));
    let r = a.get(&format!("/descriptor/{}", dr.cell().fn_hash)).await;
    assert_eq!(datumesh_core::FunctionDescriptor::from_json(&r.body).unwrap().hash(), dr.cell().fn_hash);
    let blob = bl(|| a.store.put_blob(b"hello blob").unwrap());
    let r = a.get(&format!("/blob/{blob}")).await;
    assert_eq!(r.body, b"hello blob");
    assert_eq!(a.get(&format!("/receipt/{}", "f".repeat(64))).await.status, 404);

    // GET /explain: nested envelopes, each schema-valid.
    let r = a.get(&format!("/explain/{A}/double?ccy=USD&date=2026-03-01&asof={T2}&depth=3")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let x = r.json();
    seen.schemas.assert_envelope(&x.get("envelope").unwrap().to_jcs());
    let Some(Value::Array(ins)) = x.get("inputs") else { panic!("inputs") };
    assert_eq!(ins.len(), 1);
    assert!(ins[0].get("entry").and_then(Value::as_str).is_some());
    seen.schemas.assert_envelope(&ins[0].get("envelope").unwrap().to_jcs());
    let r = a.get(&format!("/explain/{A}/total?date=2026-03-01")).await;
    let x = r.json();
    let slice_node =
        &x.get("inputs").and_then(|i| if let Value::Array(a) = i { a.first().cloned() } else { None }).unwrap();
    assert_eq!(slice_node.get("members").and_then(Value::as_number_text), Some("2"));

    // GET /find (the stub).
    let r = a.get("/find?q=exchange%20euro").await;
    let res = r.json();
    let Some(Value::Array(found)) = res.get("results") else { panic!("results") };
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].get("name").and_then(Value::as_str), Some(format!("{A}/rate").as_str()));

    // POST /verify.
    let r = a.post("/verify", double_receipt.clone()).await;
    assert_eq!(r.json().get("level").and_then(Value::as_str), Some("recomputed"), "{}", r.text());

    // Key document, mesh document (schema and resolve_mesh), bindings.
    let kd = a.get("/.well-known/datumesh.json").await;
    keys_from_response(A, 200, Some(&kd.body)).unwrap();
    let md = a.get("/.well-known/datumesh-mesh.json").await;
    let v: serde_json::Value = serde_json::from_slice(&md.body).unwrap();
    assert!(seen.schemas.mesh.is_valid(&v), "{}", md.text());
    let resolved = resolve_mesh(
        A,
        Retrieval::Response { status: 200, body: Some(&kd.body) },
        Retrieval::Response { status: 200, body: Some(&md.body) },
    )
    .unwrap();
    assert_eq!(resolved.mesh.bindings, format!("https://{A}/bindings"));
    let r = a.get(&format!("/bindings?name={A}%2Frate&name={A}%2Fnothing&asof={T1}")).await;
    let bv = r.json();
    let bs = bv.get("bindings").unwrap().as_object().unwrap();
    assert_eq!(bs.len(), 1);
    assert_eq!(bs[0].1.get("since").and_then(Value::as_str), Some(BIND_TX));

    // Health, metrics, OpenAPI, Accept.
    assert_eq!(a.get("/health").await.status, 200);
    let m = a.get("/metrics").await.text();
    assert_eq!(metric(&m, "datumesh_pool_size"), 32.0);
    let oa = a.get("/openapi.json").await.json();
    let paths: BTreeSet<String> =
        oa.get("paths").unwrap().as_object().unwrap().iter().map(|(k, _)| k.clone()).collect();
    for p in [
        "/cell/{cell}",
        "/cells",
        "/slice/{cell}",
        "/receipt/{hash}",
        "/descriptor/{hash}",
        "/manifest/{slice_id}/{members_hash}",
        "/blob/{sha256}",
        "/explain/{cell}",
        "/find",
        "/verify",
        "/jobs/{job}",
        "/.well-known/datumesh.json",
        "/.well-known/datumesh-mesh.json",
        "/bindings",
        "/health",
        "/metrics",
        "/openapi.json",
    ] {
        assert!(paths.contains(p), "OpenAPI lacks {p}");
    }
    let r = a.get_with(&format!("/cell/{A}/rate?ccy=USD&date=2026-03-01"), &[("accept", "text/html")]).await;
    assert_eq!(r.status, 406);
    let r = a.get_with(&format!("/cell/{A}/rate?ccy=USD&date=2026-03-01"), &[("accept", "text/html,*/*;q=0.8")]).await;
    assert_eq!(r.status, 200);
    assert!(r.header("x-request-id").is_some());
}
