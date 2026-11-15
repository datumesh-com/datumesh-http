//! Rule A (`CONTRACT-1.8b.md` H1): no binding or declaration start earlier than the mesh clock.
//! SQLite store; the clock is fixed at 2026-11-15T12:00:00.000000Z.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::lite::*;
use common::*;
use datumesh_core::json::{self, Value};
use datumesh_core::{FunctionDescriptor, KeyDocument};
use datumesh_http::keydoc::declare_at;
use datumesh_http::publish::{publish_stored_with, PublishError, PublishErrorKind, PublishOptions, Published};
use datumesh_store::Store;

const NS: &str = "mesh-test.datumesh.com/scratch";
const LEVEL: &str = "mesh-test.datumesh.com/scratch/level";
const PAST: &str = "2026-11-15T11:59:59.999999Z";
const FUTURE: &str = "2026-11-15T12:00:00.000001Z";
const LATER: &str = "2026-11-16T00:00:00.000000Z";
const LICENCE: &str = "https://mesh-test.datumesh.com/licences/restricted-test";

const SIG: &str =
    r#"{"args":{"day":{"type":"date","valid":true}},"order":["day"],"returns":{"type":"number","unit":"1"}}"#;

fn opts(now: &'static str) -> PublishOptions {
    PublishOptions { clock: Arc::new(FixedClock(now)), backfill: None }
}

fn backfill(now: &'static str, log: &Path) -> PublishOptions {
    PublishOptions { clock: Arc::new(FixedClock(now)), backfill: Some(log.to_path_buf()) }
}

/// A stored function line; `bound` absent when `None`.
fn function_line(name: &str, bound: Option<&str>) -> String {
    let mut members = vec![("function".to_string(), Value::String(name.into())), ("signature".to_string(), sig())];
    if let Some(b) = bound {
        members.push(("bound".to_string(), Value::String(b.into())));
    }
    String::from_utf8(Value::Object(members).to_jcs()).unwrap()
}

fn sig() -> Value {
    json::parse(SIG.as_bytes()).unwrap()
}

