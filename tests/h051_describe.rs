//! http 0.5.1, CONTRACT-0.5.1 H5: `describe` of a class walks the parent chain once per request, not once per method,
//! using store 0.6.4's index on `class_bindings(class_hash, class_name, tx)` for the defining class's name.
//! Written first; FAIL until then.
//!
//! A class of three levels over a root with 20 methods (the root defines them all; each level adds a method of its own): the leaf's `describe` answers, for each of its 23 methods, the function it denotes (`D/m` for the class D
//! that defines it). Counted with row counters on `classes` and `class_bindings` (`common/h051.rs`): 0.5.0 walks the
//! chain for every method (60 reads of `classes` and a name lookup in `class_bindings` for each of the 20 inherited
//! methods); one walk reads the chain's classes once and names each defining class once.
//!
//! The bounds are generous in what a walk may cost (the table scans of a small database count every row they visit):
//! `classes` 16 rows, `class_bindings` 24 rows, for the whole request.

mod common;
#[path = "common/h0410.rs"]
mod h0410;
#[path = "common/h051.rs"]
mod h051;

use common::lite::*;
use common::pub031::{BEFORE, T1, T2};
use h0410::*;
use serde_json::{json, Value};

const METHODS: usize = 20;

fn steward() -> Value {
    json!({"namespace": HOST, "process": format!("https://{HOST}/classes/change")})
}

fn key(name: &str) -> Value {
    json!({name: {"key_normalisation": "none", "type": "string"}})
}

fn sig(args: &[&str], returns: Value) -> Value {
    let a: serde_json::Map<String, Value> =
        args.iter().map(|k| (k.to_string(), json!({"type": "string", "valid": false}))).collect();
    json!({"args": a, "order": args, "returns": returns})
}

fn class(name: &str, keys: Value, order: &[&str], methods: Value, parent: Option<&str>) -> String {
    let mut d = json!({"keys": keys, "methods": methods, "order": order, "steward": steward()});
    if let Some(p) = parent {
        d["parent"] = json!(p);
    }
    json!({"class": name, "descriptor": d}).to_string()
}

fn c(i: usize) -> String {
    format!("{HOST}/h051/c{i}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h5_a_class_of_twenty_methods_and_three_parents_is_one_walk() {
    let fx = Fx::pg("h051-describe");
    let num = json!({"type": "number", "unit": "1"});
    let mut root_methods = serde_json::Map::new();
    for i in 0..METHODS {
        let x = format!("x{i:02}");
        root_methods.insert(format!("m{i:02}"), json!({"stored": sig(&["base", "quote", &x], num.clone())}));
    }
    let mut keys = key("base");
    keys["quote"] = key("quote")["quote"].clone();
    fx.publish(&[class(&c(0), keys, &["base", "quote"], Value::Object(root_methods), None)], BEFORE);
    // each level: no key more (a method the parent has takes all the parent's keys, 16.2 step 6), one method of its own,
    // the class above as parent
    let levels: [&'static str; 3] = [NOW, T1, T2];
    for (i, clock) in levels.iter().enumerate() {
        let parent = fx.class_hash(&c(i));
        let own = json!({format!("own{}", i + 1): {"stored": sig(&["base", "quote", "y"], num.clone())}});
        fx.publish(&[class(&c(i + 1), json!({}), &[], own, Some(&parent))], clock);
    }
    fx.probe().arm();
    h051::arm_tables(&fx, "p051_classes", &["classes"]);
    h051::arm_tables(&fx, "p051_bindings", &["class_bindings"]);
    let w = fx.serve(h051::cfg(&[], &[])).await;
    let leaf = c(3);
    for round in ["first", "second"] {
        let (k0, b0) = (h051::reads_of(&w.fx, "p051_classes"), h051::reads_of(&w.fx, "p051_bindings"));
        let r = w.get(&format!("/describe/{leaf}")).await;
        let (k1, b1) = (h051::reads_of(&w.fx, "p051_classes"), h051::reads_of(&w.fx, "p051_bindings"));
        assert_eq!(r.status, 200, "{}", r.text());
        let v = r.json();
        let ms = v["methods"].as_object().unwrap_or_else(|| panic!("{v}"));
        assert_eq!(ms.len(), METHODS + 3, "{v}");
        for i in 0..METHODS {
            assert_eq!(ms[&format!("m{i:02}")]["fn"], format!("{}/m{i:02}", c(0)), "m{i:02} is the root's");
        }
        for i in 1..=3 {
            assert_eq!(ms[&format!("own{i}")]["fn"], format!("{}/own{i}", c(i)), "own{i} is its level's");
        }
        assert!(k1 - k0 > 0 || b1 - b0 > 0, "the {round} describe read neither table: the counters are not armed");
        assert!(
            k1 - k0 <= 16,
            "the {round} describe of the leaf visited {} rows of classes; one walk of a chain of 3 is at most 16",
            k1 - k0
        );
        assert!(
            b1 - b0 <= 24,
            "the {round} describe of the leaf visited {} rows of class_bindings; at most 24",
            b1 - b0
        );
    }
}
