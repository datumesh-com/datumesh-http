//! http 0.4.10 (spec-0.6.2 in the mesh; CONTRACT-spec-0.6.2 P3, D46; SPEC 7.3, 16.8): the named
//! `test_case`.
//!
//! * On a mesh with test cells whose host has no `<host>/test_case` yet, the name binds to the named
//!   descriptor `function/test-case-named` (`case`, `function`, `function_hash`, three strings, a
//!   boolean; hash d3bc692c...). `function` is the test's function name, the name stored with the row.
//! * A name already bound to the 0.6.1 two-argument descriptor stays as it is (no rebind) and its cells
//!   are still served in the two-argument form.
//! * Two stored functions of one signature (one function hash) have distinct named cells.
//! * `describe` counts a named false cell of another host for the function whose name matches (and
//!   whose hash matches), and a two-argument false cell for every function of the hash.
//!
//! NOTE for the implementer: the older tests that call `test_case_call(hash, case)` (two arguments) on a
//! fresh mesh with test cells (tests_as_cells, test_case_foreign, stored_conformance, ...) describe the
//! 0.6.1 form; they move to the named call, or to a store that binds the two-argument form first.

#[path = "common/calculus.rs"]
mod calculus;
mod common;
#[path = "common/publish4.rs"]
mod publish4;

use calculus::*;
use common::lite::*;
use common::pub031::*;
use common::readpath::*;
use common::world::call_path;
use datumesh_core::{Cell, ReceiptBuilder, Scalar, Type};
use datumesh_store::Store;
use publish4::get;
use serde_json::json;

const NAMED_HASH: &str = "d3bc692c18ac0afee6526c2e59902e57d1390c65bd7eb747d10b2cb29c410b75";
const OTHER: &str = "pub.example";
const OTHER2: &str = "pub2.example";
const FOREIGN_TX: &str = "2026-11-15T11:00:00.000000Z";

const ALPHA: &str = "mesh-test.datumesh.com/scratch_h0410/alpha";
const BETA: &str = "mesh-test.datumesh.com/scratch_h0410/beta";

fn named_signature() -> String {
    r#"{"args":{"case":{"type":"string","valid":false},"function":{"type":"string","valid":false},"function_hash":{"type":"string","valid":false}},"order":["case","function","function_hash"],"returns":"boolean"}"#
        .to_string()
}

fn two_arg_signature() -> String {
    r#"{"args":{"case":{"type":"string","valid":false},"function_hash":{"type":"string","valid":false}},"order":["function_hash","case"],"returns":"boolean"}"#
        .to_string()
}

fn t_alpha() -> String {
    test_line(ALPHA, &format!("{ALPHA}(site: \"N\")"), None, None, num("1"))
}

fn t_beta() -> String {
    test_line(BETA, &format!("{BETA}(site: \"N\")"), None, None, num("2"))
}

/// Two stored functions of one signature, one cell and one test each.
fn lines() -> Vec<String> {
    vec![
        function_line(ALPHA, &reading_sig(), None),
        function_line(BETA, &reading_sig(), None),
        site_cell(ALPHA, "N", "1"),
        site_cell(BETA, "N", "2"),
        t_alpha(),
        t_beta(),
    ]
}

fn named_call(function: &str, hash: &str, case: &str) -> String {
    format!("{HOST}/test_case(case: \"{case}\", function: \"{function}\", function_hash: \"{hash}\")")
}

fn two_call(hash: &str, case: &str) -> String {
    format!("{HOST}/test_case(case: \"{case}\", function_hash: \"{hash}\")")
}

fn describe_path(name: &str) -> String {
    format!("/describe/{}", call_path(name))
}

fn keys_of(host: &str) -> datumesh_core::Keys {
    let mut keys = datumesh_core::Keys::new();
    keys.insert_response(
        host,
        200,
        Some(&datumesh_http::keydoc::with_key(host, None, &common::key_for(host), K_FROM).unwrap()),
    );
    keys
}

// ---- the binding ---------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fresh_mesh_with_test_cells_binds_the_named_descriptor() {
    let m = serve_lite("h0410-fresh", &lines(), Some(test_cells_config()), |_| {}).await;
    let name = format!("{HOST}/test_case");
    let bound = m.lite.store.resolve_name(&name, END).unwrap().expect("test_case is bound");
    assert_eq!(descriptor_hash(&stored_descriptor(&named_signature())), NAMED_HASH, "the vector's hash");
    let d = m.get(&format!("/descriptor/{bound}")).await;
    // The signature is the named one of spec 7.3 (`function/test-case-named`). The code member is the
    // mesh's to choose: the vector's descriptor says `stored`, but mesh-test's 0.4.9 descriptor has the
    // runtime's own lang, which the mesh computes the cells with; either way the hash may differ from the
    // vector's, so only the signature is pinned.
    assert_eq!(d["signature"], serde_json::from_str::<serde_json::Value>(&named_signature()).unwrap(), "{d}");
    assert_ne!(bound, descriptor_hash(&stored_descriptor(&two_arg_signature())));
    assert_ne!(bound, descriptor_hash(&two_arg_mesh_descriptor()), "not the 0.4.9 binding");
}