fn write(lite: &Lite, name: &str, lines: &[String]) -> PathBuf {
    let p = lite.path(name);
    std::fs::write(&p, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    p
}

fn run(lite: &Lite, path: &Path, o: &PublishOptions) -> Result<Published, PublishError> {
    let owned = vec![HOST.to_string()];
    publish_stored_with(&*lite.store, &lite.key, HOST, &owned, path, o)
}

fn unbound(lite: &Lite, name: &str) -> bool {
    lite.store.resolve_name(name, END).unwrap().is_none()
}

/// 1. A bound earlier than the clock is refused with `StartBeforeNow`, and nothing is bound.
#[test]
fn a_start_before_now_is_refused() {
    let lite = Lite::new("rule-a-1");
    let f = write(&lite, "f.jsonl", &[function_line(LEVEL, Some(PAST))]);
    let e = run(&lite, &f, &opts(NOW)).unwrap_err();
    assert_eq!(e.line, 1);
    match &e.kind {
        PublishErrorKind::StartBeforeNow { name, start, now } => {
            assert_eq!((name.as_str(), start.as_str(), now.as_str()), (LEVEL, PAST, NOW));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        e.kind.to_string(),
        format!("binding start {PAST} of {LEVEL} is earlier than the mesh clock {NOW} (rule A)")
    );
    assert!(unbound(&lite, LEVEL));
}

/// 2. Without `bound` the name binds at the clock's now.
#[test]
fn a_missing_start_is_now() {
    let lite = Lite::new("rule-a-2");
    let f = write(&lite, "f.jsonl", &[function_line(LEVEL, None)]);
    let p = run(&lite, &f, &opts(NOW)).unwrap();
    assert_eq!(p.functions, 1);
    assert_eq!(lite.bound_tx(LEVEL).as_deref(), Some(NOW));
}

/// 3 and 4. A start later than now is allowed and binds at that start; running the same file
/// again when the clock is later writes nothing and is not refused.
#[test]
fn a_future_start_binds_and_a_rerun_is_not_refused() {
    let lite = Lite::new("rule-a-3");
    let f = write(&lite, "f.jsonl", &[function_line(LEVEL, Some(FUTURE))]);
    let p = run(&lite, &f, &opts(NOW)).unwrap();
    assert_eq!(p.functions, 1);
    assert_eq!(lite.bound_tx(LEVEL).as_deref(), Some(FUTURE));

    let again = run(&lite, &f, &opts(LATER)).unwrap();
    assert_eq!(again.functions, 0, "{again:?}");
    assert_eq!(lite.bound_tx(LEVEL).as_deref(), Some(FUTURE));
}

/// 5. All or nothing: a valid function line (no `bound`), then a class line whose `bound` is in the
///    past: `StartBeforeNow`, and the function's name is unbound too.
#[test]
fn nothing_is_written_unless_every_check_passes() {
    use common::world::{class_json, fx_rate_sig, NONE_KEYS_PAIR};
    let lite = Lite::new("rule-a-5");
    // The class's method function is in the store already, bound well before the class's start.
    let rate = format!("{NS}/fx_pair/rate");
    let d = format!(r#"{{"code":{{"body":"","lang":"stored"}},"inputs":[],"signature":{}}}"#, fx_rate_sig());
    let canonical = FunctionDescriptor::from_json(d.as_bytes()).unwrap().canonical();
    let h = lite.store.put_descriptor(&canonical).unwrap();
    lite.store.bind_name(&rate, &h, K_FROM).unwrap();
    let class = format!("{NS}/fx_pair");
    let descriptor = class_json(NONE_KEYS_PAIR, &["base", "quote"], &[("rate", &h)], HOST, "");
    let class_line = String::from_utf8(
        Value::Object(vec![
            ("bound".to_string(), Value::String("2026-11-15T11:00:00.000000Z".into())),
            ("class".to_string(), Value::String(class.clone())),
            ("descriptor".to_string(), json::parse(descriptor.as_bytes()).unwrap()),
        ])
        .to_jcs(),
    )
    .unwrap();
    let f = write(&lite, "f.jsonl", &[function_line(LEVEL, None), class_line]);
    let e = run(&lite, &f, &opts(NOW)).unwrap_err();
    assert_eq!(e.line, 2);
    assert!(matches!(e.kind, PublishErrorKind::StartBeforeNow { .. }), "{e}");
    assert!(unbound(&lite, LEVEL), "the function line's name is unbound too");
    assert!(lite.store.resolve_class(&class, END).unwrap().is_none());
}

const BACKFILL_START: &str = "2026-09-01T00:00:00.000000Z";

/// 6. Backfill accepted on a fresh store: one log line, mode 0600.
fn backfilled() -> (Lite, PathBuf) {
    let lite = Lite::new("rule-a-6");
    let log = lite.path("backfill.log");
    let f = write(&lite, "f.jsonl", &[function_line(LEVEL, Some(BACKFILL_START))]);
    let p = run(&lite, &f, &backfill(NOW, &log)).unwrap();
    assert_eq!(p.functions, 1);
    assert_eq!(lite.bound_tx(LEVEL).as_deref(), Some(BACKFILL_START));
    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(text.lines().count(), 1, "{text}");
    let v = json::parse(text.lines().next().unwrap().as_bytes()).unwrap();
    let strings = |k: &str| -> Vec<String> {
        match v.get(k) {
            Some(Value::Array(a)) => a.iter().map(|x| x.as_str().unwrap().to_string()).collect(),
            other => panic!("{k}: {other:?}"),
        }
    };
    assert_eq!(strings("namespaces"), vec![NS.to_string()]);
    assert_eq!(strings("starts"), vec![BACKFILL_START.to_string()]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&log).unwrap().permissions().mode() & 0o777, 0o600);
    }
    (lite, log)
}

#[test]
fn backfill_is_accepted_on_a_fresh_store() {
    let _ = backfilled();
}

/// 7. After test 6, a second file under the same namespace binding a new name, with backfill, is
///    refused with `Backfill`; the log still has one line.
#[test]
fn backfill_is_refused_under_a_namespace_that_has_bindings() {
    let (lite, log) = backfilled();
    let other = format!("{NS}/other");
    let f = write(&lite, "g.jsonl", &[function_line(&other, Some("2026-09-02T00:00:00.000000Z"))]);
    let e = run(&lite, &f, &backfill(NOW, &log)).unwrap_err();
    assert!(matches!(e.kind, PublishErrorKind::Backfill(_)), "{e}");
    assert!(unbound(&lite, &other));
    assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 1);
}

