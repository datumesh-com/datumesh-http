//! Key-derived hosted names (Build 2, QUESTIONS-build2 D39; http 0.4.8, `DATUMESH_KEY_NAMES=1`).
//!
//! A person registers an Ed25519 public key with `POST /x/keys`, after `GET /x/keys/challenge`: the body
//! carries a challenge the mesh issued and its signature (proof of possession). The key is added to the
//! host's key document, valid from the registration, and its namespace is
//! `<host>/key/k<first 24 hex digits of SHA-256 over the 32 raw public key bytes>`. The key document is the
//! registration's record in the store: a registered key is every key of the document but the mesh's own.
//!
//! A bundle signed by a registered key is accepted only under that key's namespace (`POST /bundles`, see
//! [`bundle_refusal`]). Challenges are kept in memory, valid for `DATUMESH_KEY_CHALLENGE_TTL_SECS` of the
//! mesh clock, and good for one `POST` only.

use std::collections::HashMap;

use datumesh_core::json::{self, Value};
use datumesh_core::{lexical, verify_ed25519};

use crate::envelope::{Cache, Reply};
use crate::mesh::Inner;

/// The most challenges outstanding at once (the GET is cheap; this bounds the memory it can take).
const MAX_OUTSTANDING: usize = 10_000;
/// The most bytes of a `POST /x/keys` body.
pub(crate) const MAX_BODY: usize = 64 << 10;

/// The challenges issued and not yet used: each with the instant it expires at.
#[derive(Default)]
pub(crate) struct Challenges {
    issued: HashMap<String, String>,
}

/// The namespace a registered key publishes under.
pub(crate) fn namespace_of(host: &str, public: &[u8; 32]) -> String {
    format!("{host}/key/k{}", &datumesh_core::sha256_hex(public)[..24])
}

