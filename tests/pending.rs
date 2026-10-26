//! Test 7 of the brief: an evaluation of 10 s answers `pending` within 1 s; the job
//! completes; its result is bound to the original request; then the envelope is served from
//! the memo, within the budget.

mod common;

use std::time::{Duration, Instant};

use common::*;
use datumesh_core::json::Value;

const A: &str = "mesh-a.example";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ten_second_evaluation() {
    let a = Node::start(A, |c| c.budget = Duration::from_secs(1)).await;
    a.function(&format!("{A}/slow"), "sleep", "10000", &[], TAG_SIG);
    let keys = a.keys();
    let tag = args(&[("tag", s("ten"))]);
    let id = a.cell_id(&format!("{A}/slow"), &tag);
    let schemas = schemas();

    let t0 = Instant::now();
    let r = a.get(&format!("/cell/{A}/slow?tag=ten")).await;
    let first = t0.elapsed();
    assert_eq!(r.status, 202, "{}", r.text());
    assert!(first < Duration::from_millis(1200), "pending after {first:?}");
    schemas.assert_envelope(&r.body);
    assert_eq!(check(&r, &keys, &id, None, None), Ok("pending".into()));
    let env = r.json();
    let job = env.get("job").and_then(Value::as_str).unwrap().to_string();
    assert_eq!(job.len(), 64, "256 bits from the CSPRNG");
    assert!(env.get("estimate_seconds").is_some());
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));

    // A second request for the same cell meanwhile: pending too, another job, within 1 s.
    let t1 = Instant::now();
    let r2 = a.get(&format!("/cell/{A}/slow?tag=ten")).await;
    assert_eq!(r2.status, 202);
    assert!(t1.elapsed() < Duration::from_millis(1200));
    let job2 = r2.json().get("job").and_then(Value::as_str).unwrap().to_string();
    assert_ne!(job, job2);

    let mut polls = 0;
    let done = loop {
        let p = a.get(&format!("/jobs/{job}")).await;
        polls += 1;
        if p.status != 202 {
            break p;
        }
        assert!(p.json().get("job").and_then(Value::as_str) == Some(job.as_str()));
        assert_eq!(check(&p, &keys, &id, None, Some(&job)), Ok("pending".into()));
        assert!(t0.elapsed() < Duration::from_secs(20), "the job never completes");
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let completed = t0.elapsed();
    assert_eq!(done.status, 200, "{}", done.text());
    schemas.assert_envelope(&done.body);
    assert_eq!(check(&done, &keys, &id, None, Some(&job)), Ok("ok".into()));
    assert!(completed >= Duration::from_secs(10), "{completed:?}");
    // The result is kept: served again, the same bytes.
    let again = a.get(&format!("/jobs/{job}")).await;
    assert_eq!(again.body, done.body);
    let other = loop {
        let p = a.get(&format!("/jobs/{job2}")).await;
        if p.status != 202 {
            break p;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(other.json().get("receipt"), done.json().get("receipt"), "one evaluation for both");

    // Now from the memo, at once.
    let t2 = Instant::now();
    let r = a.get(&format!("/cell/{A}/slow?tag=ten")).await;
    let memo = t2.elapsed();
    assert_eq!(r.status, 200);
    assert!(memo < Duration::from_millis(200), "{memo:?}");
    assert_eq!(r.json().get("receipt"), done.json().get("receipt"));

    // Unknown and malformed job ids.
    for j in ["0".repeat(64), "z".repeat(40), "a".repeat(10)] {
        let p = a.get(&format!("/jobs/{j}")).await;
        assert_eq!(p.status, 400);
        assert_eq!(p.json().get("error").and_then(Value::as_str), Some("E_JOB_UNKNOWN"));
    }
    let out = format!(
        "pending: 202 after {first:?}; completed after {completed:?} ({polls} polls); then from the memo in {memo:?}\n"
    );
    std::fs::write(out_dir().join("pending.txt"), &out).unwrap();
    eprint!("{out}");
}
