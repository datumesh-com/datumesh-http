//! Test 3 of the brief: request binding. The adversarial set of the spec's cross-check
//! generator (`crosscheck/generate_envelopes.py`: a receipt for the requested cell or for a
//! sibling differing in one argument, in its function hash, its namespace or its function;
//! a requested asof none, equal, a microsecond or a year earlier or later than the
//! receipt's), plus the `pending` and job cases of `generate_041.py`, built from responses
//! of the real endpoint: each case is what a mesh answering the wrong request would send.
//! The Rust core checks every case here; the Python and JavaScript libraries check the
//! same file with `tools/client_check.sh`. Every mismatch must be `E_ENVELOPE`; none passes.

mod common;

use std::time::Duration;

use common::*;
use datumesh_core::json::Value;

const A: &str = "mesh-a.example";
const SIG: &str = r#"{"args":{"base":{"type":"string","valid":false},"date":{"type":"date","valid":true},"quote":{"type":"string","valid":false}},"order":["base","quote","date"],"returns":{"type":"number","unit":{"of":{"arg":"quote"},"per":{"arg":"base"}}}}"#;
const SIG2: &str = r#"{"args":{"base":{"type":"string","valid":false},"date":{"type":"date","valid":true},"quote":{"type":"string","valid":false}},"order":["quote","base","date"],"returns":{"type":"number","unit":{"of":{"arg":"quote"},"per":{"arg":"base"}}}}"#;
const ASOFS: [&str; 3] = ["2026-03-16T09:00:00.000000Z", "2026-03-16T09:00:00.000001Z", "2026-06-30T23:59:59.999999Z"];
/// When fx_rate is bound again (to another function hash: the "other-fn-hash" sibling).
const REBIND: &str = "2026-07-01T00:00:00.000000Z";

/// A small deterministic generator (the cases are reproducible).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn f(&mut self) -> f64 {
        (self.next() % 1_000_000) as f64 / 1_000_000.0
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() % xs.len() as u64) as usize]
    }
}

struct Sibling {
    label: &'static str,
    name: String,
    args: datumesh_core::Args,
    /// The receipts' tx offset: the other-fn-hash cell's versions come after the rebinding.
    after_rebind: bool,
}

