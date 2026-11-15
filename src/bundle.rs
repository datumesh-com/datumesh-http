//! `bundle 1` (CONTRACT-1.8b H2, `docs/bundle-1.md`): a namespace's function descriptors, name
//! bindings and signed receipts as one JSON object, with a manifest signed by the namespace's host
//! (Ed25519, RFC 8032, over the JCS bytes of the names, the namespace and the receipt hashes).
//! Operator format, not spec: the body of the plan-3.1 write endpoint and of `import-signed`.

use std::collections::BTreeMap;

use datumesh_core::json::{self, Value};
use datumesh_core::{
    keys_from_response, lexical::to_hex, verify, verify_ed25519, FunctionDescriptor, Keys, Receipt, SigningKey,
};
use datumesh_engine::Clock;
use datumesh_store::Store;

/// After every binding.
const END: &str = "9999-12-31T23:59:59.999999Z";

/// The members of a bundle, in JCS order.
const MEMBERS: [&str; 6] = ["bundle", "descriptors", "manifest", "names", "namespace", "receipts"];

/// Why a bundle is refused. The first failing check is reported; nothing is written unless all pass.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleError {
    /// Not JSON, a missing or extra member, `bundle` not 1, a malformed hex, or an order not kept.
    Format(String),
    /// The namespace's host is neither the mesh's nor an owned one, or a name is not under the namespace.
    NotOwned(String),
    /// The manifest: no key of the namespace's host, the key's window does not hold now, or the
    /// signature does not verify over the bundle's names and receipt hashes.
    Manifest(String),
    /// A descriptor that is not canonical, or a name whose descriptor is neither in the bundle nor stored.
    Descriptor(String),
    /// A receipt: computed (with inputs), not verifying against the namespace's key document, or
    /// not for one of the bundle's names.
    Receipt(String),
    /// A receipt whose `tx` is later than the mesh clock.
    TxInFuture {
        /// The receipt's hash.
        receipt: String,
        /// Its `tx`.
        tx: String,
        /// The mesh clock.
        now: String,
    },
    /// A name whose latest binding is at or after the clock.
    Bind(String),
    /// The store refused a read or a write (a cell with a later version, say).
    Store(String),
    /// A test hook (`import_signed_with`'s `stop_after`): stopped after this many writes.
    Stopped(usize),
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleError::Format(s) => write!(f, "bundle format: {s}"),
            BundleError::NotOwned(s) => write!(f, "not owned: {s}"),
            BundleError::Manifest(s) => write!(f, "manifest: {s}"),
            BundleError::Descriptor(s) => write!(f, "descriptor: {s}"),
            BundleError::Receipt(s) => write!(f, "receipt: {s}"),
            BundleError::TxInFuture { receipt, tx, now } => {
                write!(f, "receipt {receipt}: tx {tx} is later than the mesh clock {now}")
            }
            BundleError::Bind(s) => write!(f, "binding: {s}"),
            BundleError::Store(s) => write!(f, "store: {s}"),
            BundleError::Stopped(n) => write!(f, "stopped after write {n}"),
        }
    }
}

impl std::error::Error for BundleError {}

/// What an import did.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Imported {
    /// The bundle's namespace.
    pub namespace: String,
    /// Names newly bound (a name already bound to its hash is not counted).
    pub functions_bound: usize,
    /// Receipts in the bundle (each is stored; putting one again is idempotent).
    pub receipts: usize,
    /// Versions newly added.
    pub versions: usize,
    /// The instant the names were bound at: the mesh clock.
    pub bound_at: String,
}

impl Imported {
    /// The result as one JCS object (the CLI prints this).
    pub fn to_jcs(&self) -> Vec<u8> {
        Value::Object(vec![
            ("bound_at".into(), s(&self.bound_at)),
            ("functions_bound".into(), number(self.functions_bound)),
            ("namespace".into(), s(&self.namespace)),
            ("receipts".into(), number(self.receipts)),
            ("versions".into(), number(self.versions)),
        ])
        .to_jcs()
    }
}

fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

fn number(n: usize) -> Value {
    json::parse(n.to_string().as_bytes()).unwrap_or(Value::Null)
}

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).filter_map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

/// The host of a namespace: its first segment.
fn host_of(namespace: &str) -> &str {
    namespace.split('/').next().unwrap_or("")
}

/// The JSON of a name and its function hash, as the bundle and the manifest write it.
fn name_value(name: &str, fn_hash: &str) -> Value {
    Value::Object(vec![("fn_hash".into(), s(fn_hash)), ("name".into(), s(name))])
}

/// The bytes the manifest signs (CONTRACT-1.8b H2): the JCS object of the names (`(name, fn_hash)`,
/// ascending by name), the namespace and the receipt hashes (ascending).
pub fn manifest_bytes(namespace: &str, names: &[(String, String)], receipts: &[String]) -> Vec<u8> {
    Value::Object(vec![
        ("names".into(), Value::Array(names.iter().map(|(n, h)| name_value(n, h)).collect())),
        ("namespace".into(), s(namespace)),
        ("receipts".into(), Value::Array(receipts.iter().map(|h| s(h)).collect())),
    ])
    .to_jcs()
}

/// Ed25519 (RFC 8032) over `message` with the key of `seed`, by `ed25519-dalek`: the signature
/// `R || S`. The public key and key id are those of `SigningKey::from_seed(seed)`; the import
/// verifies with core's `verify_ed25519`, the strict rules of receipt verification.
fn sign(seed: &[u8; 32], message: &[u8]) -> [u8; 64] {
    use ed25519_dalek::Signer;
    ed25519_dalek::SigningKey::from_bytes(seed).sign(message).to_bytes()
}

/// Builds a bundle 1 (see the module documentation): `descriptors` (each a canonical function
/// descriptor), `names` (`(name, fn_hash)`), `receipts` (each a receipt's JSON), signed with the
/// key of `seed` as the manifest of `namespace`. The names and the receipts are put in ascending
/// order. Errors: `Format` for a receipt or descriptor that does not parse.
pub fn build(
    namespace: &str,
    descriptors: Vec<Vec<u8>>,
    names: Vec<(String, String)>,
    receipts: Vec<Vec<u8>>,
    seed: &[u8; 32],
) -> Result<Vec<u8>, BundleError> {
    let mut names = names;
    names.sort();
    let mut signed: Vec<(String, Value)> = Vec::new();
    for r in receipts {
        let hash = Receipt::from_json(&r).map_err(|e| BundleError::Format(format!("receipt: {e}")))?.hash().to_string();
        let v = json::parse(&r).map_err(|e| BundleError::Format(format!("receipt: {e}")))?;
        signed.push((hash, v));
    }
    signed.sort_by(|a, b| a.0.cmp(&b.0));
    let hashes: Vec<String> = signed.iter().map(|(h, _)| h.clone()).collect();
    let sig = sign(seed, &manifest_bytes(namespace, &names, &hashes));
    let key = SigningKey::from_seed(seed);
    let descriptors = descriptors
        .iter()
        .map(|d| json::parse(d).map_err(|e| BundleError::Format(format!("descriptor: {e}"))))
        .collect::<Result<Vec<Value>, _>>()?;
    Ok(Value::Object(vec![
        ("bundle".into(), number(1)),
        ("descriptors".into(), Value::Array(descriptors)),
        (
            "manifest".into(),
            Value::Object(vec![("key_id".into(), s(key.key_id())), ("signature".into(), s(&to_hex(&sig)))]),
        ),
        ("names".into(), Value::Array(names.iter().map(|(n, h)| name_value(n, h)).collect())),
        ("namespace".into(), s(namespace)),
        ("receipts".into(), Value::Array(signed.into_iter().map(|(_, v)| v).collect())),
    ])
    .to_jcs())
}

