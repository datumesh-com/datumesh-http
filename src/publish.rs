//! Publishing stored cells as the mesh (`datumesh-mesh publish-stored`): a JSON lines file of
//! stored functions and their cells, signed with the mesh's key, put in the store.
//!
//! Each line is one of:
//!
//! * a function: `{"function": "<ns>/<name>", "signature": {...}, "bound": "<instant>"}`: the
//!   descriptor `{"code": {"body": "", "lang": "stored"}, "inputs": [], "signature": ...}`
//!   is put and the name bound to it from `bound` (unless it already is);
//! * a formula function: `{"formula": "<ns>/<name>", "source": "<formula>", "signature":
//!   {...}, "bound": "<instant>"}`: defined with `datumesh_formula::definition::define_function`
//!   (its callees as bound at `bound`) and published (`put_descriptor`, `bind_name` from
//!   `bound`) unless the name is already bound to that function; its cells are evaluated on
//!   demand by the mesh;
//! * a class (since 0.3.0, spec-0.5.0, 16.2): `{"class": "<ns>/<name>", "descriptor": {...as
//!   written...}, "bound": "<instant>"}`: defined with the formula crate's `define_class` (core's
//!   rules: the class is under one of the mesh's own namespaces, else `E_CLASS_OWNER`; the method
//!   functions are those bound as of `bound`), `put_class`, and `bind_class` from `bound` unless the
//!   name is already bound to that class hash. Method functions come first in the file (or in an
//!   earlier one), `bound` later than theirs.
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
use datumesh_core::{Arg, Args, Cell, ClassDescriptor, FunctionDescriptor, ReceiptBuilder, Scalar, SigningKey, Type};
use datumesh_formula::definition::ClassCallee;
use datumesh_store::Store;

/// What a run put.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Published {
    /// Functions newly bound (stored and formula).
    pub functions: usize,
    /// Classes newly bound (since 0.3.0).
    pub classes: usize,
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

/// A store as the callees of a definition: a name's binding as of an instant, with its
/// descriptor; a class name's binding (since 0.3.0); the classes and functions known by hash.
struct StoreCallees<'a, S: Store + ?Sized>(&'a S);

impl<S: Store + ?Sized> datumesh_formula::definition::Callees for StoreCallees<'_, S> {
    fn callee(&self, name: &str, at: &str) -> Result<Option<datumesh_formula::definition::Callee>, String> {
        let Some(b) = self.0.resolve_binding(name, at).map_err(|e| e.to_string())? else { return Ok(None) };
        let bytes = self.0.descriptor(&b.fn_hash).ok_or_else(|| format!("descriptor {} is not stored", b.fn_hash))?;
        let descriptor = json::parse(&bytes).map_err(|e| e.to_string())?;
        Ok(Some(datumesh_formula::definition::Callee { fn_hash: b.fn_hash, start: b.tx, descriptor }))
    }

    fn bound_class(&self, name: &str, at: &str) -> Result<Option<ClassCallee>, String> {
        let Some(b) = self.0.resolve_class_binding(name, at).map_err(|e| e.to_string())? else { return Ok(None) };
        let bytes = self
            .0
            .class(&b.class_hash)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("class {} is not stored", b.class_hash))?;
        let descriptor = ClassDescriptor::from_json(&bytes).map_err(|e| e.to_string())?;
        Ok(Some(ClassCallee { class_hash: b.class_hash, start: b.tx, descriptor }))
    }

    fn class_descriptor(&self, class_hash: &str) -> Result<Option<ClassDescriptor>, String> {
        let Some(bytes) = self.0.class(class_hash).map_err(|e| e.to_string())? else { return Ok(None) };
        ClassDescriptor::from_json(&bytes).map(Some).map_err(|e| e.to_string())
    }

    fn function_descriptor(&self, fn_hash: &str) -> Result<Option<Value>, String> {
        let Some(bytes) = self.0.descriptor(fn_hash) else { return Ok(None) };
        json::parse(&bytes).map(Some).map_err(|e| e.to_string())
    }
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
/// module documentation). The mesh's own namespaces are `host` alone: see [`publish_stored_in`].
pub fn publish_stored<S: Store + ?Sized>(
    store: &S,
    key: &SigningKey,
    host: &str,
    path: &Path,
) -> Result<Published, PublishError> {
    publish_stored_in(store, key, host, &[host.to_string()], path)
}

/// As [`publish_stored`], for a mesh that owns the namespaces (hosts) `owned` (`Config::namespaces`):
/// a class line is accepted for a class under one of them, and refused with `E_CLASS_OWNER` (in the
/// [`PublishError::detail`]) under any other. Since 0.3.0.
pub fn publish_stored_in<S: Store + ?Sized>(
    store: &S,
    key: &SigningKey,
    host: &str,
    owned: &[String],
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
        if let Some(name) = v.get("class").and_then(Value::as_str) {
            let descriptor = v.get("descriptor").ok_or_else(|| err("a class without descriptor".into()))?;
            let bound = v.get("bound").and_then(Value::as_str).ok_or_else(|| err("a class without bound".into()))?;
            // 16.2 step 1: the class is under one of the mesh's own namespaces. Core checks that the
            // class's namespace is the definer's; a host the mesh does not own is defined by the mesh's
            // host, which no class name under it has, so core answers E_CLASS_OWNER.
            let host_of_name = name.split('/').next().unwrap_or("");
            let definer = if owned.iter().any(|o| o == host_of_name) {
                name.rsplit_once('/').map(|(ns, _)| ns).unwrap_or("")
            } else {
                host
            };
            let def = datumesh_formula::definition::define_class(
                name,
                &descriptor.to_jcs(),
                definer,
                &StoreCallees(store),
                bound,
            )
            .map_err(|e| err(format!("define {name}: {e}")))?;
            let bound_hash = store.put_class(&def.bytes).map_err(|e| err(e.to_string()))?;
            match store.resolve_class(name, "9999-12-31T23:59:59.999999Z").map_err(|e| err(e.to_string()))? {
                Some(h) if h == bound_hash => {}
                _ => {
                    store.bind_class(name, &bound_hash, bound).map_err(|e| err(format!("bind {name}: {e}")))?;
                    out.classes += 1;
                }
            }
            continue;
        }
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
        if let Some(name) = v.get("formula").and_then(Value::as_str) {
            let source =
                v.get("source").and_then(Value::as_str).ok_or_else(|| err("a formula without source".into()))?;
            let sig = v.get("signature").ok_or_else(|| err("a formula without a signature".into()))?;
            let bound = v.get("bound").and_then(Value::as_str).ok_or_else(|| err("a formula without bound".into()))?;
            let def = datumesh_formula::definition::define_function(
                name,
                source,
                sig,
                &Default::default(),
                &StoreCallees(store),
                bound,
            )
            .map_err(|e| err(format!("define {name}: {e}")))?;
            let h = store.put_descriptor(&def.bytes).map_err(|e| err(e.to_string()))?;
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
