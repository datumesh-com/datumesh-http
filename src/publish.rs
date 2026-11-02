//! Publishing stored cells as the mesh (`datumesh-mesh publish-stored`): a JSON lines file of
//! stored functions and their cells, signed with the mesh's key, put in the store.
//!
//! Each line is one of:
//!
//! * a function: `{"function": "<ns>/<name>", "signature": {...}, "bound": "<instant>"}`: the
//!   descriptor `{"code": {"body": "", "lang": "stored"}, "inputs": [], "signature": ...}`
//!   is put and the name bound to it from `bound` (unless it already is);
//! * a cell: `{"cell": {"fn": "<ns>/<name>", "args": {<name>: <typed scalar>}}, "value": <typed
//!   scalar> or {"blob": "<file>", "media_type": "<type>"}, "tx", "asof", "issued"?, "licence",
//!   "summary"}`: a blob's file (relative to the JSON lines file) is put with `put_blob` and the
//!   value is `<sha256>:<length>:<media type>`; the receipt is signed with the mesh's key
//!   (`computed_by` the mesh host, `publisher` the cell's namespace, `valid` its valid-time
//!   arguments; `issued` defaults to the later of `asof` and the key's start) and stored with
//!   `put_receipt` and `put_version`.
//!
//! The function of a cell is the one its name is bound to as of the cell's `tx`. Re-running a
//! file changes nothing: descriptors and blobs are content-addressed, signing is deterministic
//! (the same receipt, the same hash), and a version already there is skipped. The key is never
//! logged.

use std::io::BufRead;
use std::path::Path;

use datumesh_core::json::{self, Value};
use datumesh_core::{Arg, Args, Cell, FunctionDescriptor, ReceiptBuilder, Scalar, SigningKey, Type};
use datumesh_store::Store;

/// What a run put.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Published {
    /// Functions newly bound.
    pub functions: usize,
    /// Cell lines read.
    pub cells: usize,
    /// Receipts stored (new or already there).
    pub receipts: usize,
    /// Versions newly added.
    pub versions: usize,
    /// Blobs put (new or already there).
    pub blobs: usize,
    /// Blob bytes put.
    pub blob_bytes: u64,
}

/// Why a run stopped: the line and what was wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishError {
    /// The line number (1-based; 0 for the file).
    pub line: usize,
    /// Why.
    pub detail: String,
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.detail)
    }
}

impl std::error::Error for PublishError {}

fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

/// The receipt JSON of a signed receipt: its payload's members and `sig`.
pub fn receipt_json(r: &datumesh_core::Receipt) -> Vec<u8> {
    let mut members = vec![("sig".to_string(), Value::String(r.sig().to_string()))];
    if let Ok(Value::Object(m)) = json::parse(r.payload()) {
        members.extend(m);
    }
    Value::Object(members).to_jcs()
}