fn year(t: &str, dy: i32) -> String {
    let y: i32 = t[..4].parse().unwrap();
    format!("{}{}", y + dy, &t[4..])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adversarial_request_binding() {
    let a = Node::start(A, |c| {
        c.budget = Duration::from_millis(500);
    })
    .await;
    let fx = format!("{A}/fx_rate");
    a.stored(&fx, SIG);
    a.stored(&format!("{A}/fx_close"), SIG);
    a.stored(&format!("{A}/archive/fx_rate"), SIG);
    a.function(&format!("{A}/slow"), "sleep", "1500", &[], TAG_SIG);
    let cell = |b: &str, q: &str| args(&[("base", s(b)), ("date", d("2026-03-15")), ("quote", s(q))]);
    let mut sibs = vec![
        Sibling { label: "same", name: fx.clone(), args: cell("EUR", "USD"), after_rebind: false },
        Sibling { label: "other-quote", name: fx.clone(), args: cell("EUR", "GBP"), after_rebind: false },
        Sibling { label: "other-base", name: fx.clone(), args: cell("CHF", "USD"), after_rebind: false },
        Sibling {
            label: "other-namespace",
            name: format!("{A}/archive/fx_rate"),
            args: cell("EUR", "USD"),
            after_rebind: false,
        },
        Sibling {
            label: "other-function",
            name: format!("{A}/fx_close"),
            args: cell("EUR", "USD"),
            after_rebind: false,
        },
    ];
    for sib in &sibs {
        for (i, t) in ASOFS.iter().enumerate() {
            a.put_value(&sib.name, &sib.args, n(&format!("1.08{}", i + 1)), t);
        }
    }
    // The same cell under another function hash: fx_rate bound again after the asofs.
    let h2 = bl(|| {
        a.store
            .put_descriptor(
                format!(r#"{{"code":{{"body":"","lang":"stored"}},"inputs":[],"signature":{SIG2}}}"#).as_bytes(),
            )
            .unwrap()
    });
    bl(|| a.store.bind_name(&fx, &h2, REBIND).unwrap());
    sibs.push(Sibling { label: "other-fn-hash", name: fx.clone(), args: cell("EUR", "USD"), after_rebind: true });
    let shifted = |t: &str| t.replacen("2026-03-16", "2026-07-16", 1).replacen("2026-06-30", "2026-08-30", 1);
    for (i, t) in ASOFS.iter().enumerate() {
        a.put_value(&fx, &sibs[5].args, n(&format!("2.08{}", i + 1)), &shifted(t));
    }
    // Each sibling's cell id, under the binding it was answered with.
    let id_of = |sib: &Sibling| {
        let at = if sib.after_rebind { "2026-09-01T00:00:00.000000Z" } else { "2026-04-01T00:00:00.000000Z" };
        let fh = bl(|| a.store.resolve_name(&sib.name, at).unwrap().unwrap());
        let (ns, f) = sib.name.rsplit_once('/').unwrap();
        datumesh_core::Cell { ns: ns.into(), fn_name: f.into(), fn_hash: fh, args: sib.args.clone() }.id()
    };
    let ids: Vec<String> = sibs.iter().map(id_of).collect();
    let uri_of = |sib: &Sibling| {
        let (ns, f) = sib.name.rsplit_once('/').unwrap();
        let r = datumesh_core::CellRef { ns: ns.into(), fn_name: f.into(), args: sib.args.clone(), asof: None };
        datumesh_core::format_uri(&r)["cell://".len()..].to_string()
    };
    let keys = a.keys();
    let keys_env = Value::Object(vec![(
        A.to_string(),
        Value::Object(vec![(
            "https".into(),
            Value::Object(vec![
                ("body".into(), Value::String(String::from_utf8(key_doc(A, &a.key)).unwrap())),
                ("status".into(), datumesh_core::json::parse(b"200").unwrap()),
            ]),
        )]),
    )]);

    let mut lines = Vec::new();
    let mut emit = |id: String, r: &Resp, cell_id: &str, asof: Option<&str>, job: Option<&str>, expect: &str| {
        let got = match check(r, &keys, cell_id, asof, job) {
            Ok(_) => "ok".to_string(),
            Err(c) => c,
        };
        assert_eq!(got, expect, "{id} (Rust core): {}", r.text());
        let mut req = vec![("cell_id".to_string(), Value::String(cell_id.into()))];
        if let Some(t) = asof {
            req.push(("asof".into(), Value::String(t.into())));
        }
        if let Some(j) = job {
            req.push(("job".into(), Value::String(j.into())));
        }
        let line = Value::Object(vec![
            ("expect".into(), Value::String(expect.into())),
            ("id".into(), Value::String(id)),
            ("input".into(), Value::String(r.text())),
            ("keys".into(), keys_env.clone()),
            ("kind".into(), Value::String("envelope".into())),
            ("request".into(), Value::Object(req)),
            ("status".into(), datumesh_core::json::parse(r.status.to_string().as_bytes()).unwrap()),
        ]);
        lines.push(String::from_utf8(line.to_jcs()).unwrap());
    };

    // The generator's cases, served by the real endpoint.
    let mut rng = Rng(0x9e3779b97f4a7c15);
    let n_cases = 600;
    let (mut matched, mut refused) = (0, 0);
    let mut cache = std::collections::HashMap::new();
    for i in 0..n_cases {
        let answered = if rng.f() < 0.6 { 0 } else { (rng.next() % sibs.len() as u64) as usize };
        let k = (rng.next() % 3) as usize;
        let z = if sibs[answered].after_rebind { shifted(ASOFS[k]) } else { ASOFS[k].to_string() };
        let requested = if rng.f() < 0.7 { 0 } else { (rng.next() % sibs.len() as u64) as usize };
        let near = [
            ("none", None),
            ("same", Some(z.clone())),
            ("minus-1us", Some(plus(&z, -1))),
            ("plus-1us", Some(plus(&z, 1))),
            ("year-earlier", Some(year(&z, -1))),
            ("year-later", Some(year(&z, 1))),
        ];
        let (label, req_asof) = rng.pick(&near).clone();
        let key = (answered, k);
        if let std::collections::hash_map::Entry::Vacant(e) = cache.entry(key) {
            let r = a.get(&format!("/cell/{}&asof={z}", uri_of(&sibs[answered]))).await;
            assert_eq!(r.outcome(), "ok", "{}", r.text());
            e.insert(r);
        }
        let r = &cache[&key];
        let valid = requested == answered && req_asof.as_ref().is_none_or(|t| z.as_str() <= t.as_str());
        if valid {
            matched += 1;
        } else {
            refused += 1;
        }
        emit(
            format!("binding/{i:04}-{}-for-{}-{label}", sibs[answered].label, sibs[requested].label),
            r,
            &ids[requested],
            req_asof.as_deref(),
            None,
            if valid { "ok" } else { "E_ENVELOPE" },
        );
    }

    // Jobs: a pending envelope and the job's result, against the original request and others.
    let tag = args(&[("tag", s("j"))]);
    let slow_id = a.cell_id(&format!("{A}/slow"), &tag);
    let p = a.get(&format!("/cell/{A}/slow?tag=j")).await;
    assert_eq!(p.status, 202, "{}", p.text());
    let job = p.json().get("job").and_then(Value::as_str).unwrap().to_string();
    emit("job/pending-own".into(), &p, &slow_id, None, None, "ok");
    emit("job/pending-other-cell".into(), &p, &ids[0], None, None, "E_ENVELOPE");
    emit("job/pending-asof-not-requested".into(), &p, &slow_id, Some(ASOFS[0]), None, "E_ENVELOPE");
    let poll = format!("/jobs/{job}");
    let running = a.get(&poll).await;
    assert_eq!(running.status, 202);
    emit("job/running-own".into(), &running, &slow_id, None, Some(&job), "ok");
    emit("job/running-other-job".into(), &running, &slow_id, None, Some(&"a".repeat(64)), "E_ENVELOPE");
    let done = loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let r = a.get(&poll).await;
        if r.status != 202 {
            break r;
        }
    };
    emit("job/result-original-request".into(), &done, &slow_id, None, Some(&job), "ok");
    emit("job/result-other-cell".into(), &done, &ids[0], None, Some(&job), "E_ENVELOPE");
    let unknown = a.get(&format!("/jobs/{}", "b".repeat(64))).await;
    emit("job/unknown-at-job-url".into(), &unknown, &slow_id, None, Some(&"b".repeat(64)), "ok");
    emit("job/unknown-at-cell-url".into(), &unknown, &slow_id, None, None, "E_ENVELOPE");

    let path = out_dir().join("binding-cases.jsonl");
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    let summary = format!(
        "request binding: {} cases: {n_cases} of the generator ({matched} matching, {refused} mismatching: E_ENVELOPE) and {} job cases; Rust core 0.3.5 gives every expected result; {}\n",
        lines.len(),
        lines.len() - n_cases,
        path.display()
    );
    std::fs::write(out_dir().join("binding.txt"), &summary).unwrap();
    eprint!("{summary}");
    assert!(refused > 100 && matched > 100);
}
