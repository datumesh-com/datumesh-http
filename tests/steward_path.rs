//! CONTRACT-1.7a, found by the production rehearsal: a class whose steward namespace has a path
//! (`datumesh.com/scripture`, the production classes' steward) is published. Pass one's check that
//! the steward namespace is not suspended must ask the store about its host, which is what a
//! namespace row is (9.9), not hand the whole namespace to `Store::namespace`, which takes a host
//! (0.3.1 before this fix: `E_CANON_NAME at host: bad DNS label "com/scratch_1_7a"`).

mod common;

use common::lite::*;
use common::pub031::*;
use common::world::{class_json, NONE_KEYS_PAIR};
use datumesh_store::Store;

const CLASS: &str = "mesh-test.datumesh.com/scratch_1_7a/pair_path";

#[test]
fn a_class_whose_steward_namespace_has_a_path_is_published() {
    let lite = Lite::new("steward-path");
    let d = stored_descriptor(&rate_sig());
    let h = descriptor_hash(&d);
    let class = class_json(NONE_KEYS_PAIR, &["base", "quote"], &[("rate", &h)], NS, "");
    assert!(class.contains(r#""namespace":"mesh-test.datumesh.com/scratch_1_7a""#), "{class}");
    let f = write(&lite, "f.jsonl", &[descriptor_line(&d), class_line(CLASS, &class, None)]);
    let p = run(&lite, &f, NOW).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(p.classes, 1, "{p:?}");
    let b = lite.store.resolve_binding(&format!("{CLASS}/rate"), NOW).unwrap();
    assert_eq!(b.map(|b| b.fn_hash), Some(h));
}
