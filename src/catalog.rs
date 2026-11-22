//! The publisher's catalogue (CONTRACT-build2-part2-read R5): the `catalog` line of `publish-stored`
//! and the `catalog` member of bundle 1 have one shape, checked here before anything is written:
//! a kind (`function` or `class`), a label (at most 120 bytes), a description (1 to 1024 bytes),
//! the arguments' texts, the examples (a JSON array) and the dictionaries (for each key, values with
//! labels and optional aliases). The store keeps them with `source=publisher`.

use std::collections::BTreeMap;

use datumesh_core::json::Value;
use datumesh_core::lexical::check_name;
use datumesh_store::{CatalogEntry, CatalogKind, CatalogSource, Facets, KeyValue};

/// The keys a facet may carry (C2): the five, and no other.
const FACET_KEYS: [&str; 5] = ["domain", "thing", "measure", "region", "frequency"];

/// The most names a facet lists, and the most bytes of one (C2).
const FACET_MAX_VALUES: usize = 8;
const FACET_VALUE_BYTES: usize = 32;

/// The facets of an entry (C2): an object of at most the five keys, each a list of 1 to 8 names (2.2, at most 32 bytes),
/// ascending and unique. Absent, none.
fn parse_facets(v: Option<&Value>) -> Result<Facets, String> {
    let mut out = Facets::new();
    let obj = match v {
        None => return Ok(out),
        Some(Value::Object(o)) => o,
        Some(_) => return Err("facets is not an object".into()),
    };
    for (k, list) in obj {
        if !FACET_KEYS.contains(&k.as_str()) {
            return Err(format!("facets.{k}: not a facet (domain, thing, measure, region or frequency)"));
        }
        let Value::Array(items) = list else { return Err(format!("facets.{k} is not a list")) };
        if items.is_empty() || items.len() > FACET_MAX_VALUES {
            return Err(format!("facets.{k} has 1 to {FACET_MAX_VALUES} names"));
        }
        let mut names: Vec<String> = Vec::with_capacity(items.len());
        for x in items {
            let n = x.as_str().ok_or_else(|| format!("facets.{k}: a name is not a string"))?;
            if n.len() > FACET_VALUE_BYTES {
                return Err(format!("facets.{k}: a name has at most {FACET_VALUE_BYTES} bytes"));
            }
            check_name(n, "facets").map_err(|e| format!("facets.{k}: {e}"))?;
            names.push(n.to_string());
        }
        if names.windows(2).any(|w| w[0] >= w[1]) {
            return Err(format!("facets.{k}: the names are not ascending and unique"));
        }
        out.insert(k.clone(), names);
    }
    Ok(out)
}

/// The most bytes of a label.
pub(crate) const LABEL_BYTES: usize = 120;

/// The most bytes of a description.
pub(crate) const DESCRIPTION_BYTES: usize = 1024;

/// A parsed catalogue entry: what the store is given.
pub(crate) struct Described {
    pub(crate) kind: CatalogKind,
    pub(crate) name: String,
    pub(crate) label: String,
    pub(crate) description: String,
    pub(crate) args: BTreeMap<String, String>,
    pub(crate) examples: String,
    pub(crate) dictionary: Vec<(String, Vec<KeyValue>)>,
    /// The keys open from the first cell (`open_keys`, CONTRACT-0.4.9 H7): no automatic values for them.
    pub(crate) open_keys: Vec<String>,
    /// The class a function extends (`extends`, K9, the classes round): a function's entry only.
    pub(crate) extends: Option<String>,
    /// The facets (C2): none when the line has none, which removes the entry's facets.
    pub(crate) facets: Facets,
}

fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key).and_then(Value::as_str).ok_or_else(|| format!("{key} is not a string"))
}