/// Publishes the file at `path` into `store` as the mesh `host` signing with `key` (see the
/// module documentation).
pub fn publish_stored<S: Store + ?Sized>(
    store: &S,
    key: &SigningKey,
    host: &str,
    path: &Path,
) -> Result<Published, PublishError> {
    let file =
        std::fs::File::open(path).map_err(|e| PublishError { line: 0, detail: format!("{}: {e}", path.display()) })?;
    let base = path.parent().unwrap_or(Path::new("."));
    let mut out = Published::default();
    // The key's start: the earliest `issued`.
    let key_from = store
        .namespace(host)
        .ok()
        .flatten()
        .and_then(|n| json::parse(&n.keydoc).ok())
        .and_then(|d| match d.get("keys") {
            Some(Value::Array(ks)) => ks
                .iter()
                .find(|k| k.get("id").and_then(Value::as_str) == Some(key.key_id()))
                .and_then(|k| k.get("valid_from").and_then(Value::as_str).map(str::to_string)),
            _ => None,
        })
        .ok_or_else(|| PublishError {
            line: 0,
            detail: format!("the key {} is not in {host}'s key document in the store", key.key_id()),
        })?;
    for (i, line) in std::io::BufReader::new(file).lines().enumerate() {
        let n = i + 1;
        let err = |d: String| PublishError { line: n, detail: d };
        let line = line.map_err(|e| err(e.to_string()))?;
        if line.trim().is_empty() {
            continue;
        }
        let v = json::parse(line.as_bytes()).map_err(|e| err(e.to_string()))?;
        if let Some(name) = v.get("function").and_then(Value::as_str) {
            let sig = v.get("signature").ok_or_else(|| err("a function without a signature".into()))?;
            let bound = v.get("bound").and_then(Value::as_str).ok_or_else(|| err("a function without bound".into()))?;
            let d = Value::Object(vec![
                ("code".into(), Value::Object(vec![("body".into(), s("")), ("lang".into(), s("stored"))])),
                ("inputs".into(), Value::Array(vec![])),
                ("signature".into(), sig.clone()),
            ]);
            let fd = FunctionDescriptor::from_value(&d).map_err(|e| err(e.to_string()))?;
            let h = store.put_descriptor(&fd.canonical()).map_err(|e| err(e.to_string()))?;
            match store.resolve_binding(name, "9999-12-31T23:59:59.999999Z").map_err(|e| err(e.to_string()))? {
                Some(b) if b.fn_hash == h => {}
                _ => {
                    store.bind_name(name, &h, bound).map_err(|e| err(format!("bind {name}: {e}")))?;
                    out.functions += 1;
                }
            }
            continue;
        }
        let cell_v = v.get("cell").ok_or_else(|| err("neither a function nor a cell".into()))?;
        let name = cell_v.get("fn").and_then(Value::as_str).ok_or_else(|| err("a cell without fn".into()))?;
        let (ns, f) = name.rsplit_once('/').ok_or_else(|| err(format!("{name}: not a function name")))?;
        let field =
            |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| err(format!("no {k}")));
        let (tx, asof, licence, summary) = (field("tx")?, field("asof")?, field("licence")?, field("summary")?);
        let issued = v
            .get("issued")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| std::cmp::max(asof.clone(), key_from.clone()));
        let fn_hash = store
            .resolve_name(name, &tx)
            .map_err(|e| err(e.to_string()))?
            .ok_or_else(|| err(format!("{name} is not bound as of {tx}")))?;
        let d = store.descriptor(&fn_hash).ok_or_else(|| err(format!("no descriptor {fn_hash}")))?;
        let d = FunctionDescriptor::from_json(&d).map_err(|e| err(e.to_string()))?;
        let mut args = Args::new();
        for (k, a) in cell_v.get("args").and_then(Value::as_object).ok_or_else(|| err("a cell without args".into()))? {
            args.insert(k.clone(), Arg::Value(Scalar::from_json(&a.to_jcs()).map_err(|e| err(format!("{k}: {e}")))?));
        }
        let valid: Args = args
            .iter()
            .filter(|(k, _)| d.signature.args.get(*k).is_some_and(|x| x.valid))
            .map(|(k, a)| (k.clone(), a.clone()))
            .collect();
        let value_v = v.get("value").ok_or_else(|| err("a cell without value".into()))?;
        let value = match value_v.get("blob").and_then(Value::as_str) {
            Some(file) => {
                let media = value_v.get("media_type").and_then(Value::as_str).unwrap_or("application/octet-stream");
                let bytes = std::fs::read(base.join(file)).map_err(|e| err(format!("{file}: {e}")))?;
                let sha = store.put_blob(&bytes).map_err(|e| err(e.to_string()))?;
                out.blobs += 1;
                out.blob_bytes += bytes.len() as u64;
                Scalar::from_lexical(Type::Blob, &format!("{sha}:{}:{media}", bytes.len()))
                    .map_err(|e| err(e.to_string()))?
            }
            None => Scalar::from_json(&value_v.to_jcs()).map_err(|e| err(e.to_string()))?,
        };
        let cell = Cell { ns: ns.to_string(), fn_name: f.to_string(), fn_hash, args };
        let r = ReceiptBuilder::new()
            .cell(cell.clone())
            .value(value)
            .valid(valid)
            .asof(asof.as_str())
            .tx(tx.as_str())
            .issued(issued.as_str())
            .publisher(ns)
            .signer_host(host)
            .licence(licence.as_str())
            .summary(summary.as_str())
            .sign(key)
            .map_err(|e| err(e.to_string()))?;
        let h = store.put_receipt(&receipt_json(&r)).map_err(|e| err(e.to_string()))?;
        out.cells += 1;
        out.receipts += 1;
        let id = cell.id();
        match store.latest(&id).map_err(|e| err(e.to_string()))? {
            Some(l) if l.tx.as_str() >= tx.as_str() => {
                if store.version_asof(&id, &tx).ok().flatten().is_none_or(|v| v.receipt_hash != h) {
                    return Err(err(format!("{id} has a version at or after {tx} that is not this receipt")));
                }
            }
            _ => {
                store.put_version(&id, &tx, &h).map_err(|e| err(e.to_string()))?;
                out.versions += 1;
            }
        }
    }
    Ok(out)
}
