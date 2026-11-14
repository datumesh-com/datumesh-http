//! Classes in the mesh, 3 of 3: describe and short ids (spec-0.5.0, 16.8, 9.6;
//! `CONTRACT-http-0.3.0-classes.md` items 5 and 6).
//!
//! * `GET /describe/{id}` at the endpoint the mesh document names (`describe`, `{id}`): a class (by
//!   its name or its class hash) and an object (by its key encoding, its object id or its short id);
//!   the answers match `describe-class.schema.json` and `describe-object.schema.json`, and core's
//!   `check_describe_with` accepts them against what the client holds (the class descriptors and the
//!   method functions' descriptors, which the test takes from the store, not from the answer);
//! * the method table's `args` and `returns` are the method functions' (the arguments after the
//!   class keys); `description` comes from the method descriptor's metadata, and a function
//!   descriptor has none (7), so it is absent;
//! * a short id (the first 12 hex digits of an object id) is accepted where an object id is; one that
//!   names more than one object the mesh knows is `bad_request` `E_SHORT_ID_AMBIGUOUS`.
//!
//! An object the mesh knows by id is one of a bound class that it holds a cell of a method for (a
//! stored cell, or one it computed): objects are never stored, so there is nothing else to know them
//! by. Any well-formed key encoding of a bound class describes an object.

mod common;

use common::world::*;
use common::*;
use datumesh_core::json::Value;
use datumesh_core::{check_describe_with, DescribeKind, KeyValue, ObjectKey, Scalar, Type};

const A: &str = "mesh-a.example";

async fn mesh() -> (Node, World) {
    let a = Node::start(A, |c| c.budget = std::time::Duration::from_secs(30)).await;
    let w = install(&a);
    (a, w)
}

/// `/describe/<id>` with the id percent-encoded as a path.
fn describe_path(id: &str) -> String {
    format!("/describe/{}", call_path(id))
}

/// The key encoding of an object whose keys are strings (16.5).
fn encoding(class: &str, keys: &[(&str, &str)]) -> String {
    let pairs: Vec<String> = keys.iter().map(|(k, v)| format!("{k}:\"{v}\"")).collect();
    format!("{class}({})", pairs.join(","))
}

fn field<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or_else(|| panic!("no {k} in {}", String::from_utf8_lossy(&v.to_jcs())))
}

// ---------------------------------------------------------------------------------------------
// The mesh document

/// The mesh document lists the describe endpoint (9.6: `describe`, placeholder `{id}`, on this host),
/// and passes the spec's schema and core's `resolve_mesh`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mesh_document_lists_the_describe_endpoint() {
    let (a, _) = mesh().await;
    let md = a.get("/.well-known/datumesh-mesh.json").await;
    let doc = md.json();
    assert_eq!(
        doc.get("endpoints").and_then(|e| e.get("describe")).and_then(Value::as_str),
        Some(format!("https://{A}/describe/{{id}}").as_str()),
        "{}",
        md.text()
    );
    let v: serde_json::Value = serde_json::from_slice(&md.body).unwrap();
    assert!(schemas().mesh.is_valid(&v), "{}", md.text());
    let kd = a.get("/.well-known/datumesh.json").await;
    let resolved = datumesh_core::resolve_mesh(
        A,
        datumesh_core::Retrieval::Response { status: 200, body: Some(&kd.body) },
        datumesh_core::Retrieval::Response { status: 200, body: Some(&md.body) },
    )
    .unwrap();
    assert_eq!(
        resolved.mesh.endpoints.get("describe").map(String::as_str),
        Some(format!("https://{A}/describe/{{id}}").as_str())
    );
}

// ---------------------------------------------------------------------------------------------
// A class

