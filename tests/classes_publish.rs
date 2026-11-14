//! Classes in the mesh, 1 of 3 (spec-0.5.0; `CONTRACT-http-0.3.0-classes.md` items 2 and 3): the
//! `class` line of `publish-stored` (16.2, with the owner rule), the function lines that carry
//! `classes`, `default` and `key_normalisation` (7), and the bindings resource (9.7) answering a
//! class name with `class_hash` and its methods as functions.

mod common;

use std::io::Write;

use common::world::*;
use common::*;
use datumesh_core::json::Value;
use datumesh_http::publish::{publish_stored, publish_stored_in, PublishError, Published};

const A: &str = "mesh-a.example";

fn tempdir(label: &str) -> std::path::PathBuf {
    let d = out_dir().join(format!("classes-publish-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn put(path: &std::path::Path, lines: &[String]) {
    let mut f = std::fs::File::create(path).unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
}

fn st(v: &str) -> Value {
    Value::String(v.to_string())
}

fn sig(v: &str) -> Value {
    datumesh_core::json::parse(v.as_bytes()).unwrap()
}

fn line(members: Vec<(&str, Value)>) -> String {
    String::from_utf8(Value::Object(members.into_iter().map(|(k, v)| (k.to_string(), v)).collect()).to_jcs()).unwrap()
}

/// `{"function": ..., "signature": ..., "bound": ...}`
fn function_line(name: &str, signature: &str, bound: &str) -> String {
    line(vec![("bound", st(bound)), ("function", st(name)), ("signature", sig(signature))])
}

/// `{"formula": ..., "source": ..., "signature": ..., "bound": ...}`
fn formula_line(name: &str, source: &str, signature: &str, bound: &str) -> String {
    line(vec![("bound", st(bound)), ("formula", st(name)), ("signature", sig(signature)), ("source", st(source))])
}

/// `{"class": ..., "descriptor": {...as written...}, "bound": ...}`
fn class_line(name: &str, descriptor: &str, bound: &str) -> String {
    line(vec![("bound", st(bound)), ("class", st(name)), ("descriptor", sig(descriptor))])
}

/// The first file of the world: the method functions (stored and formula), bound from `BIND_TX`.
fn function_lines(w: &World) -> Vec<String> {
    let (v, p, u, nm) = (&w.verse, &w.pair, &w.upper, &w.named);
    vec![
        function_line(&format!("{v}/text"), &verse_text_sig(), BIND_TX),
        formula_line(
            &format!("{v}/consonantal"),
            &format!(
                r#"remove_chars(text_of({v}/text(corpus: corpus, book: book, chapter: chapter, verse: verse, edition: "uxlc")), "{}")"#,
                strip_chars()
            ),
            &verse_consonantal_sig(),
            BIND_TX,
        ),
        function_line(&format!("{p}/rate"), &fx_rate_sig(), BIND_TX),
        formula_line(
            &format!("{p}/cross"),
            &format!(
                r#"{p}/rate(base: "EUR", quote: quote, date: date) / {p}/rate(base: "EUR", quote: base, date: date)"#
            ),
            &fx_cross_sig(),
            BIND_TX,
        ),
        function_line(&format!("{u}/rate"), &upper_rate_sig(), BIND_TX),
        function_line(&format!("{nm}/rate"), &fx_rate_sig(), BIND_TX),
        function_line(&format!("{nm}/named"), &named_sig(nm), BIND_TX),
    ]
}

/// The third file: the formulas that need the classes (a dispatch over `[fx_upper]`).
fn formula_lines(w: &World) -> Vec<String> {
    vec![
        formula_line(&format!("{}/ecb/upper_rate", w.host), "pair.rate(date: date)", &dispatch_sig(&w.upper), TD),
        formula_line(
            &format!("{}/ecb/lower_rate", w.host),
            &format!(r#"{}/rate(base: lower(x), quote: "USD", date: date)"#, w.upper),
            &lower_sig(),
            TD,
        ),
    ]
}

fn run(a: &Node, owned: &[String], path: &std::path::Path) -> Result<Published, PublishError> {
    let key = a.key.clone();
    bl(|| publish_stored_in(&*a.store, &key, A, owned, path))
}

/// A `class` line defines the class with core and formula, stores it and binds it with its methods,
/// in the order method functions, classes, then the formulas that dispatch; again changes nothing;
/// the class is the one the store's own API makes from the same descriptors (so its hash is the
/// hash of 16.1's flattened descriptor); the function lines carry `classes`, `default` and
/// `key_normalisation` in their signatures.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn class_lines_define_store_and_bind() {
    let a = Node::start(A, |_| {}).await;
    let w = World::new(A);
    let dir = tempdir("lines");
    let own = vec![A.to_string()];

    let f1 = dir.join("functions.jsonl");
    put(&f1, &function_lines(&w));
    let first = run(&a, &own, &f1).unwrap();
    assert_eq!((first.functions, first.classes), (7, 0), "{first:?}");

    let sources = bl(|| w.class_sources(&a.store, TC));
    assert_eq!(sources.len(), 4);
    let f2 = dir.join("classes.jsonl");
    put(&f2, &sources.iter().map(|(n, d)| class_line(n, d, TC)).collect::<Vec<_>>());
    let second = run(&a, &own, &f2).unwrap();
    assert_eq!((second.functions, second.classes), (0, 4), "{second:?}");

    let f3 = dir.join("dispatch.jsonl");
    put(&f3, &formula_lines(&w));
    let third = run(&a, &own, &f3).unwrap();
    assert_eq!((third.functions, third.classes), (2, 0), "{third:?}");

    // Idempotent: a name already bound to that hash is not bound again.
    for f in [&f1, &f2, &f3] {
        let again = run(&a, &own, f).unwrap();
        assert_eq!((again.functions, again.classes), (0, 0), "{f:?}: {again:?}");
    }

    // The same world built through the library: the same class hashes, the same bindings.
    let b = Node::start(A, |_| {}).await;
    let wb = install(&b);
    for (name, want) in
        [(&w.verse, &wb.verse_hash), (&w.pair, &wb.pair_hash), (&w.upper, &wb.upper_hash), (&w.named, &wb.named_hash)]
    {
        let (hash, descriptor) = w.descriptor(&a.store, name);
        assert_eq!(&hash, want, "{name}: the class hash");
        assert_eq!(hash, descriptor.hash());
        assert_eq!(bl(|| a.store.class(&hash).unwrap().unwrap()), descriptor.canonical(), "{name}: stored bytes");
        for (m, fh) in &descriptor.methods {
            assert_eq!(
                bl(|| a.store.resolve_name(&format!("{name}/{m}"), "9999-12-31T23:59:59.999999Z").unwrap()).as_deref(),
                Some(fh.as_str()),
                "{name}/{m} is bound to the method's function"
            );
        }
        let binding = bl(|| a.store.resolve_class_binding(name, "9999-12-31T23:59:59.999999Z").unwrap().unwrap());
        assert_eq!(
            (binding.class_hash.as_str(), binding.tx.as_str()),
            (hash.as_str(), TC),
            "{name}: bound from `bound`"
        );
    }
    // The function lines kept the members of 7: classes, default, key_normalisation.
    for (name, member) in [
        (format!("{}/ecb/upper_rate", A), "\"classes\""),
        (format!("{}/cross", w.pair), "\"default\":\"asof\""),
        (format!("{}/rate", w.upper), "\"key_normalisation\":\"upper_ascii\""),
    ] {
        let h = bl(|| a.store.resolve_name(&name, "9999-12-31T23:59:59.999999Z").unwrap().unwrap());
        let d = String::from_utf8(bl(|| a.store.descriptor(&h).unwrap().into_owned())).unwrap();
        assert!(d.contains(member), "{name}: {d}");
    }
    // `publish_stored` (the host as the only namespace) is `publish_stored_in` with `[host]`.
    let key = a.key.clone();
    let same = bl(|| publish_stored(&*a.store, &key, A, &f2)).unwrap();
    assert_eq!((same.functions, same.classes), (0, 0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The owner rule (16.2 step 1): a class is defined under one of the mesh's own namespaces, else
/// `E_CLASS_OWNER`, and nothing is stored or bound; another namespace of the mesh is accepted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_class_under_a_namespace_the_mesh_does_not_own_is_refused() {
    let a = Node::start(A, |_| {}).await;
    let w = World::new(A);
    let dir = tempdir("owner");
    let own = vec![A.to_string()];
    let f1 = dir.join("functions.jsonl");
    put(&f1, &function_lines(&w));
    run(&a, &own, &f1).unwrap();
    let descriptor = bl(|| w.class_sources(&a.store, TC)).into_iter().find(|(n, _)| *n == w.pair).unwrap().1;
    let stored_before = bl(|| a.store.class(&sig_hash(&descriptor)).unwrap());
    assert!(stored_before.is_none());
    for name in ["other.example/ecb/fx_pair", &format!("{A}.evil.example/ecb/fx_pair"), "ecb.mesh-a.example/fx_pair"] {
        let f = dir.join("owner.jsonl");
        put(&f, &[class_line(name, &descriptor, TC)]);
        let e = run(&a, &own, &f).unwrap_err();
        assert!(e.detail.contains("E_CLASS_OWNER"), "{name}: {e}");
        assert_eq!(e.line, 1);
        assert_eq!(bl(|| a.store.resolve_class(name, "9999-12-31T23:59:59.999999Z").unwrap()), None, "{name}");
    }
    // Nothing was stored for the refused lines (the owner rule comes before put_class).
    assert!(bl(|| a.store.class(&sig_hash(&descriptor)).unwrap()).is_none());
    // A mesh that owns two namespaces accepts a class under either: the owner is the class name's
    // host (the namespace is the name up to its last `/`).
    let two = vec![A.to_string(), "other.example".to_string()];
    let f = dir.join("two.jsonl");
    put(&f, &[class_line("other.example/ecb/fx_pair", &descriptor, TC), class_line(&w.pair, &descriptor, TC)]);
    let ok = run(&a, &two, &f).unwrap();
    assert_eq!(ok.classes, 2, "{ok:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The hash of a class descriptor as written (no parent: it is its own flattened form).
fn sig_hash(descriptor: &str) -> String {
    datumesh_core::ClassDescriptor::from_json(descriptor.as_bytes()).unwrap().hash()
}

/// Define's own checks reach the line as their codes: a method whose function is not known, and a
/// key rule the method functions do not carry (Q6, 16.2 step 6): `E_CLASS_METHOD`; nothing bound.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn define_failures_come_back_with_their_codes() {
    let a = Node::start(A, |_| {}).await;
    let w = World::new(A);
    let dir = tempdir("define");
    let own = vec![A.to_string()];
    let f1 = dir.join("functions.jsonl");
    put(&f1, &function_lines(&w));
    run(&a, &own, &f1).unwrap();
    let h = |n: &str| bl(|| a.store.resolve_name(n, TC).unwrap().unwrap());
    let unknown = "ab".repeat(32);
    let cases = [
        (
            "a method hash no descriptor has",
            class_json(NONE_KEYS_PAIR, &["base", "quote"], &[("rate", &unknown)], A, ""),
            "E_CLASS_METHOD",
        ),
        (
            "a rule on the key that the method's arguments do not carry",
            class_json(UPPER_KEYS_PAIR, &["base", "quote"], &[("rate", &h(&format!("{}/rate", w.pair)))], A, ""),
            "E_CLASS_METHOD",
        ),
    ];
    for (what, descriptor, code) in cases {
        let f = dir.join("bad.jsonl");
        put(&f, &[class_line(&format!("{A}/ecb/bad"), &descriptor, TC)]);
        let e = run(&a, &own, &f).unwrap_err();
        assert!(e.detail.contains(code), "{what}: {e}");
        assert_eq!(bl(|| a.store.resolve_class(&format!("{A}/ecb/bad"), "9999-12-31T23:59:59.999999Z").unwrap()), None);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn binding_of(doc: &Value, name: &str) -> Option<Value> {
    doc.get("bindings").and_then(|b| b.get(name)).cloned()
}

/// The bindings resource (9.7): a class name is answered with `class_hash` in place of `fn_hash`
/// (and `since`: the instant it was bound from); a method name `N/m` is answered as a function, with
/// the hash the class's descriptor gives it; several names in one request; as of an earlier instant
/// the class is unbound while the method names have their earlier function bindings.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_bindings_resource_answers_classes() {
    let a = Node::start(A, |_| {}).await;
    let w = install(&a);
    let q = |names: &[&str], asof: Option<&str>| {
        let mut s = String::from("/bindings?");
        let ns: Vec<String> = names.iter().map(|n| format!("name={}", qenc(n))).collect();
        s.push_str(&ns.join("&"));
        if let Some(t) = asof {
            s.push_str(&format!("&asof={}", qenc(t)));
        }
        s
    };
    let (_, pair) = w.descriptor(&a.store, &w.pair);

    let r = a.get(&q(&[&w.pair], Some(ASOF))).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let doc = r.json();
    assert_eq!(doc.get("asof").and_then(Value::as_str), Some(ASOF));
    let b = binding_of(&doc, &w.pair).unwrap_or_else(|| panic!("no binding for the class: {}", r.text()));
    assert_eq!(b.get("class_hash").and_then(Value::as_str), Some(w.pair_hash.as_str()), "{}", r.text());
    assert_eq!(b.get("since").and_then(Value::as_str), Some(TC));
    assert!(b.get("fn_hash").is_none(), "a binding has fn_hash or class_hash, never both: {}", r.text());
    assert_eq!(b.as_object().unwrap().len(), 2);

    // Methods: functions, with the class descriptor's hash; bound together with the class (since TC).
    let rate = format!("{}/rate", w.pair);
    let r = a.get(&q(&[&rate, &format!("{}/cross", w.pair)], Some(ASOF))).await;
    let doc = r.json();
    for m in ["rate", "cross"] {
        let b =
            binding_of(&doc, &format!("{}/{m}", w.pair)).unwrap_or_else(|| panic!("no binding for {m}: {}", r.text()));
        assert_eq!(b.get("fn_hash").and_then(Value::as_str), Some(pair.methods[m].as_str()), "{m}");
        assert_eq!(b.get("since").and_then(Value::as_str), Some(TC), "{m}: bound in the class's transaction");
        assert!(b.get("class_hash").is_none());
    }

    // Everything in one request: classes and methods, with a name that is unbound.
    let names = [w.verse.as_str(), w.upper.as_str(), "mesh-a.example/ecb/none", &format!("{}/text", w.verse)];
    let r = a.get(&q(&names, None)).await;
    let doc = r.json();
    assert_eq!(
        binding_of(&doc, &w.verse).and_then(|b| b.get("class_hash").cloned()),
        Some(Value::String(w.verse_hash.clone()))
    );
    assert_eq!(
        binding_of(&doc, &w.upper).and_then(|b| b.get("class_hash").cloned()),
        Some(Value::String(w.upper_hash.clone()))
    );
    assert!(binding_of(&doc, "mesh-a.example/ecb/none").is_none());
    assert!(binding_of(&doc, &format!("{}/text", w.verse)).unwrap().get("fn_hash").is_some());
    assert_eq!(doc.get("bindings").and_then(Value::as_object).unwrap().len(), 3, "{}", r.text());

    // Before the class was bound it is unbound; its method names are still the functions bound
    // before (by BIND_TX), with the earlier `since`.
    let early = "2020-01-01T12:00:00.000000Z";
    let r = a.get(&q(&[&w.pair, &rate], Some(early))).await;
    let doc = r.json();
    assert!(binding_of(&doc, &w.pair).is_none(), "{}", r.text());
    let b = binding_of(&doc, &rate).expect("the function binding before the class");
    assert_eq!(b.get("since").and_then(Value::as_str), Some(BIND_TX));
    assert_eq!(b.get("fn_hash").and_then(Value::as_str), Some(pair.methods["rate"].as_str()));

    // A client of the mesh document finds the bindings resource as before.
    let md = a.get("/.well-known/datumesh-mesh.json").await.json();
    assert_eq!(md.get("bindings").and_then(Value::as_str), Some(format!("https://{A}/bindings").as_str()));
}
