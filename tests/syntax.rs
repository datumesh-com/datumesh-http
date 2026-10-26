//! Test 2 of the brief: the call syntax and the cell URI of the same cell give
//! byte-identical envelopes; short names never appear in a response; and the spec's URI
//! vectors resolve as SPEC 5.4 says (URI, HTTPS mapping, every code).

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use datumesh_core::json::{self, Value};
use datumesh_core::{ErrorCode, FunctionDescriptor};
use datumesh_http::call::{self, Bindings, Bound, Rejection};

const A: &str = "mesh-a.example";

/// Percent-encodes a call for a URL path.
fn enc(call: &str) -> String {
    let mut out = String::new();
    for b in call.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:@*".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn call_and_uri_are_one_request() {
    let a = Node::start(A, |c| {
        c.aliases.insert("fx".into(), format!("{A}/rate"));
        c.aliases.insert("twice".into(), format!("{A}/double"));
    })
    .await;
    seed(&a, 10);
    let groups: Vec<(String, Vec<String>)> = vec![
        (
            format!("{A}/rate?ccy=USD&date=2026-03-01"),
            vec![
                enc(&format!("{A}/rate(\"USD\", 2026-03-01)")),
                enc(&format!("{A}/rate(ccy: \"USD\", date: 2026-03-01)")),
                enc(&format!("{A}/rate(date: 2026-03-01, ccy: \"USD\")")),
                enc(&format!("{A}/rate(\"USD\", date: 2026-03-01)")),
                enc("@fx(\"USD\", 2026-03-01)"),
                enc(&format!(" {A}/rate( \"USD\" ,2026-03-01 )")),
            ],
        ),
        (
            format!("{A}/rate?ccy=USD&date=2026-03-01&asof={T2}"),
            vec![enc(&format!("{A}/rate(\"USD\", 2026-03-01)")) + &format!("?asof={T2}"), enc("@fx(ccy: \"USD\", date: 2026-03-01)") + &format!("?asof={T2}")],
        ),
        (
            format!("{A}/double?ccy=GBP&date=2026-03-01&asof={T2}"),
            vec![enc("@twice(\"GBP\", 2026-03-01)") + &format!("?asof={T2}"), enc(&format!("{A}/double(\"GBP\", 2026-03-01)")) + &format!("?asof={T2}")],
        ),
        (
            format!("{A}/broken?ccy=USD&date=2026-03-01"),
            vec![enc(&format!("{A}/broken(\"USD\", 2026-03-01)"))],
        ),
        (
            format!("{A}/rate?ccy=CHF&date=2026-03-01"),
            vec![enc("@fx(\"CHF\", 2026-03-01)")],
        ),
        (
            // A number with a sign, a boolean, an instant and a blob in the call syntax.
            format!("{A}/types?b=false&d=2026-03-01&i=2026-03-01T00:00:00.000000Z&n=-1.5&s=a%20b&x=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855:0:text%2Fplain"),
            vec![enc(&format!("{A}/types(false, 2026-03-01, 2026-03-01T00:00:00.000000Z, -1.5, \"a b\", \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855:0:text/plain\")"))],
        ),
    ];
    for (uri, calls) in &groups {
        let base = a.get(&format!("/cell/{uri}")).await;
        assert!(!base.body.is_empty());
        for c in calls {
            let r = a.get(&format!("/cell/{c}")).await;
            assert_eq!(r.status, base.status, "{c}: {}", r.text());
            assert_eq!(r.body, base.body, "{c} and {uri} differ:\n{}\n{}", r.text(), base.text());
            // The canonical link names the asof answered for (now, at each request).
            let strip = |l: Option<String>| l.map(|l| l.split("&asof=").next().unwrap_or("").to_string());
            assert_eq!(strip(r.header("link")), strip(base.header("link")));
            assert_eq!(r.header("cache-control"), base.header("cache-control"));
            let text = r.text();
            assert!(
                !text.contains('@') && !text.contains("\"fx\"") && !text.contains("twice"),
                "a short name in {text}"
            );
            if let Some(l) = r.header("link") {
                assert!(
                    l.starts_with(&format!("<https://{A}/cell/{A}/")),
                    "the canonical link names the full name: {l}"
                );
            }
        }
    }
    // The batch takes calls too, and answers them as /cell does.
    let body = format!(
        "[{}]",
        groups
            .iter()
            .flat_map(|(u, cs)| std::iter::once(u.clone()).chain(cs.iter().map(|c| percent_decode(c))))
            .map(|s| Value::String(s).to_jcs())
            .map(|b| String::from_utf8(b).unwrap())
            .collect::<Vec<_>>()
            .join(",")
    );
    let r = a.post("/cells", body.into_bytes()).await;
    let Value::Array(items) = r.json() else { panic!("{}", r.text()) };
    let mut i = 0;
    for (_, cs) in &groups {
        let first = items[i].to_jcs();
        for k in 0..cs.len() {
            assert_eq!(items[i + 1 + k].to_jcs(), first);
        }
        i += 1 + cs.len();
    }

    // Calls that are not accepted in 0.1.0, and what they give.
    for (c, code) in [
        (enc(&format!("{A}/rate(USD, 2026-03-01)")), "E_PARSE_URI"), // a key literal (15.15)
        (enc(&format!("{A}/rate(\"USD\", 2026-03-01).spot(1)")), "E_PARSE_URI"), // a method
        (enc(&format!("{A}/rate(\"USD\", 2026-03-01 + 1)")), "E_PARSE_URI"), // an expression
        (enc(&format!("{A}/rate(\"USD\", 2026-03-01, 3)")), "E_PARSE_URI"),
        (enc(&format!("{A}/rate(\"USD\")")), "E_ARG_MISSING"),
        (enc(&format!("{A}/rate(\"USD\", date: 2026-03-01, zzz: 1)")), "E_ARG_UNKNOWN"),
        (enc(&format!("{A}/rate(1, 2026-03-01)")), "E_CANON_TYPE"),
        (enc(&format!("{A}/rate(\"Zu\u{308}rich\", 2026-03-01)")), "E_CANON_STRING"),
        (enc(&format!("{A}/rate(\"USD\", *)")), "E_PARSE_URI"),
        (enc(&format!("{A}/rate(\"USD\", 2026-03-01)")) + "?asof=yesterday", "E_CANON_INSTANT"),
        (enc(&format!("{A}/rate(\"USD\", 2026-03-01)")) + "?x=1", "E_PARSE_URI"),
    ] {
        let r = a.get(&format!("/cell/{c}")).await;
        assert_eq!(r.status, 400, "{c}: {}", r.text());
        assert_eq!(r.json().get("error").and_then(Value::as_str), Some(code), "{c}");
    }
    let r = a.get(&format!("/cell/{}", enc("@nope(\"USD\")"))).await;
    assert_eq!(r.outcome(), "not_covered", "an unknown alias names no function");
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap(), 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

/// A function bound to every name, with one signature.
struct One(Option<Arc<FunctionDescriptor>>);

impl Bindings for One {
    fn binding(&self, _: &str, _: &str) -> Result<Option<Arc<Bound>>, ErrorCode> {
        Ok(self.0.clone().map(|d| Arc::new(Bound::new(d.hash(), BIND_TX, d))))
    }
}

#[test]
fn spec_uri_vectors() {
    let mut files = Vec::new();
    for dir in ["uri", "invalid"] {
        for e in std::fs::read_dir(spec_dir().join("vectors").join(dir)).unwrap() {
            let p = e.unwrap().path();
            if p.file_name().unwrap().to_string_lossy().starts_with("uri") || dir == "uri" {
                files.push(p);
            }
        }
    }
    files.sort();
    let (mut valid, mut invalid, mut skipped) = (0, 0, Vec::new());
    for p in files {
        let v = json::parse(&std::fs::read(&p).unwrap()).unwrap();
        if v.get("kind").and_then(Value::as_str) != Some("uri") {
            continue;
        }
        let id = v.get("id").and_then(Value::as_str).unwrap().to_string();
        let input = v.get("input").and_then(Value::as_str).unwrap();
        let sig = v.get("signature").cloned().unwrap_or(Value::Null);
        let d = format!(
            r#"{{"code":{{"body":"","lang":"stored"}},"inputs":[],"signature":{}}}"#,
            String::from_utf8(sig.to_jcs()).unwrap()
        );
        let Ok(desc) = FunctionDescriptor::from_json(d.as_bytes()) else {
            skipped.push(format!("{id} (the signature is not a descriptor's)"));
            continue;
        };
        let Some(rest) = input.strip_prefix("cell://") else {
            skipped.push(format!("{id} (not cell://: no /cell request carries it)"));
            continue;
        };
        let slice = rest.contains("=*");
        let r = call::resolve_uri(rest, "2026-01-01T00:00:00.000000Z", &One(Some(Arc::new(desc))), slice);
        if v.get("valid") == Some(&Value::Bool(true)) {
            let t = r.unwrap_or_else(|e| panic!("{id}: {e:?}"));
            assert_eq!(t.uri, input, "{id}");
            let https = v.get("https").and_then(Value::as_str).unwrap();
            assert_eq!(format!("https://mesh.example/cell/{}", t.rest()), https, "{id}");
            assert_eq!(
                datumesh_core::sha256_hex(&t.reference.canonical()),
                v.get("sha256").and_then(Value::as_str).unwrap(),
                "{id}"
            );
            valid += 1;
        } else {
            let want = ErrorCode::from_code(v.get("error").and_then(Value::as_str).unwrap()).unwrap();
            match r {
                Err(Rejection::BadRequest(c)) => assert_eq!(c, want, "{id}"),
                other => panic!("{id}: {other:?}, want {want:?}"),
            }
            invalid += 1;
        }
    }
    let out =
        format!("uri vectors: {valid} valid, {invalid} invalid resolved; skipped {}: {skipped:?}\n", skipped.len());
    std::fs::write(out_dir().join("uri-vectors.txt"), &out).unwrap();
    eprint!("{out}");
    assert!(valid >= 10 && invalid >= 25);
    let _ = BTreeMap::<String, String>::new();
}
