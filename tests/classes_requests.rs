//! Classes in the mesh, 2 of 3: requests by call form (spec-0.5.0, 5.8, 16.3, 16.7;
//! `CONTRACT-http-0.3.0-classes.md` item 4), at `/cell/<call>` and at the batch endpoint.
//!
//! * **method-call equivalence** over HTTP: `N(keys).m(args)` and the plain read `N/m(...)` (and its
//!   URI) give the same cell id and the same receipt hash, for stored and for computed cells;
//! * unquoted values by position type, `N[label]` through the class's `lookup`, the `asof` default
//!   taking the call's `asof` or the mesh's now (the cell URI carries the value), bare names through
//!   the alias map;
//! * `resolved` (5.5) on every answer where something was resolved, and only there;
//! * `E_KEY_NORMALISATION` and `E_KEY_ENCODING` as `bad_request`.
//!
//! The world is `common::world` (the live check's `verse` and `fx_pair`, and `fx_upper`,
//! `fx_named`), built straight in the store.

mod common;

use common::world::*;
use common::*;
use datumesh_core::json::{self, Value};
use datumesh_core::Receipt;

const A: &str = "mesh-a.example";

fn aliases(c: &mut datumesh_http::Config) {
    c.aliases.insert("xcross".into(), format!("{A}/ecb/fx_pair/cross"));
    c.aliases.insert("rate".into(), format!("{A}/ecb/fx_pair/rate"));
    c.budget = std::time::Duration::from_secs(30);
}

async fn mesh() -> (Node, World) {
    let a = Node::start(A, aliases).await;
    let w = install(&a);
    (a, w)
}

/// What a `/cell` answer says about its cell: the receipt's hash, the cell's id, `resolved`.
struct Got {
    status: u16,
    env: Value,
    receipt: Option<Vec<u8>>,
    hash: Option<String>,
    cell_id: Option<String>,
}

fn got(r: &Resp) -> Got {
    let env = r.json();
    let receipt = env.get("receipt").map(Value::to_jcs);
    let parsed = receipt.as_ref().map(|b| Receipt::from_json(b).unwrap_or_else(|e| panic!("{e}: {}", r.text())));
    Got {
        status: r.status,
        hash: parsed.as_ref().map(|p| p.hash().to_string()),
        cell_id: parsed.as_ref().map(|p| p.cell().id()),
        receipt,
        env,
    }
}

impl Got {
    fn resolved(&self) -> Option<&Value> {
        self.env.get("resolved")
    }
    fn error(&self) -> Option<&str> {
        self.env.get("error").and_then(Value::as_str)
    }
    fn outcome(&self) -> &str {
        self.env.get("outcome").and_then(Value::as_str).unwrap_or("")
    }
}

/// `resolved`: `call`, then each resolution `(from, kind, to)` (a `lookup` one with its `asof`).
/// A resolution: `from`, `kind`, `to` and, for a lookup, its `asof`.
type Resolution = (String, String, String, Option<String>);

fn resolved_of(g: &Got) -> (String, Vec<Resolution>) {
    let r = g.resolved().unwrap_or_else(|| panic!("no resolved in {}", String::from_utf8_lossy(&g.env.to_jcs())));
    let call = r.get("call").and_then(Value::as_str).unwrap().to_string();
    let Some(Value::Array(rs)) = r.get("resolutions") else { panic!("resolutions") };
    let list = rs
        .iter()
        .map(|x| {
            let f = |k: &str| x.get(k).and_then(Value::as_str).map(str::to_string);
            (f("from").unwrap(), f("kind").unwrap(), f("to").unwrap(), f("asof"))
        })
        .collect();
    (call, list)
}

fn res(from: &str, kind: &str, to: &str) -> Resolution {
    (from.into(), kind.into(), to.into(), None)
}

/// A call as a URL path: percent-encoded, with its `?asof=`.
fn path(call: &str, asof: Option<&str>) -> String {
    let mut p = format!("/cell/{}", call_path(call));
    if let Some(a) = asof {
        p.push_str(&format!("?asof={a}"));
    }
    p
}

