//! Envelopes (SPEC 5.5) as the mesh writes them, and [`Reply`]: an HTTP answer before it is
//! an HTTP response.
//!
//! Every envelope is written as canonical JSON (JCS), so the same statement always has the
//! same bytes (a call and a cell URI for one cell give byte-identical bodies). The embedded
//! receipt is the stored receipt, unchanged.

use datumesh_core::json::{self, Value};
use datumesh_core::{ErrorCode, Fault, Pending, Receipt, Trust};

/// How a reply may be cached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cache {
    /// `public, max-age=31536000, immutable`: content-addressed bytes, and answers as of a
    /// past instant.
    Immutable,
    /// `public, max-age=<n>`: answers about "now", and other short-lived statements.
    Public(u64),
    /// `private, no-store`: answers that depend on who asks.
    Private,
    /// `no-store`: errors, refusals, jobs.
    NoStore,
}

impl Cache {
    /// The `Cache-Control` header value.
    pub fn header(self) -> String {
        match self {
            Cache::Immutable => "public, max-age=31536000, immutable".to_string(),
            Cache::Public(n) => format!("public, max-age={n}"),
            Cache::Private => "private, no-store".to_string(),
            Cache::NoStore => "no-store".to_string(),
        }
    }
}

/// An answer: HTTP status, body and the headers the mesh sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// The HTTP status.
    pub status: u16,
    /// The body.
    pub body: Vec<u8>,
    /// The media type.
    pub content_type: &'static str,
    /// Caching.
    pub cache: Cache,
    /// A `Link: <...>; rel="canonical"` target: the as-of form of an answer about "now".
    pub canonical: Option<String>,
    /// `Retry-After`, in seconds.
    pub retry_after: Option<u64>,
}

/// The media type of JSON bodies.
pub const JSON: &str = "application/json";

impl Reply {
    /// A JSON reply.
    pub fn json(status: u16, body: Vec<u8>, cache: Cache) -> Reply {
        Reply { status, body, content_type: JSON, cache, canonical: None, retry_after: None }
    }

    /// The envelope's `outcome`, when the body is an envelope.
    pub fn outcome(&self) -> Option<String> {
        json::parse(&self.body).ok()?.get("outcome")?.as_str().map(str::to_string)
    }
}

/// A JSON number (the envelope's integers: SPEC 5.5 writes them `0` or `[1-9][0-9]*`).
pub(crate) fn num(n: u64) -> Value {
    json::parse(n.to_string().as_bytes()).unwrap_or(Value::Null)
}

fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

