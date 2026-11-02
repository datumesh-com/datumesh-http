//! Envelopes (SPEC 5.5) as the mesh writes them, and [`Reply`]: an HTTP answer before it is
//! an HTTP response.
//!
//! Every envelope is written as canonical JSON (JCS), so the same statement always has the
//! same bytes (a call and a cell URI for one cell give byte-identical bodies). The embedded
//! receipt is the stored receipt, unchanged.

pub use axum::body::Bytes;
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
    pub body: Bytes,
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
    pub fn json(status: u16, body: impl Into<Bytes>, cache: Cache) -> Reply {
        Reply { status, body: body.into(), content_type: JSON, cache, canonical: None, retry_after: None }
    }

    /// The envelope's `outcome`, when the body is an envelope the mesh wrote. (Its members
    /// are in JCS order and none before `outcome` holds text, so the first `"outcome":` is
    /// the envelope's own.)
    pub fn outcome(&self) -> Option<String> {
        self.outcome_str().map(str::to_string)
    }

    /// As [`Reply::outcome`], borrowed. The first `"outcome":"` starts at a quote, so only
    /// the body's quotes are tried (a byte search), not every position: since spec-0.4.2 the
    /// members before `outcome` include `fn_hash` (64 hex digits), and a batch looks up the
    /// outcome of each of its (up to 1,000) envelopes.
    pub(crate) fn outcome_str(&self) -> Option<&str> {
        if self.content_type != JSON {
            return None;
        }
        const KEY: &[u8] = b"\"outcome\":\"";
        let body: &[u8] = &self.body;
        let mut from = 0;
        let at = loop {
            let q = from + body[from..].iter().position(|b| *b == b'"')?;
            if body[q..].starts_with(KEY) {
                break q + KEY.len();
            }
            from = q + 1;
        };
        let end = body[at..].iter().position(|b| *b == b'"')?;
        std::str::from_utf8(&body[at..at + end]).ok()
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

/// The parse codes a `bad_request` may carry at `/cell` (SPEC 5.5; `E_CANON_HEX` for a pin since
/// spec-0.4.2).
pub const BAD_REQUEST_CODES: &[ErrorCode] = &[
    ErrorCode::CanonHex,
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

/// Whether `code` may be a `bad_request`'s `error` at `/cell` (the parse codes, and
/// `E_SLICE_AT_CELL`).
pub fn is_request_code(code: ErrorCode) -> bool {
    BAD_REQUEST_CODES.contains(&code) || code == ErrorCode::SliceAtCell
}

/// Whether `code` may be an `error` envelope's `error`: any code of 10.2 but `E_ENVELOPE` and
/// the request codes `E_JOB_UNKNOWN`, `E_SLICE_AT_CELL` and `E_NAME_UNBOUND`.
pub fn is_error_code(code: ErrorCode) -> bool {
    !matches!(code, ErrorCode::Envelope | ErrorCode::JobUnknown | ErrorCode::SliceAtCell | ErrorCode::NameUnbound)
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

/// The trust label of spec-0.4.2 for a receipt this mesh serves (SPEC 5.5): `primary` exactly
/// when the signer's host is the namespace's host and there are no inputs; `computed` when
/// this mesh signed it (it evaluated the cell); `unverified` otherwise (this mesh does not
/// recompute other meshes' receipts).
pub fn trust_label_of(receipt: &Receipt, mesh_host: &str) -> datumesh_core::TrustLabel {
    let ns_host = receipt.cell().ns.split('/').next().unwrap_or("");
    if receipt.signer_host() == ns_host && receipt.inputs().is_empty() {
        datumesh_core::TrustLabel::Primary
    } else if receipt.signer_host() == mesh_host {
        datumesh_core::TrustLabel::Computed
    } else {
        datumesh_core::TrustLabel::Unverified
    }
}

/// The reply of an answer built with the core's envelope builder (spec-0.4.2), with its
/// status.
pub fn answer_reply(answer: &datumesh_core::Answer, cache: Cache) -> Reply {
    Reply::json(answer.status(), answer.to_json(), cache)
}

/// An answer with the function hash echoed (SPEC 5.5: required for a pinned request, allowed
/// otherwise), when one is given.
pub fn with_fn_hash(answer: datumesh_core::Answer, fn_hash: Option<&str>) -> datumesh_core::Answer {
    match fn_hash {
        Some(h) => answer.clone().with_fn_hash(h).unwrap_or(answer),
        None => answer,
    }
}

/// The reply of an `error` answer with `code`, the function hash echoed.
pub fn error_answer_reply(code: ErrorCode, fn_hash: Option<&str>) -> Reply {
    match datumesh_core::Answer::error(code) {
        Ok(a) => {
            let a = with_fn_hash(a, fn_hash).with_status(error_status(code));
            answer_reply(&a, Cache::NoStore)
        }
        Err(_) => error_reply(ErrorCode::EvalUnavailable),
    }
}

/// The reply of a `not_covered` answer with `E_NAME_UNBOUND` (spec-0.4.2).
pub fn name_unbound_reply(max_age: u64) -> Reply {
    match datumesh_core::Answer::not_covered().with_error(ErrorCode::NameUnbound) {
        Ok(a) => answer_reply(&a, Cache::Public(max_age)),
        Err(_) => Reply::json(404, not_covered(None), Cache::Public(max_age)),
    }
}