/// describe of a class, by name: the schema, core's six-step check against the class descriptors and
/// the method descriptors the client holds, and the method table as the method functions give it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describe_a_class() {
    let (a, w) = mesh().await;
    let schemas = schemas();
    let held = w.held(&a.store);
    let functions = w.functions(&a.store);
    assert_eq!(held.len(), 4);
    for (name, hash) in
        [(&w.verse, &w.verse_hash), (&w.pair, &w.pair_hash), (&w.upper, &w.upper_hash), (&w.named, &w.named_hash)]
    {
        let r = a.get(&describe_path(name)).await;
        assert_eq!(r.status, 200, "{name}: {}", r.text());
        assert!(r.header("content-type").unwrap_or_default().starts_with("application/json"), "{name}");
        schemas.assert_describe(&r.body);
        let d = check_describe_with(&r.body, &held, &functions).unwrap_or_else(|e| panic!("{name}: {e}: {}", r.text()));
        assert_eq!(
            (d.kind, d.class.as_str(), d.class_hash.as_str()),
            (DescribeKind::Class, name.as_str(), hash.as_str())
        );
        // And with no method function held, the check goes as far as it can.
        check_describe_with(&r.body, &held, &[]).unwrap();
        // The descriptor is the flattened one; no method has a description (the descriptors carry no
        // metadata); `fn` and `fn_hash` are the method's.
        let doc = r.json();
        let class = &held[name];
        assert_eq!(doc.get("descriptor").map(Value::to_jcs), Some(class.canonical()), "{name}");
        let table = doc.get("methods").and_then(Value::as_object).unwrap();
        assert_eq!(table.len(), class.methods.len(), "{name}");
        for (m, entry) in table {
            assert_eq!(field(entry, "fn"), format!("{name}/{m}"));
            assert_eq!(field(entry, "fn_hash"), class.methods[m]);
            assert!(entry.get("description").is_none(), "{name}/{m}: a function descriptor has no description");
        }
    }
    // The table of fx_pair: the keys are not in `args`; `cross`'s date keeps its default; the
    // returns are the function's (a unit that names the key arguments).
    let r = a.get(&describe_path(&w.pair)).await;
    let doc = r.json();
    let cross = doc.get("methods").and_then(|m| m.get("cross")).unwrap();
    assert_eq!(
        cross.get("args").map(Value::to_jcs),
        Some(br#"{"date":{"default":"asof","type":"date","valid":true}}"#.to_vec())
    );
    assert_eq!(
        cross.get("returns").map(Value::to_jcs),
        Some(br#"{"type":"number","unit":{"of":{"arg":"quote"},"per":{"arg":"base"}}}"#.to_vec())
    );
    let rate = doc.get("methods").and_then(|m| m.get("rate")).unwrap();
    assert_eq!(rate.get("args").map(Value::to_jcs), Some(br#"{"date":{"type":"date","valid":true}}"#.to_vec()));
    // verse.text: `edition` is the only argument after the four keys; the return is a blob.
    let doc = a.get(&describe_path(&w.verse)).await.json();
    let text = doc.get("methods").and_then(|m| m.get("text")).unwrap();
    assert_eq!(text.get("args").map(Value::to_jcs), Some(br#"{"edition":{"type":"string","valid":false}}"#.to_vec()));
    assert_eq!(text.get("returns"), Some(&Value::String("blob".into())));
    let cons = doc.get("methods").and_then(|m| m.get("consonantal")).unwrap();
    assert_eq!(cons.get("args").map(Value::to_jcs), Some(b"{}".to_vec()));
}

/// A class by its class hash gives the answer of its name; a hash or a name the mesh does not hold
/// is `404`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describe_a_class_by_hash() {
    let (a, w) = mesh().await;
    for (name, hash) in [(&w.verse, &w.verse_hash), (&w.pair, &w.pair_hash), (&w.named, &w.named_hash)] {
        let by_name = a.get(&describe_path(name)).await;
        let by_hash = a.get(&describe_path(hash)).await;
        assert_eq!(by_hash.status, 200, "{name}: {}", by_hash.text());
        assert_eq!(by_hash.body, by_name.body, "{name}: the answer of a class by its hash is the answer by its name");
    }
    for id in [format!("{A}/ecb/nothing"), "ab".repeat(32), format!("{}/rate", w.pair), "nonsense".to_string()] {
        let r = a.get(&describe_path(&id)).await;
        assert_eq!(r.status, 404, "{id}: {}", r.text());
    }
}

// ---------------------------------------------------------------------------------------------
// An object

/// describe of an object by its key encoding: core's check against the held class, `object_id` the
/// object's id (16.5), `short_id` its first 12 digits, `call` the call form, `methods` the class's
/// method names; a class-agnostic check of numeric keys on `verse`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describe_an_object() {
    let (a, w) = mesh().await;
    let schemas = schemas();
    let held = w.held(&a.store);
    let pair_enc = encoding(&w.pair, &[("base", "EUR"), ("quote", "USD")]);
    let r = a.get(&describe_path(&pair_enc)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    schemas.assert_describe(&r.body);
    let d = check_describe_with(&r.body, &held, &[]).unwrap_or_else(|e| panic!("{e}: {}", r.text()));
    assert_eq!(
        (d.kind, d.class.as_str(), d.class_hash.as_str()),
        (DescribeKind::Object, w.pair.as_str(), w.pair_hash.as_str())
    );
    let doc = r.json();
    let want = ObjectKey::new(
        &w.pair,
        [
            ("base".to_string(), KeyValue::Scalar(Scalar::from_lexical(Type::String, "EUR").unwrap())),
            ("quote".to_string(), KeyValue::Scalar(Scalar::from_lexical(Type::String, "USD").unwrap())),
        ],
    )
    .unwrap();
    let id = want.object_id(&w.pair_hash).unwrap();
    assert_eq!(field(&doc, "object_id"), id);
    assert_eq!(d.object_id.as_deref(), Some(id.as_str()));
    assert_eq!(field(&doc, "short_id"), &id[..12]);
    assert_eq!(field(&doc, "call"), format!(r#"{}(base: "EUR", quote: "USD")"#, w.pair));
    assert_eq!(doc.get("methods").map(Value::to_jcs), Some(br#"["cross","rate"]"#.to_vec()));
    assert!(doc.get("same_as").is_none(), "a class without same_as methods has no links");
    // Typed keys: the number keys of verse are `number` scalars.
    let verse_enc = format!(r#"{}(book:"Gen",chapter:1,corpus:"tanakh",verse:1)"#, w.verse);
    let r = a.get(&describe_path(&verse_enc)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    schemas.assert_describe(&r.body);
    check_describe_with(&r.body, &held, &[]).unwrap_or_else(|e| panic!("{e}: {}", r.text()));
    let object = r.json().get("object").cloned().unwrap();
    assert_eq!(
        object.get("keys").and_then(|k| k.get("chapter")).map(Value::to_jcs),
        Some(br#"{"type":"number","value":"1"}"#.to_vec())
    );
    // An object with no cell is an object all the same: objects are never stored.
    let none = encoding(&w.pair, &[("base", "EUR"), ("quote", "CHF")]);
    let r = a.get(&describe_path(&none)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    check_describe_with(&r.body, &held, &[]).unwrap();
}

/// An object by object id and by short id gives the answer of its key encoding (an object the mesh
/// knows: it holds a cell of one of its class's methods); an id it does not know is `404`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describe_an_object_by_id_and_by_short_id() {
    let (a, w) = mesh().await;
    let held = w.held(&a.store);
    for enc in [
        encoding(&w.pair, &[("base", "EUR"), ("quote", "USD")]),
        encoding(&w.pair, &[("base", "EUR"), ("quote", "GBP")]),
        encoding(&w.named, &[("base", "GBP"), ("quote", "USD")]),
        encoding(&w.upper, &[("base", "EUR"), ("quote", "USD")]),
        format!(r#"{}(book:"Gen",chapter:1,corpus:"tanakh",verse:1)"#, w.verse),
    ] {
        let by_key = a.get(&describe_path(&enc)).await;
        assert_eq!(by_key.status, 200, "{enc}: {}", by_key.text());
        let doc = by_key.json();
        let (id, short) = (field(&doc, "object_id").to_string(), field(&doc, "short_id").to_string());
        assert_eq!((id.len(), short.len()), (64, 12));
        for what in [&id, &short] {
            let r = a.get(&describe_path(what)).await;
            assert_eq!(r.status, 200, "{enc} by {what}: {}", r.text());
            assert_eq!(r.body, by_key.body, "{enc} by {what}");
            check_describe_with(&r.body, &held, &[]).unwrap();
        }
    }
    // Unknown: well-formed ids no object of the mesh has.
    for id in ["f".repeat(64), "f".repeat(12), "0".repeat(12)] {
        let r = a.get(&describe_path(&id)).await;
        assert_eq!(r.status, 404, "{id}: {}", r.text());
    }
}

// ---------------------------------------------------------------------------------------------
// Short ids

/// Two keys of `fx_pair` whose objects have the same short id (the first 12 hex digits of the object
/// id, a 48-bit collision, found by [`find_a_short_id_collision`] for this world's class hash). If
/// the world's descriptors change, the class hash changes, and this test says so.
const COLLISION: (&str, &str) = ("59176a321e94", "8f44d5efe915");

fn object_id(class: &str, class_hash: &str, base: &str) -> String {
    ObjectKey::new(
        class,
        [
            ("base".to_string(), KeyValue::Scalar(Scalar::from_lexical(Type::String, base).unwrap())),
            ("quote".to_string(), KeyValue::Scalar(Scalar::from_lexical(Type::String, "EUR").unwrap())),
        ],
    )
    .unwrap()
    .object_id(class_hash)
    .unwrap()
}

/// A short id naming more than one object the mesh knows is `bad_request` `E_SHORT_ID_AMBIGUOUS`
/// (16.8, 5.5): at the endpoint, never a pick of one of them. The full ids still describe each.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_ambiguous_short_id_is_e_short_id_ambiguous() {
    let (a, w) = mesh().await;
    let (x, y) = COLLISION;
    let (ix, iy) = (object_id(&w.pair, &w.pair_hash, x), object_id(&w.pair, &w.pair_hash, y));
    assert_ne!(ix, iy);
    assert_eq!(
        &ix[..12],
        &iy[..12],
        "COLLISION is not a short id collision for this world's fx_pair ({}): run `cargo test --test \
         classes_describe -- --ignored --nocapture find_a_short_id_collision` and put its keys in COLLISION",
        w.pair_hash
    );
    // The mesh knows both objects: it holds a cell of fx_pair/rate for each.
    for base in [x, y] {
        a.put_value(
            &format!("{}/rate", w.pair),
            &args(&[("base", s(base)), ("date", d(DAY)), ("quote", s("EUR"))]),
            n("1"),
            TV,
        );
    }
    let short = &ix[..12];
    let r = a.get(&describe_path(short)).await;
    assert_eq!(r.status, 400, "{}", r.text());
    let doc = r.json();
    assert_eq!(doc.get("outcome").and_then(Value::as_str), Some("bad_request"));
    assert_eq!(doc.get("error").and_then(Value::as_str), Some("E_SHORT_ID_AMBIGUOUS"), "{}", r.text());
    assert_eq!(doc.as_object().unwrap().len(), 2, "{}", r.text());
    // The envelope is one a client accepts for a request that could not be bound (no receipt).
    let schemas = schemas();
    schemas.assert_envelope(&r.body);
    // Each full id describes its own object.
    let held = w.held(&a.store);
    for (id, base) in [(&ix, x), (&iy, y)] {
        let r = a.get(&describe_path(id)).await;
        assert_eq!(r.status, 200, "{}", r.text());
        check_describe_with(&r.body, &held, &[]).unwrap();
        let o = r.json();
        assert_eq!(
            o.get("object")
                .and_then(|o| o.get("keys"))
                .and_then(|k| k.get("base"))
                .and_then(|b| b.get("value"))
                .and_then(Value::as_str),
            Some(base)
        );
    }
}

/// The tool that finds [`COLLISION`]: Floyd's cycle search over `f(base) = the short id of the
/// object fx_pair(base, "EUR")`, 2^24 steps expected, a few minutes unoptimised. Run it when the
/// world's classes change:
/// `cargo test --release --test classes_describe -- --ignored --nocapture find_a_short_id_collision`.
#[test]
#[ignore = "a tool: prints the keys for COLLISION"]
fn find_a_short_id_collision() {
    let node = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let (class, hash) = node.block_on(async {
        let (_a, w) = mesh().await;
        (w.pair.clone(), w.pair_hash.clone())
    });
    let f = |x: &str| object_id(&class, &hash, x)[..12].to_string();
    let (mut tortoise, mut hare) = (f("start"), f(&f("start")));
    while tortoise != hare {
        tortoise = f(&tortoise);
        hare = f(&f(&hare));
    }
    let mut tortoise = "start".to_string();
    loop {
        let (nt, nh) = (f(&tortoise), f(&hare));
        if nt == nh {
            println!("COLLISION: ({tortoise:?}, {hare:?}) -> {nt}");
            return;
        }
        tortoise = nt;
        hare = nh;
    }
}