/// Method calls, plain reads of the same cell, the function's name, the asof.
type Case = (Vec<String>, Vec<String>, String, Option<&'static str>);

// ---------------------------------------------------------------------------------------------
// Method-call equivalence

/// `N(keys).m(args)` is the same request as the plain read `N/m(...)`: one cell id, one receipt
/// hash, byte-identical receipts, a receipt that verifies, and an envelope a client accepts for
/// that cell. For stored cells (a blob, a number) and for computed ones.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_method_call_and_the_plain_read_are_one_cell() {
    let (a, w) = mesh().await;
    let (v, p) = (&w.verse, &w.pair);
    // (method calls, plain reads, the cell's function name, asof)
    let cases: Vec<Case> = vec![
        (
            vec![
                path(&format!("{v}(tanakh, Gen, 1, 1).text(uxlc)"), None),
                path(&format!("{v}(\"tanakh\", \"Gen\", 1, 1).text(\"uxlc\")"), None),
                path(&format!("{v}(tanakh,Gen,1,1).text(uxlc)"), None),
                path(&format!("{v}(corpus: tanakh, book: Gen, chapter: 1, verse: 1).text(edition: uxlc)"), None),
                path(&format!("{v}(verse: 1, chapter: 1, book: Gen, corpus: tanakh).text(uxlc)"), None),
            ],
            vec![
                format!("/cell/{v}/text?book=Gen&chapter=1&corpus=tanakh&edition=uxlc&verse=1"),
                path(&format!("{v}/text(\"tanakh\", \"Gen\", 1, 1, \"uxlc\")"), None),
                path(
                    &format!("{v}/text(corpus: \"tanakh\", book: \"Gen\", chapter: 1, verse: 1, edition: \"uxlc\")"),
                    None,
                ),
            ],
            format!("{v}/text"),
            None,
        ),
        (
            vec![
                path(&format!("{p}(EUR, USD).rate({DAY})"), None),
                path(&format!("{p}(\"EUR\", \"USD\").rate({DAY})"), None),
                path(&format!("{p}(base: EUR, quote: USD).rate(date: {DAY})"), None),
            ],
            vec![
                format!("/cell/{p}/rate?base=EUR&date={DAY}&quote=USD"),
                path(&format!("{p}/rate(\"EUR\", \"USD\", {DAY})"), None),
            ],
            format!("{p}/rate"),
            None,
        ),
        // A computed cell: same asof on both sides.
        (
            vec![
                path(&format!("{v}(tanakh, Gen, 1, 1).consonantal()"), Some(ASOF)),
                path(&format!("{v}(\"tanakh\", \"Gen\", 1, 1).consonantal()"), Some(ASOF)),
            ],
            vec![
                format!("/cell/{v}/consonantal?book=Gen&chapter=1&corpus=tanakh&verse=1&asof={ASOF}"),
                path(&format!("{v}/consonantal(\"tanakh\", \"Gen\", 1, 1)"), Some(ASOF)),
            ],
            format!("{v}/consonantal"),
            Some(ASOF),
        ),
        (
            vec![
                path(&format!("{p}(GBP, USD).cross({DAY})"), Some(ASOF)),
                path(&format!("{p}(base: GBP, quote: USD).cross(date: {DAY})"), Some(ASOF)),
            ],
            vec![
                format!("/cell/{p}/cross?base=GBP&date={DAY}&quote=USD&asof={ASOF}"),
                path(&format!("{p}/cross(\"GBP\", \"USD\", {DAY})"), Some(ASOF)),
            ],
            format!("{p}/cross"),
            Some(ASOF),
        ),
    ];
    for (methods, plains, name, asof) in cases {
        let reference = a.get(&plains[0]).await;
        let want = got(&reference);
        assert_eq!(want.outcome(), "ok", "{name}: {}", reference.text());
        assert!(want.resolved().is_none(), "{name}: a plain read of a full name resolves nothing");
        let id = want.cell_id.clone().unwrap();
        // The envelope is what a client accepts for that cell (checks 1-6 of 5.5, resolved included).
        assert_eq!(check(&reference, &a.keys(), &id, asof, None), Ok("ok".to_string()), "{name}");
        for path in plains.iter().skip(1).chain(methods.iter()) {
            let r = a.get(path).await;
            let g = got(&r);
            assert_eq!(g.status, 200, "{path}: {}", r.text());
            assert_eq!(g.cell_id, want.cell_id, "{path}: the cell id");
            assert_eq!(g.hash, want.hash, "{path}: the receipt hash");
            assert_eq!(g.receipt, want.receipt, "{path}: the receipt");
            assert_eq!(g.env.get("trust"), want.env.get("trust"), "{path}");
            assert_eq!(g.env.get("fn_hash"), want.env.get("fn_hash"), "{path}");
            // The mesh has no reference classes (16.9): it never says `shadows`.
            assert!(g.env.get("shadows").is_none(), "{path}: {}", r.text());
            assert_eq!(check(&r, &a.keys(), &id, asof, None), Ok("ok".to_string()), "{path}: {}", r.text());
            if methods.contains(path) {
                let (call, rs) = resolved_of(&g);
                assert!(call.starts_with(&format!("{name}(")), "{path}: resolved.call is the full call: {call}");
                assert!(rs.iter().any(|x| x.1 == "method" && x.2 == name), "{path}: a method resolution: {rs:?}");
            } else {
                assert!(g.resolved().is_none(), "{path}: nothing resolved");
            }
        }
        let receipt = want.receipt.clone().unwrap();
        let verified = datumesh_core::verify(&receipt, &a.keys(), None, None);
        assert!(verified.ok, "{name}: {:?}", verified.error());
    }
    // The values themselves: the blob's text through the formula, the cross rate 1.2 / 0.8.
    let r = got(&a.get(&path(&format!("{v}(tanakh, Gen, 1, 1).consonantal()"), Some(ASOF))).await);
    assert_eq!(r.env.get("receipt").and_then(|x| x.get("value")).and_then(Value::as_str), Some(GEN_1_1_CONSONANTAL));
    assert_eq!(r.env.get("trust").and_then(Value::as_str), Some("computed"));
    let r = got(&a.get(&path(&format!("{p}(GBP, USD).cross({DAY})"), Some(ASOF))).await);
    assert_eq!(r.env.get("receipt").and_then(|x| x.get("value")).and_then(Value::as_str), Some("1.5"));
}