/// The descriptor mesh-test has bound since 0.4.2: the two-argument signature, the runtime's lang.
fn two_arg_mesh_descriptor() -> String {
    stored_descriptor(&two_arg_signature()).replace("\"lang\":\"stored\"", "\"lang\":\"test_case\"")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_name_bound_to_the_two_argument_form_keeps_it_and_its_cells() {
    let lite = Lite::new("h0410-two");
    let f = write(&lite, "fixture.jsonl", &lines());
    run(&lite, &f, NOW).unwrap();
    // the 0.6.1 form, bound as mesh-test has it
    let old = two_arg_mesh_descriptor();
    let d = datumesh_core::FunctionDescriptor::from_json(old.as_bytes()).unwrap();
    lite.store.put_descriptor(&d.canonical()).unwrap();
    let name = format!("{HOST}/test_case");
    lite.store.bind_name(&name, &d.hash(), "2000-01-01T00:00:00.000000Z").unwrap();
    let m = build_served(lite, Some(test_cells_config()), |_| {}).await;

    assert_eq!(m.lite.store.resolve_name(&name, END).unwrap().as_deref(), Some(d.hash().as_str()), "no rebind");
    // a single binding: the history holds no second hash
    assert_eq!(m.lite.bound_tx(&name).as_deref(), Some("2000-01-01T00:00:00.000000Z"));

    let hash = fn_hash(&m, ALPHA);
    for t in [t_alpha(), t_beta()] {
        let f = if t == t_alpha() { ALPHA } else { BETA };
        let v = m.get(&format!("/cell/{}", call_path(&two_call(&hash, &case_id(&t))))).await;
        assert_eq!(v["outcome"], "ok", "{f}: {v}");
        assert_eq!(v["receipt"]["value"], json!(true), "{v}");
        assert_eq!(v["trust"], "primary", "{v}");
        assert!(v["receipt"]["cell"]["args"].get("function").is_none(), "the two-argument form has no function: {v}");
    }
    // describe keeps counting
    assert_eq!(m.get(&describe_path(ALPHA)).await["tests"]["count"], 1);
}

// ---- the cells -----------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn named_cells_carry_the_function_name_and_verify() {
    let m = serve_lite("h0410-cells", &lines(), Some(test_cells_config()), |_| {}).await;
    let hash = fn_hash(&m, ALPHA);
    assert_eq!(hash, fn_hash(&m, BETA), "one signature, one function hash");
    for (name, t) in [(ALPHA, t_alpha()), (BETA, t_beta())] {
        let v = m.get(&format!("/cell/{}", call_path(&named_call(name, &hash, &case_id(&t))))).await;
        assert_eq!(v["outcome"], "ok", "{name}: {v}");
        assert_eq!(v["trust"], "primary", "{v}");
        assert_eq!(v["receipt"]["value"], json!(true), "{v}");
        assert_eq!(v["receipt"]["cell"]["fn"], "test_case");
        assert_eq!(v["receipt"]["cell"]["ns"], HOST);
        assert_eq!(v["receipt"]["cell"]["fn_hash"], NAMED_HASH, "{v}");
        let args = &v["receipt"]["cell"]["args"];
        assert_eq!(args["function"], json!({"type": "string", "value": name}), "{v}");
        assert_eq!(args["function_hash"]["value"], hash.as_str(), "{v}");
        assert_eq!(args["case"]["value"], case_id(&t).as_str(), "{v}");
        let r = datumesh_core::verify(&serde_json::to_vec(&v["receipt"]).unwrap(), &keys_of(HOST), None, None);
        assert!(r.ok, "{:?}", r.error());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_stored_functions_of_one_signature_have_distinct_named_cells() {
    let m = serve_lite("h0410-distinct", &lines(), Some(test_cells_config()), |_| {}).await;
    let hash = fn_hash(&m, ALPHA);
    let (ca, cb) = (case_id(&t_alpha()), case_id(&t_beta()));
    assert_ne!(ca, cb);
    let a = m.get(&format!("/cell/{}", call_path(&named_call(ALPHA, &hash, &ca)))).await;
    let b = m.get(&format!("/cell/{}", call_path(&named_call(BETA, &hash, &cb)))).await;
    assert_ne!(a["receipt"]["cell"]["args"], b["receipt"]["cell"]["args"]);
    // alpha's case under beta's name is not a test of beta, so not a value
    let x = m.get_any(&format!("/cell/{}", call_path(&named_call(BETA, &hash, &ca)))).await;
    assert_ne!(x.json["outcome"], "ok", "{}", x.json);
    assert!((400..600).contains(&x.status), "{}", x.status);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_test_is_a_false_named_cell() {
    let wrong = test_line(ALPHA, &format!("{ALPHA}(site: \"N\")"), None, None, num("7"));
    let mut ls = lines();
    ls.push(wrong.clone());
    let m = serve_lite("h0410-false", &ls, Some(test_cells_config()), |_| {}).await;
    let hash = fn_hash(&m, ALPHA);
    let v = m.get(&format!("/cell/{}", call_path(&named_call(ALPHA, &hash, &case_id(&wrong))))).await;
    assert_eq!((v["outcome"].as_str(), &v["receipt"]["value"]), (Some("ok"), &json!(false)), "{v}");
    assert_eq!(v["receipt"]["cell"]["args"]["function"]["value"], ALPHA, "{v}");
}

// ---- counting in describe ------------------------------------------------------------------------------

/// A signed `<host>/test_case` cell of `host` in the store, in the named (`named` is `Some(name)`) or the
/// two-argument form.
fn hold(lite: &Lite, host: &str, named: Option<&str>, fn_hash: &str, case: &str, value: bool) {
    let key = common::key_for(host);
    if lite.store.namespace(host).unwrap().is_none() {
        lite.store.put_namespace(host, &common::key_doc(host, &key), K_FROM).unwrap();
    }
    let sig = if named.is_some() { named_signature() } else { two_arg_signature() };
    let d = datumesh_core::FunctionDescriptor::from_json(stored_descriptor(&sig).as_bytes()).unwrap();
    lite.store.put_descriptor(&d.canonical()).unwrap();
    let mut a = vec![("case", common::s(case)), ("function_hash", common::s(fn_hash))];
    if let Some(n) = named {
        a.push(("function", common::s(n)));
    }
    let cell = Cell { ns: host.to_string(), fn_name: "test_case".into(), fn_hash: d.hash(), args: common::args(&a) };
    let id = cell.id();
    let r = ReceiptBuilder::new()
        .cell(cell)
        .value(Scalar::from_lexical(Type::Boolean, if value { "true" } else { "false" }).unwrap())
        .valid(common::args(&[]))
        .asof(FOREIGN_TX)
        .tx(FOREIGN_TX)
        .issued(FOREIGN_TX)
        .publisher(host)
        .signer_host(host)
        .licence("CC0-1.0")
        .summary("A test of someone else's function.")
        .sign(&key)
        .unwrap();
    lite.store.put_receipt(&common::receipt_json(&r)).unwrap();
    lite.store.put_version(&id, FOREIGN_TX, r.hash()).unwrap();
    let name = format!("{host}/test_case");
    if lite.store.resolve_name(&name, END).unwrap().is_none() {
        lite.store.bind_name(&name, &d.hash(), "2000-01-01T00:00:00.000000Z").unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn describe_counts_named_cells_by_name_and_two_argument_cells_by_hash() {
    for (label, base) in [("production", None), ("test cells", Some(test_cells_config()))] {
        let lite = Lite::new("h0410-count");
        let f = write(&lite, "fixture.jsonl", &lines());
        run(&lite, &f, NOW).unwrap();
        let h = lite.store.resolve_binding(ALPHA, END).unwrap().unwrap().fn_hash;
        let other_hash = "ab".repeat(32);
        // OTHER publishes the named form
        hold(&lite, OTHER, Some(ALPHA), &h, "00000000000000aa", false); // alpha only
        hold(&lite, OTHER, Some(ALPHA), &h, "00000000000000dd", false); // alpha only
        hold(&lite, OTHER, Some(ALPHA), &h, "00000000000000ee", true); // true: nobody
        hold(&lite, OTHER, Some(BETA), &h, "00000000000000bb", false); // beta only
        hold(&lite, OTHER, Some(ALPHA), &other_hash, "00000000000000ff", false); // another hash: nobody
        hold(&lite, OTHER, Some("mesh-test.datumesh.com/scratch_h0410/gamma"), &h, "0000000000000011", false); // another name: nobody
        // OTHER2 publishes the two-argument form: counts by hash, for both
        hold(&lite, OTHER2, None, &h, "00000000000000cc", false);
        let m = build_served(lite, base, |_| {}).await;

        let a = m.get(&describe_path(ALPHA)).await;
        let b = m.get(&describe_path(BETA)).await;
        assert_eq!(a["tests"]["counterexamples"], 3, "{label}: two named for alpha and the two-argument one: {a}");
        assert_eq!(b["tests"]["counterexamples"], 2, "{label}: one named for beta and the two-argument one: {b}");
        assert_eq!(a["tests"]["count"], 1, "{label}: {a}");
        assert_eq!(b["tests"]["count"], 1, "{label}: {b}");
        // core accepts the count
        let held = datumesh_core::FunctionDescriptor::from_json(&m.get_any(&format!("/descriptor/{h}")).await.body).unwrap();
        let ra = m.get_any(&describe_path(ALPHA)).await;
        let got = datumesh_core::check_describe_with(&ra.body, &Default::default(), &[held]).unwrap();
        assert_eq!(got.tests.unwrap().counterexamples, 3, "{label}");
        // the function pages agree
        let pa = get(&m, &format!("/fn/{ALPHA}?format=json"), &[]).await.json();
        let pb = get(&m, &format!("/fn/{BETA}?format=json"), &[]).await.json();
        assert_eq!(pa["tests"]["counterexamples"], 3, "{label}: {pa}");
        assert_eq!(pb["tests"]["counterexamples"], 2, "{label}: {pb}");
    }
}