/// Whether `ns` is a key namespace of `host`: `<host>/key/k` and exactly 24 lower-case hex digits.
pub(crate) fn is_key_namespace(host: &str, ns: &str) -> bool {
    let Some(rest) = ns.strip_prefix(host).and_then(|r| r.strip_prefix("/key/k")) else { return false };
    rest.len() == 24 && rest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `instant` (`YYYY-MM-DDTHH:MM:SS.ffffffZ`) plus `secs` seconds, in the same form.
pub(crate) fn add_seconds(instant: &str, secs: u64) -> Option<String> {
    if lexical::check_instant(instant, "$").is_err() {
        return None;
    }
    let num = |a: usize, b: usize| instant.get(a..b)?.parse::<i64>().ok();
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hh, mm, ss) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let frac = instant.get(20..26)?;
    // days since 1970-01-01 (proleptic Gregorian), and back
    let days = {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    };
    let total = days * 86_400 + hh * 3600 + mm * 60 + ss + secs as i64;
    let (days, rem) = (total.div_euclid(86_400), total.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d2 = doy - (153 * mp + 2) / 5 + 1;
    let m2 = if mp < 10 { mp + 3 } else { mp - 9 };
    let y2 = yoe + era * 400 + i64::from(m2 <= 2);
    Some(format!("{y2:04}-{m2:02}-{d2:02}T{:02}:{:02}:{:02}.{frac}Z", rem / 3600, rem % 3600 / 60, rem % 60))
}

/// The refusal of a key-names request: its status, its code and its detail.
pub(crate) fn refuse(status: u16, code: &str, detail: &str) -> Reply {
    let body = Value::Object(vec![
        ("error".into(), Value::String(code.into())),
        ("detail".into(), Value::String(detail.into())),
    ]);
    Reply::json(status, body.to_jcs(), Cache::NoStore)
}

/// `429 rate_limited` of the key-names routes (`error` as the other refusals, with `Retry-After`).
pub(crate) fn rate_limited(seconds: u64) -> Reply {
    let mut r = refuse(429, "rate_limited", "too many requests from this address: try again later");
    r.retry_after = Some(seconds);
    r
}

/// `GET /x/keys/challenge`: a new challenge and its expiry (`{"challenge", "expires"}`).
pub(crate) fn challenge(inner: &Inner) -> Reply {
    let now = inner.now();
    let Some(expires) = add_seconds(&now, inner.config.key_challenge_ttl.as_secs()) else {
        return refuse(500, "error", "the mesh clock is not an instant");
    };
    let mut seed = [0u8; 32];
    if getrandom::fill(&mut seed).is_err() {
        return refuse(500, "error", "no random source");
    }
    let challenge = lexical::to_hex(&seed);
    let mut c = inner.challenges.lock().unwrap_or_else(|e| e.into_inner());
    c.issued.retain(|_, e| now < *e);
    if c.issued.len() >= MAX_OUTSTANDING {
        drop(c);
        return rate_limited(60);
    }
    c.issued.insert(challenge.clone(), expires.clone());
    drop(c);
    let body =
        Value::Object(vec![("challenge".into(), Value::String(challenge)), ("expires".into(), Value::String(expires))]);
    Reply::json(200, body.to_jcs(), Cache::NoStore)
}

/// A parsed `POST /x/keys` body.
pub(crate) struct Proof {
    public: [u8; 32],
    challenge: String,
    signature: [u8; 64],
}

fn hex_of<const N: usize>(text: &str, lower_only: bool) -> Option<[u8; N]> {
    if text.len() != 2 * N || (lower_only && text.bytes().any(|b| b.is_ascii_uppercase())) {
        return None;
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// The body of `POST /x/keys`: a JSON object with `public_key` (64 lower-case hex digits), `challenge` (a
/// string) and `signature` (128 hex digits). Anything else is `E_KEY_MALFORMED`.
pub(crate) fn parse(body: &[u8]) -> Result<Proof, String> {
    let v = json::parse(body).map_err(|e| format!("not JSON: {e}"))?;
    if !matches!(v, Value::Object(_)) {
        return Err("the body is a JSON object".into());
    }
    let text = |k: &str| v.get(k).and_then(Value::as_str).ok_or_else(|| format!("{k} is a string"));
    let public = hex_of::<32>(text("public_key")?, true).ok_or("public_key is 64 lower-case hex digits")?;
    let challenge = text("challenge")?.to_string();
    let signature = hex_of::<64>(text("signature")?, false).ok_or("signature is 128 hex digits")?;
    Ok(Proof { public, challenge, signature })
}

/// `POST /x/keys` after the parse: the challenge is used (good once), the signature is checked, and the key
/// is added to the key document unless it is there already. `201` with the namespace and key id when it is
/// added, `200` with the same body when it was registered before.
pub(crate) fn register(inner: &Inner, p: &Proof) -> Reply {
    let now = inner.now();
    let known = {
        let mut c = inner.challenges.lock().unwrap_or_else(|e| e.into_inner());
        c.issued.remove(&p.challenge)
    };
    match known {
        Some(expires) if now < expires => {}
        _ => return refuse(400, "E_CHALLENGE", "the challenge is unknown, expired or used"),
    }
    if !verify_ed25519(&p.public, p.challenge.as_bytes(), &p.signature) {
        return refuse(403, "E_KEY_SIGNATURE", "the signature of the challenge does not verify with public_key");
    }
    let host = &inner.config.host;
    let namespace = namespace_of(host, &p.public);
    let key_id = datumesh_core::key_id(&p.public);
    let body = || {
        Value::Object(vec![
            ("namespace".into(), Value::String(namespace.clone())),
            ("key_id".into(), Value::String(key_id.clone())),
        ])
        .to_jcs()
    };
    let _writes = inner.keys_lock.lock().unwrap_or_else(|e| e.into_inner());
    let existing = match inner.store.namespace(host) {
        Ok(Some(n)) => n.keydoc,
        _ => return refuse(500, "error", "the store holds no key document of this mesh"),
    };
    match crate::keydoc::with_registered(host, &existing, &p.public, &now) {
        Ok(Some(doc)) => match inner.store.put_namespace(host, &doc, &now) {
            Ok(()) => Reply::json(201, body(), Cache::NoStore),
            Err(e) => refuse(500, "error", &format!("the key document is not written: {e}")),
        },
        Ok(None) => Reply::json(200, body(), Cache::NoStore),
        Err(e) => refuse(500, "error", &format!("the key document is not made: {e}")),
    }
}

/// The public key of `key_id` in a key document, if the document holds it.
fn public_of(keydoc: &[u8], key_id: &str) -> Option<[u8; 32]> {
    let v = json::parse(keydoc).ok()?;
    let Value::Array(keys) = v.get("keys")? else { return None };
    let entry = keys.iter().find(|k| k.get("id").and_then(Value::as_str) == Some(key_id))?;
    hex_of::<32>(entry.get("public")?.as_str()?, true)
}

/// The key namespace a bundle signed by `key_id` may publish under (H8): the namespace derived from the key, when
/// the key is a registered one (not the mesh's own key). `None` for the mesh's own key, or a key the document lacks.
pub(crate) fn signer_namespace(inner: &Inner, key_id: &str, keydoc: &[u8]) -> Option<String> {
    if inner.own_key_id.as_deref() == Some(key_id) {
        return None;
    }
    let public = public_of(keydoc, key_id)?;
    Some(namespace_of(&inner.config.host, &public))
}

/// The key-names rule of `POST /bundles` (D39), for a bundle of the mesh's own host signed by `key_id`: a key
/// namespace, or a key the mesh registered (any key but its own), publishes under the namespace of its own
/// key only. `Some((status, code, detail))` refuses the bundle; nothing is written then.
pub(crate) fn bundle_refusal(
    inner: &Inner,
    namespace: &str,
    key_id: &str,
    keydoc: &[u8],
) -> Option<(u16, &'static str, String)> {
    if !inner.config.key_names {
        return None;
    }
    let host = &inner.config.host;
    let registered = inner.own_key_id.as_deref().is_some_and(|own| own != key_id);
    if !registered && !is_key_namespace(host, namespace) {
        return None;
    }
    let public = public_of(keydoc, key_id)?;
    let expected = namespace_of(host, &public);
    if namespace == expected {
        return None;
    }
    Some((422, "E_NOT_OWNED", format!("key {key_id} publishes under {expected} only")))
}

/// The seed of a mesh key file (32 raw bytes, or 64 hex digits and a line end), for the mesh's key id.
pub(crate) fn seed_of_file(bytes: &[u8]) -> Option<[u8; 32]> {
    if bytes.len() == 32 {
        return bytes.try_into().ok();
    }
    let text = std::str::from_utf8(bytes).ok()?.trim();
    hex_of::<32>(text, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instants_add_seconds_across_days_and_years() {
        assert_eq!(add_seconds("2026-11-16T00:00:00.000000Z", 300).unwrap(), "2026-11-16T00:05:00.000000Z");
        assert_eq!(add_seconds("2026-12-31T23:59:30.123456Z", 60).unwrap(), "2027-01-01T00:00:30.123456Z");
        assert_eq!(add_seconds("2024-02-28T23:59:59.000000Z", 86_400).unwrap(), "2024-02-29T23:59:59.000000Z");
        assert_eq!(add_seconds("2026-11-16T00:00:00.000000Z", 0).unwrap(), "2026-11-16T00:00:00.000000Z");
        assert!(add_seconds("yesterday", 1).is_none());
    }

    #[test]
    fn key_namespaces_are_exactly_the_24_lower_hex_form() {
        let h = "mesh.example";
        let ok = format!("{h}/key/k{}", "0123456789abcdef01234567");
        assert!(is_key_namespace(h, &ok));
        assert!(!is_key_namespace(h, &format!("{h}/key/k{}", "0123456789abcdef0123456")));
        assert!(!is_key_namespace(h, &format!("{h}/key/k{}", "0123456789ABCDEF01234567")));
        assert!(!is_key_namespace(h, &format!("{h}/key/k{}x", "0123456789abcdef01234567")));
        assert!(!is_key_namespace(h, &format!("{h}/key/{}", "0123456789abcdef01234567")));
        assert!(!is_key_namespace("other.example", &ok));
    }
}