/// `resolved` of a method call: its full canonical call (the full name and every argument labelled,
/// ascending, `name: <literal>`, then `?asof=`) and the `method` resolution; and nothing else.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resolved_names_the_full_call_and_the_method() {
    let (a, w) = mesh().await;
    let v = &w.verse;
    let g = got(&a.get(&path(&format!("{v}(tanakh, Gen, 1, 1).text(uxlc)"), Some(ASOF))).await);
    let (call, rs) = resolved_of(&g);
    assert_eq!(
        call,
        format!(r#"{v}/text(book: "Gen", chapter: 1, corpus: "tanakh", edition: "uxlc", verse: 1)?asof={ASOF}"#)
    );
    assert_eq!(rs, vec![res(&format!("{v}(...).text"), "method", &format!("{v}/text"))]);
    // aliases_tx is present exactly when an alias snapshot was used: none here.
    assert!(g.resolved().unwrap().get("aliases_tx").is_none());
    // A pinned call of a full name resolves nothing; so does a cell URI.
    let hash = bl(|| a.store.resolve_name(&format!("{v}/text"), ASOF).unwrap().unwrap());
    let pinned = a.get(&path(&format!("{v}/text@{hash}(\"tanakh\", \"Gen\", 1, 1, \"uxlc\")"), None)).await;
    assert_eq!(pinned.outcome(), "ok", "{}", pinned.text());
    assert!(got(&pinned).resolved().is_none(), "{}", pinned.text());
}

// ---------------------------------------------------------------------------------------------
// resolved on a short call with a defaulted date