fn declarations_of(lite: &Lite) -> KeyDocument {
    let doc = lite.store.namespace(HOST).unwrap().unwrap().keydoc;
    KeyDocument::from_json(HOST, &doc).unwrap()
}

/// 8. Declarations: a `valid_from` before the clock is refused; none means now; closing with a
///    `valid_to` before the clock is refused.
#[test]
fn declarations_are_checked_against_the_clock() {
    let lite = Lite::new("rule-a-8");
    let clock = FixedClock(NOW);
    let early = "2026-11-15T11:00:00.000000Z";
    assert!(declare_at(&*lite.store, HOST, LEVEL, LICENCE, Some(early), None, &clock).is_err());
    assert!(declarations_of(&lite).declarations.is_none_or(|d| d.get(LEVEL).is_empty()));

    declare_at(&*lite.store, HOST, LEVEL, LICENCE, None, None, &clock).unwrap();
    let entry = declarations_of(&lite).declarations.unwrap().get(LEVEL)[0].clone();
    assert_eq!(entry.valid_from, NOW);

    // Closing the open entry with a valid_to before the clock.
    assert!(declare_at(&*lite.store, HOST, LEVEL, LICENCE, Some(NOW), Some(early), &clock).is_err());
    assert!(declarations_of(&lite).declarations.unwrap().get(LEVEL)[0].valid_to.is_none(), "still open");
}

/// 9. import-vectors: refused as a whole, before any write, when a name it would bind is under a
///    namespace the mesh owns; the spec's vectors import as before.
#[test]
fn import_vectors_refuses_owned_names() {
    let lite = Lite::new("rule-a-9");
    let owned = vec![HOST.to_string()];
    let dir = lite.path("vectors");
    std::fs::create_dir_all(&dir).unwrap();
    let abs = std::fs::read_to_string(spec_dir().join("vectors/formula-eval/abs.json")).unwrap();
    std::fs::write(dir.join("abs.json"), &abs).unwrap();
    let x = "mesh-test.datumesh.com/x";
    let renamed = abs.replace("calc.example.net/e", x).replace("formula-eval/abs", "formula-eval/x");
    std::fs::write(dir.join("x.json"), renamed).unwrap();
    let t = now();
    let e = datumesh_http::vectors::import_in(&*lite.store, &dir, &t, HOST, &owned).unwrap_err();
    match &e.kind {
        PublishErrorKind::OwnedVectorName(n) => assert_eq!(n, x),
        other => panic!("{other:?}"),
    }
    assert!(unbound(&lite, x));
    assert!(unbound(&lite, "calc.example.net/e"), "refused as a whole: the other vector's name too");

    // The spec's vectors, with the mesh's host, import as before.
    let spec = spec_dir().join("vectors");
    let before = Lite::new("rule-a-9-before");
    let want = datumesh_http::vectors::import(&*before.store, &spec, &t);
    let got = datumesh_http::vectors::import_in(&*lite.store, &spec, &t, HOST, &owned).unwrap();
    assert_eq!(got.to_jcs(), want.to_jcs());
}
