//! Key documents (SPEC 9.2) for the mesh's own host, rotation (SPEC 9.5), and since 0.2.1
//! their declarations (spec-0.4.3).

use datumesh_core::json::{self, Value};
use datumesh_core::{
    key_document_from_response, keys_from_response, Declaration, Declarations, Error, KeyDocument, SigningKey,
};
use datumesh_engine::Clock;
use datumesh_store::Store;

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

/// Why [`declare_at`] refused a declaration.
#[non_exhaustive]
#[derive(Debug)]
pub enum DeclareError {
    /// An explicit `valid_from` earlier than the mesh clock (rule A, CONTRACT-1.8b H1).
    StartBeforeNow {
        /// The function.
        fn_name: String,
        /// The `valid_from`.
        start: String,
        /// The mesh clock.
        now: String,
    },
    /// A `valid_to` earlier than the mesh clock (rule A).
    EndBeforeNow {
        /// The function.
        fn_name: String,
        /// The `valid_to`.
        end: String,
        /// The mesh clock.
        now: String,
    },
    /// The key document refuses the declaration (SPEC 9.2, as [`with_declaration`] says).
    Key(Error),
    /// The store refused: no key document of the host, or a write failed.
    Store(String),
}

impl std::fmt::Display for DeclareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeclareError::StartBeforeNow { fn_name, start, now } => {
                write!(f, "declaration start {start} of {fn_name} is earlier than the mesh clock {now} (rule A)")
            }
            DeclareError::EndBeforeNow { fn_name, end, now } => {
                write!(f, "declaration end {end} of {fn_name} is earlier than the mesh clock {now} (rule A)")
            }
            DeclareError::Key(e) => write!(f, "{e}"),
            DeclareError::Store(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for DeclareError {}

/// As [`with_declaration`], for the store's key document of `host` (since CONTRACT-1.8b H1): the
/// declaration is written with `put_namespace` at the clock's now. Rule A: an explicit
/// `valid_from` or `valid_to` earlier than the clock's now is refused, with nothing written; with
/// no `valid_from` it is now. The document is read from `store`, which must hold one.
pub fn declare_at<S: Store + ?Sized>(
    store: &S,
    host: &str,
    fn_name: &str,
    licence: &str,
    valid_from: Option<&str>,
    valid_to: Option<&str>,
    clock: &dyn Clock,
) -> Result<Declared, DeclareError> {
    let now = clock.now();
    if let Some(start) = valid_from.filter(|f| *f < now.as_str()) {
        return Err(DeclareError::StartBeforeNow { fn_name: fn_name.into(), start: start.into(), now });
    }
    if let Some(end) = valid_to.filter(|t| *t < now.as_str()) {
        return Err(DeclareError::EndBeforeNow { fn_name: fn_name.into(), end: end.into(), now });
    }
    let existing = store
        .namespace(host)
        .map_err(|e| DeclareError::Store(e.to_string()))?
        .ok_or_else(|| DeclareError::Store(format!("the store holds no key document of {host}")))?
        .keydoc;
    let from = valid_from.unwrap_or(now.as_str());
    let (doc, what) = with_declaration(host, &existing, fn_name, licence, from, valid_to).map_err(DeclareError::Key)?;
    if what != Declared::Unchanged {
        store.put_namespace(host, &doc, &now).map_err(|e| DeclareError::Store(e.to_string()))?;
    }
    Ok(what)
}

/// The key document `existing` of `host` with the key `public` registered from `valid_from` (QUESTIONS-build2
/// D39): the key is added beside the keys there are, and no key is rotated or closed. `None` when the document
/// holds that key already (then nothing is to be written). The declarations are kept. Checked by 9.2.
pub fn with_registered(
    host: &str,
    existing: &[u8],
    public: &[u8; 32],
    valid_from: &str,
) -> Result<Option<Vec<u8>>, Error> {
    keys_from_response(host, 200, Some(existing))?;
    let v = json::parse(existing)?;
    let id = datumesh_core::key_id(public);
    let mut keys: Vec<Value> = match v.get("keys") {
        Some(Value::Array(ks)) => ks.clone(),
        _ => Vec::new(),
    };
    if keys.iter().any(|k| k.get("id").and_then(Value::as_str) == Some(id.as_str())) {
        return Ok(None);
    }
    keys.push(Value::Object(vec![
        ("alg".into(), Value::String("ed25519".into())),
        ("id".into(), Value::String(id)),
        ("public".into(), Value::String(datumesh_core::lexical::to_hex(public))),
        ("valid_from".into(), Value::String(valid_from.to_string())),
    ]));
    let declarations = v.get("declarations").cloned();
    document(host, keys, declarations).map(Some)
}