/// A short call whose `date` is left out: the default takes the call's `asof` (the cell URI carries
/// the value, so the call is the plain read with that date), `resolved` records the method and the
/// default (in the order alias, lookup, method, default), and a bare name through the alias map is an
/// `alias` resolution.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resolved_on_a_short_call_with_a_defaulted_date() {
    let (a, w) = mesh().await;
    let p = &w.pair;
    let plain = a.get(&format!("/cell/{p}/cross?base=GBP&date={DAY}&quote=USD&asof={ASOF}")).await;
    let want = got(&plain);
    assert_eq!(want.outcome(), "ok", "{}", plain.text());
    let full = format!(r#"{p}/cross(base: "GBP", date: {DAY}, quote: "USD")?asof={ASOF}"#);

    // A method call: the date defaults to the asof's date.
    let r = a.get(&path(&format!("{p}(GBP, USD).cross()"), Some(ASOF))).await;
    let g = got(&r);
    assert_eq!(g.outcome(), "ok", "{}", r.text());
    assert_eq!(
        (g.hash.clone(), g.cell_id.clone()),
        (want.hash.clone(), want.cell_id.clone()),
        "the default is bound in the cell"
    );
    let (call, rs) = resolved_of(&g);
    assert_eq!(call, full);
    assert_eq!(rs, vec![res(&format!("{p}(...).cross"), "method", &format!("{p}/cross")), res("date", "default", DAY)]);
    let id = want.cell_id.clone().unwrap();
    assert_eq!(check(&r, &a.keys(), &id, Some(ASOF), None), Ok("ok".to_string()), "{}", r.text());

    // A bare name of the alias map, and `@alias`: the alias resolution comes first.
    for call in ["xcross(GBP, USD)", "@xcross(GBP, USD)", "xcross(base: GBP, quote: USD)"] {
        let r = a.get(&path(call, Some(ASOF))).await;
        let g = got(&r);
        assert_eq!(g.outcome(), "ok", "{call}: {}", r.text());
        assert_eq!(g.hash, want.hash, "{call}");
        let (c, rs) = resolved_of(&g);
        assert_eq!(c, full, "{call}");
        let from = if call.starts_with('@') { "@xcross" } else { "xcross" };
        assert_eq!(rs, vec![res(from, "alias", &format!("{p}/cross")), res("date", "default", DAY)], "{call}");
    }

    // No asof: the default takes the mesh's now (here no cell has a value for that date: not_covered,
    // which carries `resolved` as well). The date in `resolved.call` is today's, UTC.
    let before = now();
    let r = a.get(&path(&format!("{p}(GBP, USD).cross()"), None)).await;
    let after = now();
    let g = got(&r);
    assert_eq!((r.status, g.outcome()), (404, "not_covered"), "{}", r.text());
    let (call, rs) = resolved_of(&g);
    let days = [&before[..10], &after[..10]];
    assert!(days.iter().any(|d| call == format!(r#"{p}/cross(base: "GBP", date: {d}, quote: "USD")"#)), "{call}");
    assert!(rs.iter().any(|x| x.0 == "date" && x.1 == "default" && days.contains(&x.2.as_str())), "{rs:?}");

    // A date given is not defaulted: no `default` resolution.
    let g = got(&a.get(&path(&format!("{p}(GBP, USD).cross({DAY})"), Some(ASOF))).await);
    assert_eq!(resolved_of(&g).1, vec![res(&format!("{p}(...).cross"), "method", &format!("{p}/cross"))]);

    // The batch endpoint answers each call as /cell does, `resolved` included.
    let calls = [
        format!("{p}(GBP, USD).cross()?asof={ASOF}"),
        format!("{p}(GBP, USD).cross({DAY})?asof={ASOF}"),
        format!("xcross(GBP, USD)?asof={ASOF}"),
        format!("{p}/cross?base=GBP&date={DAY}&quote=USD&asof={ASOF}"),
    ];
    let body = format!(
        "[{}]",
        calls
            .iter()
            .map(|c| String::from_utf8(Value::String(c.clone()).to_jcs()).unwrap())
            .collect::<Vec<_>>()
            .join(",")
    );
    let r = a.post("/cells", body.into_bytes()).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let Value::Array(items) = r.json() else { panic!("{}", r.text()) };
    assert_eq!(items.len(), 4);
    let envs: Vec<Value> = items.iter().map(|i| i.get("envelope").cloned().unwrap()).collect();
    for e in &envs {
        assert_eq!(e.get("outcome").and_then(Value::as_str), Some("ok"), "{}", String::from_utf8_lossy(&e.to_jcs()));
        assert_eq!(e.get("receipt"), envs[3].get("receipt"));
    }
    for (i, want_call) in [(0, &full), (2, &full)] {
        assert_eq!(
            envs[i].get("resolved").and_then(|x| x.get("call")).and_then(Value::as_str),
            Some(want_call.as_str())
        );
    }
    assert!(envs[3].get("resolved").is_none());
}

// ---------------------------------------------------------------------------------------------
// Values by the type of their position

/// Every argument is read by the declared type of its position (5.8): an unquoted number in a string
/// position is that string, a quoted string in a number position, or a word, is `E_CANON_TYPE`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn arguments_are_read_by_the_type_of_their_position() {
    let (a, w) = mesh().await;
    let v = &w.verse;
    // "2" is the string "2" in the `book` position; no cell has a value: not_covered, with resolved.
    let g = got(&a.get(&path(&format!("{v}(tanakh, 2, 1, 255).text(uxlc)"), None)).await);
    assert_eq!(g.outcome(), "not_covered", "{:?}", g.env);
    let (call, _) = resolved_of(&g);
    assert_eq!(call, format!(r#"{v}/text(book: "2", chapter: 1, corpus: "tanakh", edition: "uxlc", verse: 255)"#));
    for (call, code) in [
        (format!("{v}(tanakh, Gen, \"1\", 1).text(uxlc)"), "E_CANON_TYPE"),
        (format!("{v}(tanakh, Gen, one, 1).text(uxlc)"), "E_CANON_TYPE"),
        (format!("{v}(tanakh, Gen, 1, 1).text(uxlc, extra)"), "E_ARG_UNKNOWN"),
        (format!("{v}(tanakh, Gen, 1, 1).text()"), "E_ARG_MISSING"),
        (format!("{v}(tanakh, Gen, 1).text(uxlc)"), "E_ARG_MISSING"),
        (format!("{v}(tanakh, Gen, 1, 1, 9).text(uxlc)"), "E_ARG_UNKNOWN"),
        (format!("{v}(tanakh, Gen, 1, 1).nope(uxlc)"), "E_NAME_UNBOUND"),
        (format!("{v}(tanakh, Gen, 1, 1)"), "E_PARSE_URI"),
        (format!("{v}(tanakh, Gen, 1, 1).text(uxlc).text(uxlc)"), "E_PARSE_URI"),
        (format!("{v}(tanakh, Gen, a b, 1).text(uxlc)"), "E_PARSE_URI"),
    ] {
        let g = got(&a.get(&path(&call, None)).await);
        // E_NAME_UNBOUND is a not_covered's error (5.5); the others are bad_request.
        let outcome = if code == "E_NAME_UNBOUND" { "not_covered" } else { "bad_request" };
        assert_eq!((g.outcome(), g.error()), (outcome, Some(code)), "{call}");
    }
    // An unquoted token in the older positions still means what it did: a name is not an argument.
    let g = got(&a.get(&path(&format!("{v}/text(tanakh, Gen, 1, 1, uxlc)"), None)).await);
    assert_eq!(g.outcome(), "ok", "an unquoted string in a plain read, by position: {:?}", g.env);
}

// ---------------------------------------------------------------------------------------------
// Labels, aliases

/// `N[label]` names the object the class's `lookup` maps the label to, as of the call's asof; the
/// lookup cell is `N/<lookup>(label: ...)`; `resolved` has a `lookup` resolution with its `asof`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_label_resolves_through_the_classes_lookup() {
    let (a, w) = mesh().await;
    let (nm, p) = (&w.named, &w.pair);
    let plain = got(&a.get(&format!("/cell/{nm}/rate?base=GBP&date={DAY}&quote=USD")).await);
    assert_eq!(plain.outcome(), "ok", "{:?}", plain.env);
    for call in [format!("{nm}[cable].rate({DAY})"), format!("{nm}[\"cable\"].rate({DAY})")] {
        let r = a.get(&path(&call, Some(ASOF))).await;
        let g = got(&r);
        assert_eq!(g.outcome(), "ok", "{call}: {}", r.text());
        assert_eq!((g.hash.clone(), g.cell_id.clone()), (plain.hash.clone(), plain.cell_id.clone()), "{call}");
        let (c, rs) = resolved_of(&g);
        assert_eq!(c, format!(r#"{nm}/rate(base: "GBP", date: {DAY}, quote: "USD")?asof={ASOF}"#), "{call}");
        assert_eq!(
            rs,
            vec![
                (
                    "cable".to_string(),
                    "lookup".to_string(),
                    format!(r#"{nm}(base:"GBP",quote:"USD")"#),
                    Some(ASOF.to_string())
                ),
                res(&format!("{nm}(...).rate"), "method", &format!("{nm}/rate")),
            ],
            "{call}"
        );
        let id = plain.cell_id.clone().unwrap();
        assert_eq!(check(&r, &a.keys(), &id, Some(ASOF), None), Ok("ok".to_string()), "{call}");
    }
    // Without an asof the lookup is read as of the mesh's now.
    let g = got(&a.get(&path(&format!("{nm}[cable].rate({DAY})"), None)).await);
    assert_eq!(g.outcome(), "ok");
    let rs = resolved_of(&g).1;
    let at = rs[0].3.clone().expect("the lookup's asof");
    assert!(json::parse(format!("\"{at}\"").as_bytes()).is_ok() && at.ends_with('Z') && at.len() == 27, "{at}");
    // A label the lookup has no value for, a class without lookup, and the lookup called on an object:
    // not bound (a later cell may bind it).
    for call in
        [format!("{nm}[nope].rate({DAY})"), format!("{p}[cable].rate({DAY})"), format!("{nm}(GBP, USD).named(cable)")]
    {
        let g = got(&a.get(&path(&call, Some(ASOF))).await);
        assert_eq!(
            (g.status, g.outcome(), g.error()),
            (404, "not_covered", Some("E_NAME_UNBOUND")),
            "{call}: {:?}",
            g.env
        );
    }
    // K8 (the user's decision, CONTRACT-classes-round): `N[label]` with no method, at `/cell`, answers that object's
    // `describe` with 200 and kind "object" (it was E_PARSE_URI); `resolved` says the lookup was made.
    let g = got(&a.get(&path(&format!("{nm}[cable]"), Some(ASOF))).await);
    assert_eq!((g.status, g.env.get("kind").and_then(Value::as_str)), (200, Some("object")), "{:?}", g.env);
    let rs = resolved_of(&g).1;
    assert_eq!(rs.len(), 1, "{rs:?}");
    assert_eq!(rs[0].1, "lookup", "{rs:?}");
}

/// A bare name in the function position resolves through the mesh's alias map (5.8, 16.7); an alias
/// the mesh does not have is `E_NAME_UNBOUND`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bare_name_resolves_through_the_alias_map() {
    let (a, w) = mesh().await;
    let p = &w.pair;
    let plain = got(&a.get(&format!("/cell/{p}/rate?base=EUR&date={DAY}&quote=USD")).await);
    let g = got(&a.get(&path(&format!("rate(EUR, USD, {DAY})"), None)).await);
    assert_eq!(g.hash, plain.hash);
    let (call, rs) = resolved_of(&g);
    assert_eq!(call, format!(r#"{p}/rate(base: "EUR", date: {DAY}, quote: "USD")"#));
    assert_eq!(rs, vec![res("rate", "alias", &format!("{p}/rate"))]);
    // A bare word in an argument position is a value, never an alias.
    let g = got(&a.get(&path(&format!("{p}/rate(EUR, rate, {DAY})"), None)).await);
    assert_eq!(g.outcome(), "not_covered", "{:?}", g.env);
    assert!(g.resolved().is_none());
    for call in ["nope(EUR, USD)", "@nope(EUR, USD)"] {
        let g = got(&a.get(&path(call, None)).await);
        assert_eq!((g.status, g.outcome(), g.error()), (404, "not_covered", Some("E_NAME_UNBOUND")), "{call}");
    }
}

// ---------------------------------------------------------------------------------------------
// Key normalisation and key encodings

/// A string not canonical under its key's rule is `bad_request` `E_KEY_NORMALISATION`, never
/// repaired: in a method call (the receiver is read first, normalisation included), in the plain
/// read of the method's function (its key arguments carry the rule, 16.2), and in a URI.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_key_not_canonical_under_its_rule_is_e_key_normalisation() {
    let (a, w) = mesh().await;
    let u = &w.upper;
    let ok = got(&a.get(&path(&format!("{u}(EUR, USD).rate({DAY})"), None)).await);
    assert_eq!(ok.outcome(), "ok", "{:?}", ok.env);
    let plain = got(&a.get(&format!("/cell/{u}/rate?base=EUR&date={DAY}&quote=USD")).await);
    assert_eq!(plain.hash, ok.hash);
    for call in [
        format!("{u}(eur, USD).rate({DAY})"),
        format!("{u}(EUR, usd).rate({DAY})"),
        format!("{u}(\"eur\", \"USD\").rate({DAY})"),
        // The receiver is read first: a bad key before a bad date, and before a missing argument.
        format!("{u}(EUR, usd).rate(notadate)"),
        format!("{u}(eur, USD).rate()"),
        // The plain read of the method's function.
        format!("{u}/rate(\"eur\", \"USD\", {DAY})"),
        format!("{u}/rate(base: \"EUR\", quote: \"us d\", date: {DAY})"),
    ] {
        let r = a.get(&path(&call, None)).await;
        let g = got(&r);
        assert_eq!(
            (g.status, g.outcome(), g.error()),
            (400, "bad_request", Some("E_KEY_NORMALISATION")),
            "{call}: {}",
            r.text()
        );
        assert!(g.resolved().is_none() && g.env.get("receipt").is_none());
    }
    // (The cell URI form of the plain read, `/cell/<ns>/rate?base=eur&...`, is not asserted: 5.8 gives
    // E_KEY_NORMALISATION for a call; what a URI gives is not said, see IMPL-NOTES-0.3.0.md.)
    // The normalisation of a number, a date, or a `none` key is not a thing: lower case is fine.
    let g = got(&a.get(&path(&format!("{}(eur, usd).rate({DAY})", w.pair), None)).await);
    assert_eq!(g.outcome(), "not_covered", "{:?}", g.env);
}

/// A class-typed position takes a key encoding (16.5), written as the object or as a quoted string:
/// a malformed encoding is `E_KEY_ENCODING`; a string inside it not canonical under its key's rule is
/// `E_KEY_NORMALISATION`; key names or types not the class's (or a class not declared there) are
/// `E_CANON_TYPE`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_class_typed_position_takes_a_key_encoding() {
    let (a, w) = mesh().await;
    let (u, p) = (&w.upper, &w.pair);
    let f = format!("{A}/ecb/upper_rate");
    let good = got(&a.get(&path(&format!(r#"{f}(pair: {u}(EUR, USD), date: {DAY})"#), None)).await);
    assert_eq!(good.outcome(), "ok", "{:?}", good.env);
    assert_eq!(good.env.get("receipt").and_then(|x| x.get("value")).and_then(Value::as_str), Some("1.2"));
    // The same cell as the plain read of the dispatching function with the encoding as its value.
    let enc = format!(r#"{u}(base:"EUR",quote:"USD")"#);
    let plain = got(&a.get(&format!("/cell/{f}?date={DAY}&pair={}", qenc(&enc))).await);
    assert_eq!(plain.hash, good.hash);
    assert_eq!(plain.cell_id, good.cell_id);
    // A quoted encoding is the same value, with `"` doubled.
    let quoted = format!(r#"{f}(pair: "{u}(base:""EUR"",quote:""USD"")", date: {DAY})"#);
    let g = got(&a.get(&path(&quoted, None)).await);
    assert_eq!(g.hash, good.hash, "{:?}", g.env);
    for (call, code) in [
        (format!(r#"{f}(pair: "notakey", date: {DAY})"#), "E_KEY_ENCODING"),
        (format!(r#"{f}(pair: "{u}(base:""EUR"")x", date: {DAY})"#), "E_KEY_ENCODING"),
        (format!(r#"{f}(pair: "{u}(quote:""USD"",base:""EUR"")", date: {DAY})"#), "E_KEY_ENCODING"),
        (format!(r#"{f}(pair: {u}(eur, USD), date: {DAY})"#), "E_KEY_NORMALISATION"),
        (format!(r#"{f}(pair: {p}(EUR, USD), date: {DAY})"#), "E_CANON_TYPE"),
    ] {
        let r = a.get(&path(&call, None)).await;
        let g = got(&r);
        assert_eq!((g.status, g.outcome(), g.error()), (400, "bad_request", Some(code)), "{call}: {}", r.text());
    }
}

/// H3 (5.8, 16.5): a quoted encoding in a class-typed position is decoded against its class as an
/// object written there is, so a string inside it not canonical under its key's rule is
/// `E_KEY_NORMALISATION`, a `bad_request`, not a fault at evaluation. (The call is parsed by the
/// formula crate; formula a6fd557 accepted this call and the cell then faulted `E_FORMULA_KEY`;
/// b6dd484, the c8896ad re-pin, answers it as 5.8 says.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_quoted_encoding_is_decoded_against_its_class() {
    let (a, w) = mesh().await;
    let (u, f) = (&w.upper, format!("{A}/ecb/upper_rate"));
    for call in [
        format!(r#"{f}(pair: "{u}(base:""eur"",quote:""USD"")", date: {DAY})"#),
        format!(r#"{f}(pair: "{u}(base:""EUR"",quote:""usd"")", date: {DAY})"#),
        format!(r#"{f}("{u}(base:""eur"",quote:""USD"")", {DAY})"#),
    ] {
        let r = a.get(&path(&call, None)).await;
        let g = got(&r);
        assert_eq!(
            (g.status, g.outcome(), g.error()),
            (400, "bad_request", Some("E_KEY_NORMALISATION")),
            "{call}: {}",
            r.text()
        );
    }
    // Key names that are not the class's: E_CANON_TYPE, as for a class not declared there.
    let call = format!(r#"{f}(pair: "{u}(base:""EUR"")", date: {DAY})"#);
    let g = got(&a.get(&path(&call, None)).await);
    assert_eq!((g.status, g.outcome(), g.error()), (400, "bad_request", Some("E_CANON_TYPE")), "{call}");
}

// ---------------------------------------------------------------------------------------------
// Batch

/// The batch endpoint answers calls of every form as `/cell` does: the same envelope for the same
/// request (so the same receipt for a method call and the plain read), `resolved` where something was
/// resolved, `bad_request` items in place.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_batch_endpoint_takes_calls_of_every_form() {
    let (a, w) = mesh().await;
    let (p, u) = (&w.pair, &w.upper);
    let calls: Vec<String> = vec![
        format!("{p}(EUR, USD).rate({DAY})"),
        format!("{p}/rate?base=EUR&date={DAY}&quote=USD"),
        format!("rate(EUR, USD, {DAY})"),
        format!("{u}(eur, USD).rate({DAY})"),
        format!(r#"{A}/ecb/upper_rate(pair: "notakey", date: {DAY})"#),
        format!("{}[cable].rate({DAY})?asof={ASOF}", w.named),
        format!("{p}(EUR, USD).nope({DAY})"),
    ];
    let single: Vec<Resp> = {
        let mut v = Vec::new();
        for c in &calls {
            // /cell takes the URI form as the rest, the call percent-encoded.
            let path = if c.contains('(') || c.contains('[') {
                let (call, q) = c.split_once("?asof=").map(|(a, b)| (a, Some(b))).unwrap_or((c.as_str(), None));
                path(call, q)
            } else {
                format!("/cell/{c}")
            };
            v.push(a.get(&path).await);
        }
        v
    };
    let body = format!(
        "[{}]",
        calls
            .iter()
            .map(|c| String::from_utf8(Value::String(c.clone()).to_jcs()).unwrap())
            .collect::<Vec<_>>()
            .join(",")
    );
    let r = a.post("/cells", body.into_bytes()).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let Value::Array(items) = r.json() else { panic!("{}", r.text()) };
    assert_eq!(items.len(), calls.len());
    for (i, item) in items.iter().enumerate() {
        let status = item.get("status").map(|s| String::from_utf8(s.to_jcs()).unwrap());
        assert_eq!(status.as_deref(), Some(single[i].status.to_string().as_str()), "{}", calls[i]);
        assert_eq!(item.get("envelope"), Some(&single[i].json()), "{}", calls[i]);
    }
    // The method call, the plain read of the same cell and the bare name: one receipt.
    let receipts: Vec<_> = (0..3).map(|i| items[i].get("envelope").and_then(|e| e.get("receipt")).cloned()).collect();
    assert!(receipts[0].is_some() && receipts[0] == receipts[1] && receipts[1] == receipts[2]);
    assert!(items[0].get("envelope").and_then(|e| e.get("resolved")).is_some());
    assert!(items[1].get("envelope").and_then(|e| e.get("resolved")).is_none());
    assert!(items[2].get("envelope").and_then(|e| e.get("resolved")).is_some());
    let code =
        |i: usize| items[i].get("envelope").and_then(|e| e.get("error")).and_then(Value::as_str).map(str::to_string);
    assert_eq!(code(3).as_deref(), Some("E_KEY_NORMALISATION"));
    assert_eq!(code(4).as_deref(), Some("E_KEY_ENCODING"));
    assert_eq!(code(6).as_deref(), Some("E_NAME_UNBOUND"));
}
