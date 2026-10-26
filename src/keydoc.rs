//! Key documents (SPEC 9.2) for the mesh's own host, and rotation (SPEC 9.5).

use datumesh_core::json::{self, Value};
use datumesh_core::{keys_from_response, Error, SigningKey};

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
    let doc = Value::Object(vec![
        ("host".into(), Value::String(host.to_string())),
        ("keys".into(), Value::Array(keys)),
        ("proto".into(), Value::String("datumesh/1".into())),
    ])
    .to_jcs();
    keys_from_response(host, 200, Some(&doc))?;
    Ok(doc)
}
