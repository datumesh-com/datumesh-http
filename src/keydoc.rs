//! Key documents (SPEC 9.2) for the mesh's own host, rotation (SPEC 9.5), and since 0.2.1
//! their declarations (spec-0.4.3).

use datumesh_core::json::{self, Value};
use datumesh_core::{
    key_document_from_response, keys_from_response, Declaration, Declarations, Error, KeyDocument, SigningKey,
};

/// The key document of `host` publishing `key` from `valid_from`, after the keys of
/// `existing` (a document already published for `host`, if any): rotation as SPEC 9.5 says,
/// each existing key without a `valid_to` (or with a later one) ending at `valid_from`. A key
/// already in `existing` is not added again. The result is canonical JSON, checked by 9.2.
pub fn with_key(host: &str, existing: Option<&[u8]>, key: &SigningKey, valid_from: &str) -> Result<Vec<u8>, Error> {
    let public = datumesh_core::lexical::to_hex(&key.public_key());
    let mut keys: Vec<Value> = Vec::new();
    let mut present = false;
    if let Some(doc) = existing {
        keys_from_response(host, 200, Some(doc))?;
        let v = json::parse(doc)?;
        if let Some(Value::Array(ks)) = v.get("keys") {
            for k in ks {
                let Value::Object(members) = k else { continue };
                if k.get("id").and_then(Value::as_str) == Some(key.key_id()) {
                    present = true;
                    keys.push(k.clone());
                    continue;
                }
                let ends_later = match k.get("valid_to").and_then(Value::as_str) {
                    None => true,
                    Some(t) => t > valid_from,
                };
                let starts_before = k.get("valid_from").and_then(Value::as_str).is_some_and(|f| f < valid_from);
                if ends_later && starts_before {
                    let mut m: Vec<(String, Value)> =
                        members.iter().filter(|(n, _)| n != "valid_to").cloned().collect();
                    m.push(("valid_to".into(), Value::String(valid_from.to_string())));
                    keys.push(Value::Object(m));
                } else {
                    keys.push(k.clone());
                }
            }
        }
    }
    if !present {
        keys.push(Value::Object(vec![
            ("alg".into(), Value::String("ed25519".into())),
            ("id".into(), Value::String(key.key_id().to_string())),
            ("public".into(), Value::String(public)),
            ("valid_from".into(), Value::String(valid_from.to_string())),
        ]));
    }
    // Since 0.2.1: the existing document's `declarations` (spec-0.4.3) are kept as they are.
    let declarations = match existing {
        Some(doc) => json::parse(doc)?.get("declarations").cloned(),
        None => None,
    };
    document(host, keys, declarations)
}

/// The canonical key document of `host` from its `keys` and optional `declarations`
/// members, checked by 9.2 (declarations included).
fn document(host: &str, keys: Vec<Value>, declarations: Option<Value>) -> Result<Vec<u8>, Error> {
    let mut members = Vec::new();
    if let Some(d) = declarations {
        members.push(("declarations".into(), d));
    }
    members.push(("host".into(), Value::String(host.to_string())));
    members.push(("keys".into(), Value::Array(keys)));
    members.push(("proto".into(), Value::String("datumesh/1".into())));
    let doc = Value::Object(members).to_jcs();
    key_document_from_response(host, 200, Some(&doc))?;
    Ok(doc)
}

/// What [`with_declaration`] did to the key document.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Declared {
    /// The entry was added.
    Added,
    /// An open entry with the same `licence` and `valid_from` was closed at `valid_to`.
    Closed,
    /// The document already has exactly this entry: unchanged (the same bytes).
    Unchanged,
}

/// The key document `existing` of `host` with a declaration (SPEC 9.2, spec-0.4.3): `fn_name`
/// (a full function name under `host`) may be computed over inputs under the restricted
/// `licence` from `valid_from` (an instant) until `valid_to` (absent: open-ended), the result
/// carrying that licence (SPEC 8.5). Since 0.2.1.
///
/// Entries are never removed, only closed: an entry exactly as given leaves the document
/// unchanged (so a declaration step can run again); an open entry with the same `licence` and
/// `valid_from`, given a `valid_to`, is closed then; any other entry with the same `licence`
/// and `valid_from` is refused (`E_KEY_DOC`), as is anything 9.2 rejects (a name under another
/// host, a bad licence or instant, more than 64 names or 16 entries). The keys are kept as
/// they are. The result is canonical JSON, checked by 9.2.
pub fn with_declaration(
    host: &str,
    existing: &[u8],
    fn_name: &str,
    licence: &str,
    valid_from: &str,
    valid_to: Option<&str>,
) -> Result<(Vec<u8>, Declared), Error> {
    let doc = KeyDocument::from_json(host, existing)?;
    let entry = Declaration::new(licence, valid_from, valid_to)?;
    let mut declarations = doc.declarations.unwrap_or_default();
    let same_start = declarations.get(fn_name).iter().position(|e| e.licence == licence && e.valid_from == valid_from);
    let (declarations, what) = match same_start {
        Some(i) if declarations.get(fn_name)[i] == entry => return Ok((existing.to_vec(), Declared::Unchanged)),
        Some(i) if declarations.get(fn_name)[i].valid_to.is_none() && valid_to.is_some() => {
            // Rebuilt with that entry closed, every other entry as it was.
            let mut out = Declarations::new();
            for (name, entries) in declarations.iter() {
                for (j, e) in entries.iter().enumerate() {
                    let e = if name == fn_name && j == i { entry.clone() } else { e.clone() };
                    out.insert(name, e)?;
                }
            }
            (out, Declared::Closed)
        }
        Some(_) => {
            return Err(Error::new(
                datumesh_core::ErrorCode::KeyDoc,
                "$.declarations",
                format!("{fn_name} already has an entry for {licence} from {valid_from}: entries are never removed, only closed"),
            ))
        }
        None => {
            declarations.insert(fn_name, entry)?;
            (declarations, Declared::Added)
        }
    };
    let keys = match json::parse(existing)?.get("keys") {
        Some(Value::Array(k)) => k.clone(),
        _ => Vec::new(),
    };
    Ok((document(host, keys, Some(declarations.to_value()))?, what))
}