fn object(members: Vec<(&str, Value)>) -> Value {
    Value::Object(members.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The canonical bytes of an envelope.
fn bytes(v: Value) -> Vec<u8> {
    v.to_jcs()
}

/// `trust` for a receipt this mesh serves (SPEC 5.5): `primary` exactly when the signer's
/// host is the namespace's host and there are no inputs; `verified` for a receipt this mesh
/// computed (`computed_by` its own host); `unverified` otherwise.
pub fn trust_of(receipt: &Receipt, mesh_host: &str) -> Trust {
    let ns_host = receipt.cell().ns.split('/').next().unwrap_or("");
    if receipt.signer_host() == ns_host && receipt.inputs().is_empty() {
        Trust::Primary
    } else if receipt.signer_host() == mesh_host {
        Trust::Verified
    } else {
        Trust::Unverified
    }
}

/// An `ok` envelope around the receipt's bytes (its JSON, unchanged).
pub fn ok(receipt: &[u8], trust: Trust) -> Vec<u8> {
    let r = json::parse(receipt).unwrap_or(Value::Null);
    bytes(object(vec![("outcome", s("ok")), ("receipt", r), ("trust", s(trust.as_str()))]))
}

/// A `stale` envelope.
pub fn stale(receipt: &[u8], trust: Trust, age_seconds: u64) -> Vec<u8> {
    let r = json::parse(receipt).unwrap_or(Value::Null);
    bytes(object(vec![
        ("age_seconds", num(age_seconds)),
        ("outcome", s("stale")),
        ("receipt", r),
        ("trust", s(trust.as_str())),
    ]))
}

/// A `not_covered` envelope, with the evaluation's fault if it faulted.
pub fn not_covered(fault: Option<Fault>) -> Vec<u8> {
    let mut m = vec![("outcome", s("not_covered"))];
    if let Some(f) = fault {
        m.push(("fault", s(f.as_str())));
    }
    bytes(object(m))
}

/// An `unlicensed` envelope.
pub fn unlicensed() -> Vec<u8> {
    bytes(object(vec![("outcome", s("unlicensed"))]))
}

/// A `payment_required` envelope.
pub fn payment_required(amount: &str, currency: &str, authorise: &str) -> Vec<u8> {
    bytes(object(vec![
        ("outcome", s("payment_required")),
        ("quote", object(vec![("amount", s(amount)), ("authorise", s(authorise)), ("currency", s(currency))])),
    ]))
}

/// A `rate_limited` envelope.
pub fn rate_limited(retry_after_seconds: u64) -> Vec<u8> {
    bytes(object(vec![("outcome", s("rate_limited")), ("retry_after_seconds", num(retry_after_seconds))]))
}

/// A `bad_request` envelope.
pub fn bad_request(code: ErrorCode) -> Vec<u8> {
    bytes(object(vec![("error", s(code.as_str())), ("outcome", s("bad_request"))]))
}

/// An `error` envelope.
pub fn error(code: ErrorCode) -> Vec<u8> {
    bytes(object(vec![("error", s(code.as_str())), ("outcome", s("error"))]))
}

/// A `pending` envelope.
pub fn pending(p: &Pending) -> Vec<u8> {
    p.to_json()
}

/// The parse codes a `bad_request` may carry at `/cell` (SPEC 5.5).
pub const BAD_REQUEST_CODES: &[ErrorCode] = &[
    ErrorCode::ParseUri,
    ErrorCode::CanonName,
    ErrorCode::CanonArgName,
    ErrorCode::CanonArgDuplicate,
    ErrorCode::CanonArgOrder,
    ErrorCode::CanonPercent,
    ErrorCode::CanonType,
    ErrorCode::CanonNumber,
    ErrorCode::CanonDate,
    ErrorCode::CanonInstant,
    ErrorCode::CanonString,
    ErrorCode::CanonBlob,
    ErrorCode::CanonLimit,
    ErrorCode::ArgUnknown,
    ErrorCode::ArgMissing,
];

/// Whether `code` may be a `bad_request`'s `error` at `/cell`.
pub fn is_request_code(code: ErrorCode) -> bool {
    BAD_REQUEST_CODES.contains(&code)
}

/// Whether `code` may be an `error` envelope's `error`: any code of 10.2 but `E_ENVELOPE` and
/// `E_JOB_UNKNOWN`.
pub fn is_error_code(code: ErrorCode) -> bool {
    !matches!(code, ErrorCode::Envelope | ErrorCode::JobUnknown)
}

/// The HTTP status of an `error` envelope with `code`: 503 when something the mesh depends on
/// is unavailable (the store, another mesh, a key document), 500 otherwise.
pub fn error_status(code: ErrorCode) -> u16 {
    match code {
        ErrorCode::InputUnavailable | ErrorCode::KeyUnavailable | ErrorCode::MeshDoc | ErrorCode::EvalUnavailable => {
            503
        }
        _ => 500,
    }
}

/// The reply of an `error` envelope.
pub fn error_reply(code: ErrorCode) -> Reply {
    Reply::json(error_status(code), error(code), Cache::NoStore)
}

/// The reply of a `bad_request` envelope.
pub fn bad_request_reply(code: ErrorCode) -> Reply {
    Reply::json(400, bad_request(code), Cache::Public(60))
}

/// The reply of a `rate_limited` envelope.
pub fn rate_limited_reply(retry_after_seconds: u64) -> Reply {
    let mut r = Reply::json(429, rate_limited(retry_after_seconds), Cache::NoStore);
    r.retry_after = Some(retry_after_seconds);
    r
}
