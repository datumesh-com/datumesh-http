//! Fixtures of the 0.7 round's catalogue tests (CONTRACT-round-0.7.md R5, R7 to R11), over `catworld` and `crworld`: families of
//! classes (a parent, a subclass, a grandchild) that are sources of one subject, withdrawals as the job writes them under R1,
//! and the metric of the pages' builds. Include after `catworld` and `crworld`:
//! `#[path = "common/r07world.rs"] mod r07world;`.
#![allow(dead_code)]

use serde_json::{json, Value};

use crate::catworld::*;
use crate::common::lite::*;
use crate::common::pub031::{class_line, function_line, BEFORE};
use crate::common::readpath::*;
use crate::common::world::class_json;
use crate::crworld::*;

/// The reason of a withdrawal of a subject or facet value that is gone (R10).
pub const GONE: &str = "the subject is no longer in the catalogue";
/// The reason of a withdrawal of a rank past the count (R10).
pub const PAST: &str = "rank past the count of sources";

/// The facets of the subject the families are sources of.
pub fn fam_facets() -> Value {
    fac(&["fam"], &["thing"], &["measure"], &["world"], &["static"])
}

/// The subject of the families: `(domain, thing, measure, region)`.
pub const FAM: (&str, &str, &str, &str) = ("fam", "thing", "measure", "world");

/// A class of a family (an empty `own_key` adds no key: a subclass that has a subclass of its own must add none, because core
/// checks an inherited method against the parent's whole key list, so a grandchild over a child that adds a key is refused with
/// `E_CLASS_METHOD`, see the report): its name, its parent (an index into the list, earlier than itself), the key it adds (a root has the
/// keys `base` and `quote`), the one method it adds, and the instant its name is bound.
pub struct Fam {
    pub name: String,
    pub parent: Option<usize>,
    pub own_key: &'static str,
    pub method: &'static str,
    pub clock: &'static str,
}

pub fn fam(name: &str, parent: Option<usize>, own_key: &'static str, method: &'static str, clock: &'static str) -> Fam {
    Fam { name: host_name(name), parent, own_key, method, clock }
}

fn keys_of(list: &[Fam], i: usize) -> Vec<String> {
    let mut k = match list[i].parent {
        Some(p) => keys_of(list, p),
        None => vec!["base".to_string(), "quote".to_string()],
    };
    if !list[i].own_key.is_empty() {
        k.push(list[i].own_key.to_string());
    }
    k
}

fn fam_sig(keys: &[String]) -> String {
    let mut args = serde_json::Map::new();
    for k in keys {
        args.insert(k.clone(), serde_json::from_str(STR).unwrap());
    }
    json!({"args": args, "order": keys, "returns": serde_json::from_str::<Value>(NUM1).unwrap()}).to_string()
}

/// A store with the classes of a family: the methods' functions at `BEFORE`, then each class with its catalogue entry (facets
/// of [`fam_facets`]) at its own instant, in list order. `more` are lines published at `BEFORE` first (relays, say).
pub fn family(label: &str, list: &[Fam], more: &[String]) -> Lite {
    let lite = Lite::new(label);
    let mut fns: Vec<String> = more.to_vec();
    for (i, c) in list.iter().enumerate() {
        fns.push(function_line(&format!("{}/{}", c.name, c.method), &fam_sig(&keys_of(list, i)), None));
    }
    publish(&lite, "functions.jsonl", &fns, BEFORE).unwrap_or_else(|e| panic!("functions: {e}"));
    for (i, c) in list.iter().enumerate() {
        let own: Vec<String> = match c.parent {
            Some(_) if c.own_key.is_empty() => vec![],
            Some(_) => vec![c.own_key.to_string()],
            None => vec!["base".to_string(), "quote".to_string()],
        };
        let keys = if c.parent.is_some() && c.own_key.is_empty() {
            "{}".to_string()
        } else if c.parent.is_some() {
            format!(r#"{{"{}":{NONE_STR}}}"#, c.own_key)
        } else {
            crate::common::world::NONE_KEYS_PAIR.to_string()
        };
        let order: Vec<&str> = own.iter().map(String::as_str).collect();
        let extra = match c.parent {
            Some(p) => {
                let h = class_hash(&lite, &list[p].name);
                format!(r#""parent":"{h}","#)
            }
            None => String::new(),
        };
        let h = fn_hash(&lite, &format!("{}/{}", c.name, c.method));
        let d = class_json(&keys, &order, &[(c.method, &h)], HOST, &extra);
        let lines =
            vec![class_line(&c.name, &d, None), catalog_line_of(&c.name, "class", json!({"facets": fam_facets()}))];
        let at: &'static str = c.clock;
        publish(&lite, &format!("class{i}.jsonl"), &lines, at).unwrap_or_else(|e| panic!("class {}: {e}", c.name));
    }
    lite
}

/// The sources of a subject in rank order, read from its cells (`sources()` and `source(r)`).
pub async fn sources_list(m: &Served, s: (&str, &str, &str, &str)) -> Vec<String> {
    let n = cell(m, &sc(s.0, s.1, s.2, s.3, "sources()")).await;
    assert_eq!((n.status, n.outcome()), (200, "ok"), "sources(): {}", n.text);
    let n: u32 = n.value().unwrap().parse().unwrap();
    let mut out = Vec::new();
    for r in 1..=n {
        out.push(source_of(m, s.0, s.1, s.2, s.3, r).await.unwrap_or_else(|| panic!("no source({r})")));
    }
    out
}

/// The count of `datumesh_catalog_builds_total{by="<by>"}`; an absent line is 0.
pub async fn builds(m: &Served, by: &str) -> u64 {
    let t = page(m, "/metrics").await.text();
    let want = format!("datumesh_catalog_builds_total{{by=\"{by}\"}}");
    t.lines().find_map(|l| l.strip_prefix(&want).map(|r| r.trim().parse::<u64>().unwrap())).unwrap_or(0)
}

/// The answer is the null version of the cell (R1): `not_covered` (404) with a withdrawal receipt (its `withdrawn` member is
/// `reason`, it has no `value`) and `reason` equal to it, `trust` primary.
#[track_caller]
pub fn assert_withdrawn(a: &Ans, reason: &str, what: &str) {
    assert_eq!((a.status, a.outcome()), (404, "not_covered"), "{what}: {}", a.text);
    assert_eq!(a.v["reason"], reason, "{what}: {}", a.text);
    let r = &a.v["receipt"];
    assert_eq!(r["withdrawn"], reason, "{what}: the receipt is a withdrawal: {}", a.text);
    assert!(r.get("value").is_none() && r.get("type").is_none(), "{what}: a withdrawal has no value: {}", a.text);
    assert_eq!(a.v["trust"], "primary", "{what}: {}", a.text);
}

/// An answer that is `ok` and not a withdrawal.
#[track_caller]
pub fn assert_ok(a: &Ans, what: &str) {
    assert_eq!((a.status, a.outcome()), (200, "ok"), "{what}: {}", a.text);
}

/// `/fn/<class name>` as the path of a request (the name's slashes are the path's).
pub fn class_path(name: &str) -> String {
    format!("/fn/{name}")
}

pub const T7: &str = "2026-11-15T12:00:07.000000Z";
pub const T8: &str = "2026-11-15T12:00:08.000000Z";