/// The keys of an object's members, sorted.
fn member_names(v: &Value) -> Option<Vec<&str>> {
    let mut keys: Vec<&str> = v.as_object()?.iter().map(|(k, _)| k.as_str()).collect();
    keys.sort_unstable();
    Some(keys)
}

/// A string member of `v`, or a format error.
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, BundleError> {
    v.get(key).and_then(Value::as_str).ok_or_else(|| BundleError::Format(format!("{key} is not a string")))
}

/// Imports the bundle `bytes` for the mesh `host`, whose own namespaces are `owned`, at the clock's
/// now (CONTRACT-1.8b H2). Checks run in the order of the contract, and nothing is written unless
/// they all pass. The writes are, in order: the descriptors; the receipts (each `put_receipt`), then
/// the versions (each `put_version`); the name bindings last, so that a crash part-way leaves no
/// name answering. A re-run completes a partial import: a version or a binding already there is
/// skipped.
pub fn import_signed<S: Store + ?Sized>(
    store: &S,
    host: &str,
    owned: &[String],
    clock: &dyn Clock,
    bytes: &[u8],
) -> Result<Imported, BundleError> {
    import_signed_with(store, host, owned, clock, bytes, None)
}

/// As [`import_signed`], with a test hook (`#[doc(hidden)]`): with `stop_after` `Some(n)` the import
/// stops with [`BundleError::Stopped`]`(n)` after its `n`-th write (each `put_descriptor`,
/// `put_receipt`, `put_version` and `bind_name` counts as one), to test that a re-run completes it.
#[doc(hidden)]
pub fn import_signed_with<S: Store + ?Sized>(
    store: &S,
    host: &str,
    owned: &[String],
    clock: &dyn Clock,
    bytes: &[u8],
    stop_after: Option<usize>,
) -> Result<Imported, BundleError> {
    let now = clock.now();
    let doc = json::parse(bytes).map_err(|e| BundleError::Format(format!("not JSON: {e}")))?;

    // 1. Format.
    let mut keys = member_names(&doc).ok_or_else(|| BundleError::Format("not an object".into()))?;
    keys.sort_unstable();
    let mut want = MEMBERS;
    want.sort_unstable();
    if keys != want {
        return Err(BundleError::Format("the members are not exactly those of a bundle".into()));
    }
    if doc.get("bundle").and_then(Value::as_number_text) != Some("1") {
        return Err(BundleError::Format("bundle is not 1".into()));
    }
    let namespace = text(&doc, "namespace")?.to_string();
    if namespace.split('/').count() < 2 {
        return Err(BundleError::Format("namespace has no host and name".into()));
    }
    let manifest = doc.get("manifest").ok_or_else(|| BundleError::Format("no manifest".into()))?;
    if member_names(manifest) != Some(vec!["key_id", "signature"]) {
        return Err(BundleError::Format("the manifest's members are not key_id and signature".into()));
    }
    let key_id = text(manifest, "key_id")?.to_string();
    let signature = text(manifest, "signature")?.to_string();
    if !is_hex(&key_id, 16) || !is_hex(&signature, 128) {
        return Err(BundleError::Format("the manifest's key_id or signature is not hex".into()));
    }
    let mut names: Vec<(String, String)> = Vec::new();
    let Some(Value::Array(name_vs)) = doc.get("names") else {
        return Err(BundleError::Format("names is not an array".into()));
    };
    for n in name_vs {
        if member_names(n) != Some(vec!["fn_hash", "name"]) {
            return Err(BundleError::Format("a name's members are not fn_hash and name".into()));
        }
        let fh = text(n, "fn_hash")?;
        if !is_hex(fh, 64) {
            return Err(BundleError::Format(format!("fn_hash {fh} is not 64 hex digits")));
        }
        names.push((text(n, "name")?.to_string(), fh.to_string()));
    }
    if names.windows(2).any(|w| w[0].0 >= w[1].0) {
        return Err(BundleError::Format("names are not in ascending order".into()));
    }
    let Some(Value::Array(desc_vs)) = doc.get("descriptors") else {
        return Err(BundleError::Format("descriptors is not an array".into()));
    };
    let Some(Value::Array(receipt_vs)) = doc.get("receipts") else {
        return Err(BundleError::Format("receipts is not an array".into()));
    };
    // (the JCS bytes of the receipt, the receipt)
    let mut receipts: Vec<(Vec<u8>, Receipt)> = Vec::new();
    for r in receipt_vs {
        let rb = r.to_jcs();
        let parsed = Receipt::from_json(&rb).map_err(|e| BundleError::Receipt(format!("unreadable: {e}")))?;
        receipts.push((rb, parsed));
    }
    if receipts.windows(2).any(|w| w[0].1.hash() >= w[1].1.hash()) {
        return Err(BundleError::Format("receipts are not in ascending order of their hashes".into()));
    }
    let hashes: Vec<String> = receipts.iter().map(|(_, r)| r.hash().to_string()).collect();

    // 2. NotOwned.
    let ns_host = host_of(&namespace);
    if ns_host != host && !owned.iter().any(|o| o == ns_host) {
        return Err(BundleError::NotOwned(format!("{ns_host} is neither this mesh's host nor owned")));
    }
    if let Some((n, _)) = names.iter().find(|(n, _)| !n.starts_with(&format!("{namespace}/"))) {
        return Err(BundleError::NotOwned(format!("{n} is not under {namespace}/")));
    }

    // 3. Manifest: the key of the namespace's host, valid now, signing the bundle's names and receipts.
    let keydoc = store
        .namespace(ns_host)
        .map_err(|e| BundleError::Store(e.to_string()))?
        .ok_or_else(|| BundleError::Manifest(format!("the store holds no key document of {ns_host}")))?
        .keydoc;
    let kd = keys_from_response(ns_host, 200, Some(&keydoc)).map_err(|e| BundleError::Manifest(e.to_string()))?;
    let key = kd
        .key(&key_id)
        .ok_or_else(|| BundleError::Manifest(format!("key {key_id} is not in the key document of {ns_host}")))?;
    if !key.usable_at(&now) {
        return Err(BundleError::Manifest(format!("key {key_id} does not hold at {now}")));
    }
    let mut keys_set = Keys::new();
    keys_set.insert(kd.clone());
    let sig: [u8; 64] = unhex(&signature).try_into().map_err(|_| BundleError::Format("signature length".into()))?;
    if !verify_ed25519(&key.public, &manifest_bytes(&namespace, &names, &hashes), &sig) {
        return Err(BundleError::Manifest("the signature does not verify".into()));
    }

    // 4. Descriptor.
    let mut descriptors: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for d in desc_vs {
        let db = d.to_jcs();
        let fd = FunctionDescriptor::from_json(&db).map_err(|e| BundleError::Descriptor(e.to_string()))?;
        if fd.canonical() != db {
            return Err(BundleError::Descriptor("a descriptor is not canonical".into()));
        }
        descriptors.insert(fd.hash(), db);
    }
    for (name, fh) in &names {
        if !descriptors.contains_key(fh) && store.descriptor(fh).is_none() {
            return Err(BundleError::Descriptor(format!(
                "{name}: descriptor {fh} is neither in the bundle nor stored"
            )));
        }
    }

    // 5. Receipt.
    for (rb, r) in &receipts {
        if !r.inputs().is_empty() {
            return Err(BundleError::Receipt(format!("{}: a computed receipt is not imported", r.hash())));
        }
        let fname = r.cell().function_name();
        let Some((_, fh)) = names.iter().find(|(n, _)| *n == fname) else {
            return Err(BundleError::Receipt(format!("{}: {fname} is not among the names", r.hash())));
        };
        if r.cell().fn_hash != *fh {
            return Err(BundleError::Receipt(format!("{}: the function hash differs from {fname}'s", r.hash())));
        }
        let out = verify(rb, &keys_set, None, None);
        if !out.ok {
            let why = out.error().map(|e| e.to_string()).unwrap_or_else(|| "does not verify".into());
            return Err(BundleError::Receipt(format!("{}: {why}", r.hash())));
        }
    }

    // 6. TxInFuture.
    for (_, r) in &receipts {
        if r.tx() > now.as_str() {
            return Err(BundleError::TxInFuture {
                receipt: r.hash().to_string(),
                tx: r.tx().to_string(),
                now: now.clone(),
            });
        }
    }

    // Plan: the binds, and the versions not already there; a conflict is refused before any write.
    let mut binds: Vec<&(String, String)> = Vec::new();
    for pair in &names {
        let (name, fh) = pair;
        match store.resolve_binding(name, END).map_err(|e| BundleError::Store(e.to_string()))? {
            Some(b) if b.fn_hash == *fh => {}
            Some(b) if b.tx.as_str() >= now.as_str() => {
                return Err(BundleError::Bind(format!("{name} is bound from {} (at or after the clock)", b.tx)));
            }
            _ => binds.push(pair),
        }
    }
    let mut new_versions: Vec<(String, String, String)> = Vec::new(); // (cell id, tx, receipt hash)
    for (_, r) in &receipts {
        let id = r.cell().id();
        let tx = r.tx();
        match store.version_asof(&id, tx).map_err(|e| BundleError::Store(e.to_string()))? {
            Some(v) if v.tx == tx => {
                if v.receipt_hash != r.hash() {
                    return Err(BundleError::Store(format!("{id} has another version at {tx}")));
                }
                continue;
            }
            _ => {}
        }
        if let Some(l) = store.latest(&id).map_err(|e| BundleError::Store(e.to_string()))? {
            if l.tx.as_str() >= tx {
                return Err(BundleError::Store(format!("{id} has a version at or after {tx}")));
            }
        }
        new_versions.push((id, tx.to_string(), r.hash().to_string()));
    }

    // Writes: descriptors, receipts, versions, then the bindings (a version without its binding is
    // not answered, so a crash before the bindings leaves nothing answerable).
    let mut writes = 0usize;
    let mut step = || -> Result<(), BundleError> {
        writes += 1;
        if stop_after == Some(writes) {
            return Err(BundleError::Stopped(writes));
        }
        Ok(())
    };
    for db in descriptors.values() {
        store.put_descriptor(db).map_err(|e| BundleError::Store(e.to_string()))?;
        step()?;
    }
    for (rb, _) in &receipts {
        store.put_receipt(rb).map_err(|e| BundleError::Store(e.to_string()))?;
        step()?;
    }
    for (id, tx, h) in &new_versions {
        store.put_version(id, tx, h).map_err(|e| BundleError::Store(e.to_string()))?;
        step()?;
    }
    for (name, fh) in &binds {
        store.bind_name(name, fh, &now).map_err(|e| BundleError::Bind(format!("{name}: {e}")))?;
        step()?;
    }
    Ok(Imported {
        namespace,
        functions_bound: binds.len(),
        receipts: receipts.len(),
        versions: new_versions.len(),
        bound_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 8032 section 7.1, test 1: the empty message.
    #[test]
    fn rfc8032_test_1() {
        let seed: [u8; 32] =
            unhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60").try_into().unwrap();
        let sig = sign(&seed, b"");
        assert_eq!(
            to_hex(&sig),
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
    }

    /// A signature over some bytes verifies with core's strict verification, under the key of the seed.
    #[test]
    fn a_signature_verifies_under_the_seed_key() {
        let seed = [7u8; 32];
        let message = b"the manifest bytes";
        let sig = sign(&seed, message);
        let public = SigningKey::from_seed(&seed).public_key();
        assert!(verify_ed25519(&public, message, &sig));
        assert!(!verify_ed25519(&public, b"other bytes", &sig));
    }
}