/// Parses the entry `v`, whose name is the member `name_key` (`catalog` in a line, `name` in a
/// bundle's catalogue). Errors name the member that breaks its bound.
pub(crate) fn parse(v: &Value, name_key: &str) -> Result<Described, String> {
    let name = text(v, name_key)?.to_string();
    let kind = match v.get("kind").and_then(Value::as_str) {
        Some("function") => CatalogKind::Function,
        Some("class") => CatalogKind::Class,
        _ => return Err("kind is not function or class".into()),
    };
    let label = text(v, "label")?.to_string();
    if label.len() > LABEL_BYTES {
        return Err(format!("a label has at most {LABEL_BYTES} bytes"));
    }
    let description = text(v, "description")?.to_string();
    if description.is_empty() || description.len() > DESCRIPTION_BYTES {
        return Err(format!("a description has 1 to {DESCRIPTION_BYTES} bytes"));
    }
    let mut args = BTreeMap::new();
    match v.get("args") {
        None => {}
        Some(Value::Object(m)) => {
            for (k, x) in m {
                check_name(k, "args").map_err(|e| format!("args: {e}"))?;
                let t = x.as_str().ok_or_else(|| format!("args.{k} is not a string"))?;
                if t.len() > DESCRIPTION_BYTES {
                    return Err(format!("args.{k} has at most {DESCRIPTION_BYTES} bytes"));
                }
                args.insert(k.clone(), t.to_string());
            }
        }
        Some(_) => return Err("args is not an object".into()),
    }
    let examples = match v.get("examples") {
        None => "[]".to_string(),
        Some(e @ Value::Array(_)) => String::from_utf8(e.to_jcs()).map_err(|e| e.to_string())?,
        Some(_) => return Err("examples is not an array".into()),
    };
    if examples.len() > 65536 {
        return Err("examples have at most 65536 bytes of JSON".into());
    }
    let mut dictionary = Vec::new();
    match v.get("dictionary") {
        None => {}
        Some(Value::Object(m)) => {
            for (key, vals) in m {
                check_name(key, "dictionary").map_err(|e| format!("dictionary: {e}"))?;
                let Value::Array(vals) = vals else { return Err(format!("dictionary.{key} is not an array")) };
                let mut out = Vec::new();
                for d in vals {
                    let value = text(d, "value").map_err(|e| format!("dictionary.{key}: {e}"))?;
                    if value.is_empty() || value.len() > DESCRIPTION_BYTES {
                        return Err(format!("dictionary.{key}: a value has 1 to {DESCRIPTION_BYTES} bytes"));
                    }
                    let label = text(d, "label").map_err(|e| format!("dictionary.{key}: {e}"))?;
                    if label.len() > LABEL_BYTES {
                        return Err(format!("dictionary.{key}: a label has at most {LABEL_BYTES} bytes"));
                    }
                    let mut aliases = Vec::new();
                    match d.get("aliases") {
                        None => {}
                        Some(Value::Array(a)) => {
                            for x in a {
                                let s =
                                    x.as_str().ok_or_else(|| format!("dictionary.{key}: an alias is not a string"))?;
                                if s.is_empty() || s.len() > LABEL_BYTES {
                                    return Err(format!("dictionary.{key}: an alias has 1 to {LABEL_BYTES} bytes"));
                                }
                                aliases.push(s.to_string());
                            }
                        }
                        Some(_) => return Err(format!("dictionary.{key}: aliases is not an array")),
                    }
                    out.push(KeyValue { value: value.to_string(), label: label.to_string(), aliases });
                }
                dictionary.push((key.clone(), out));
            }
        }
        Some(_) => return Err("dictionary is not an object".into()),
    }
    let mut open_keys = Vec::new();
    match v.get("open_keys") {
        None => {}
        Some(Value::Array(ks)) => {
            for k in ks {
                let k = k.as_str().ok_or("open_keys: a key is not a string")?;
                check_name(k, "open_keys").map_err(|e| format!("open_keys: {e}"))?;
                open_keys.push(k.to_string());
            }
        }
        Some(_) => return Err("open_keys is not an array".into()),
    }
    let extends = match v.get("extends") {
        None => None,
        Some(Value::String(c)) => {
            if !matches!(kind, CatalogKind::Function) {
                return Err("extends is for a function's entry".into());
            }
            datumesh_core::lexical::check_function_name(c, "extends").map_err(|e| format!("extends: {e}"))?;
            Some(c.clone())
        }
        Some(_) => return Err("extends is a class's full name".into()),
    };
    let facets = parse_facets(v.get("facets")).map_err(|e| format!("facets: {e}"))?;
    Ok(Described { kind, name, label, description, args, examples, dictionary, open_keys, extends, facets })
}

impl Described {
    /// The store's entry for this text, with its function or class hash and the signer.
    pub(crate) fn entry(&self, hash: Option<String>, source: CatalogSource, signed_by: Option<String>) -> CatalogEntry {
        CatalogEntry {
            kind: self.kind,
            name: self.name.clone(),
            hash,
            label: self.label.clone(),
            description: self.description.clone(),
            args: self.args.clone(),
            examples: self.examples.clone(),
            source,
            signed_by,
        }
    }
}

/// Stores a publisher's entry and its dictionaries (R5), writing only what differs from the store:
/// the same text again writes nothing, so a second run of a file changes no byte of the store.
pub(crate) fn store_publisher<S: datumesh_store::Store + ?Sized>(
    store: &S,
    entry: &CatalogEntry,
    dictionaries: &[(String, Vec<KeyValue>)],
    open_keys: &[String],
    extends: Option<&str>,
    facets: &Facets,
) -> Result<(), String> {
    // The line is the publisher's whole statement of the entry: its facets are the line's (C2), so a line without
    // facets removes them.
    let old = store.catalog_entry_ext(entry.kind, &entry.name).map_err(|e| e.to_string())?;
    let same = old.as_ref().is_some_and(|old| {
        old.entry.kind == entry.kind
            && old.entry.hash == entry.hash
            && old.entry.label == entry.label
            && old.entry.description == entry.description
            && old.entry.args == entry.args
            && old.entry.examples == entry.examples
            && old.entry.source == entry.source
            && old.entry.signed_by == entry.signed_by
            && old.extends.as_deref() == extends
            && old.facets == *facets
    });
    if !same {
        let mut ext = datumesh_store::CatalogEntryExt::new(entry.clone());
        ext.facets = facets.clone();
        ext.extends = extends.map(str::to_string);
        store.upsert_catalog_ext(std::slice::from_ref(&ext)).map_err(|e| e.to_string())?;
    }
    // The keys the entry names are open from now: no automatic values for them (CONTRACT-0.4.9 H7).
    for key in open_keys {
        store.mark_key_open(&entry.name, key).map_err(|e| e.to_string())?;
    }
    for (key, values) in dictionaries {
        let fresh: Vec<KeyValue> = values
            .iter()
            .filter(|kv| {
                // A value the store does not hold is new (the exact lookup); one it holds is compared with its label.
                !crate::find::has_value(store, &entry.name, key, &kv.value)
                    || crate::find::value_row(store, &entry.name, key, &kv.value)
                        .is_none_or(|(label, aliases)| label != kv.label || aliases != kv.aliases)
            })
            .cloned()
            .collect();
        if !fresh.is_empty() {
            store.upsert_key_values(&entry.name, key, &fresh).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
