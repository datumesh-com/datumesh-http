//! What a request names: a cell URI's `<rest>` (SPEC 5.3) or a call (SPEC 5.8), resolved to
//! one canonical reference, pinned to a function hash or not (spec-0.4.2).
//!
//! # The forms `/cell` accepts
//!
//! * **A cell URI's rest**, byte for byte (SPEC 5.3):
//!   `/cell/ecb.europa.eu/fx_rate?base=EUR&date=2024-03-15&quote=USD`, then optionally
//!   `&fn_hash=<64 hex digits>` (a pin, 5.1) and `&asof=<instant>`. It is parsed by the core's
//!   `parse_reference` against the signature of the function the pin names, or else the one
//!   the name is bound to as of that asof (or now), so every code of SPEC 5.4 is reported as it
//!   defines.
//! * **A call** (5.8), percent-encoded as any URL path is
//!   (`/cell/rates.example.org/fx_rate(%22EUR%22,%20%22USD%22,%202024-03-15)`), with an optional
//!   query `?asof=<instant>` and nothing else: a read of the formula dialect whose arguments
//!   are literals, the callee a full name or an `@alias` of the configured map, optionally
//!   pinned (`name@<fn_hash>(...)`). It is parsed by the formula crate's
//!   `datumesh_formula::call::parse_call` in 5.8's order and codes, written as its cell URI and
//!   resolved as above: a call and its URI are one request.
//!
//!   Not accepted (reserved by SPEC 15.15 for classes, 1.3b): method chaining
//!   (`f(...).g(...)`), key literals (`EUR`, `1M`) and class-typed keys; nor expressions,
//!   ranges or units: `E_PARSE_URI`.
//!
//! An unbound name (or alias) is `not_covered` with `E_NAME_UNBOUND`
//! ([`Rejection::Error`]`(ErrorCode::NameUnbound)`, which the mesh answers so); a pin no
//! descriptor of the mesh has is a plain `not_covered` ([`Rejection::NotCovered`]); a slice
//! marker at `/cell` is `E_SLICE_AT_CELL`.

use std::collections::BTreeMap;
use std::sync::Arc;

use datumesh_core::json::Value;
use datumesh_core::{
    format_reference, lexical, parse_reference, Arg, Cell, CellRef, ErrorCode, FunctionDescriptor, Reference,
};

/// A function bound to a name as of an instant.
#[derive(Clone, Debug)]
pub struct Bound {
    /// The function hash.
    pub fn_hash: String,
    /// When the binding began.
    pub since: String,
    /// The descriptor.
    pub descriptor: Arc<FunctionDescriptor>,
    /// The descriptor's signature as JSON (what `parse_uri` reads).
    pub signature: Arc<Value>,
}

impl Bound {
    /// A binding of `fn_hash` since `since` to `descriptor`.
    pub fn new(fn_hash: impl Into<String>, since: impl Into<String>, descriptor: Arc<FunctionDescriptor>) -> Bound {
        let signature = Arc::new(descriptor.signature.to_value());
        Bound { fn_hash: fn_hash.into(), since: since.into(), descriptor, signature }
    }
}

/// Where names are looked up: the mesh's store (and, for names under other hosts, the
/// cross-mesh resolver in front of it).
pub trait Bindings {
    /// The binding of `fn_name` as of `asof`; `Ok(None)` when it has none. An `Err` is the
    /// code of an `error` envelope (the store or another mesh failed).
    fn binding(&self, fn_name: &str, asof: &str) -> Result<Option<Arc<Bound>>, ErrorCode>;

    /// A target resolved before for the cell URI rest `rest` under function hash `fn_hash`
    /// (parsing depends on nothing else). Provided: none.
    fn cached(&self, fn_hash: &str, rest: &str) -> Option<Arc<Target>> {
        let _ = (fn_hash, rest);
        None
    }

    /// Remembers a resolved target for [`Bindings::cached`]. Provided: nothing.
    fn remember(&self, rest: &str, target: &Arc<Target>) {
        let _ = (rest, target);
    }

    /// The descriptor whose function hash is `fn_hash` (a pin, spec-0.4.2), as a binding of
    /// no time (`since` empty); `Ok(None)` when the mesh has none. Provided: none.
    fn pinned(&self, fn_hash: &str) -> Result<Option<Arc<Bound>>, ErrorCode> {
        let _ = fn_hash;
        Ok(None)
    }

    /// The fully qualified name an alias (without `@`) stands for. Provided: none.
    fn alias(&self, alias: &str) -> Option<String> {
        let _ = alias;
        None
    }
}

/// A request resolved to its cell (or slice, at `/slice`).
#[derive(Clone, Debug)]
pub struct Target {
    /// The canonical cell URI, `asof` included when the request had one.
    pub uri: String,
    /// The reference (`asof` included).
    pub reference: CellRef,
    /// The cell, with the function hash the name is bound to as of the asof.
    pub cell: Cell,
    id: std::sync::OnceLock<String>,
    key: std::sync::OnceLock<String>,
    /// The binding (for a pinned request, the pinned descriptor).
    pub bound: Arc<Bound>,
    /// The pin (spec-0.4.2): the function hash the request named, if it named one.
    pub pin: Option<String>,
}

impl Target {
    /// The function's fully qualified name.
    pub fn fn_name(&self) -> String {
        format!("{}/{}", self.reference.ns, self.reference.fn_name)
    }

    /// The cell's id (SPEC 4.4), computed once.
    pub fn cell_id(&self) -> &str {
        self.id.get_or_init(|| self.cell.id())
    }

    /// The cell's key: its function hash and its URI without `asof` (computed once).
    pub fn cell_key(&self) -> &str {
        self.key.get_or_init(|| {
            let rest = self.rest();
            let base = match self.asof() {
                Some(a) => &rest[..rest.len() - a.len() - "asof=".len() - 1],
                None => rest,
            };
            format!("{} {}", self.bound.fn_hash, base)
        })
    }

    /// The reference with its pin (spec-0.4.2): what the request denotes.
    pub fn pinned_reference(&self) -> Reference {
        let mut r = Reference::from(self.reference.clone());
        r.fn_hash = self.pin.clone();
        r
    }

    /// The requested asof, if any.
    pub fn asof(&self) -> Option<&str> {
        self.reference.asof.as_deref()
    }

    /// The cell URI's `<rest>` (SPEC 5.3), without `cell://`.
    pub fn rest(&self) -> &str {
        &self.uri["cell://".len()..]
    }

    /// The rest with `asof` set to `asof` (replacing one there).
    pub fn rest_asof(&self, asof: &str) -> String {
        let rest = self.rest();
        let base = match self.reference.asof.as_deref() {
            // `asof` is the last parameter of a canonical URI.
            Some(a) => &rest[..rest.len() - a.len() - "asof=".len() - 1],
            None => rest,
        };
        let sep = if base.contains('?') { '&' } else { '?' };
        format!("{base}{sep}asof={asof}")
    }
}

/// Why a request names no cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// `bad_request` with this code.
    BadRequest(ErrorCode),
    /// `not_covered`: the mesh does not serve the function (a pin no descriptor of it has).
    NotCovered,
    /// `error` with this code: looking the name up failed. `ErrorCode::NameUnbound` stands
    /// for an unbound name, answered `not_covered` with that code (spec-0.4.2; this enum is
    /// exhaustive, so 0.1.1 gives that case no variant of its own).
    Error(ErrorCode),
}

/// Whether `rest` (the text after `/cell/`, query included, as received) is a call rather
/// than a cell URI's rest: its path (before the first `?`) has a `(` or a `%`, or starts with
/// `@`, none of which a cell URI's path (namespace and name) has.
pub fn is_call(rest: &str) -> bool {
    let path = rest.split('?').next().unwrap_or("");
    path.contains('(') || path.contains('%') || path.starts_with('@')
}

/// Resolves a request: a call or a cell URI's rest (see the module documentation). `now` is
/// the asof used to look the name up when the request has none. `slice` allows `*` (and
/// requires one).
pub fn resolve(
    rest: &str,
    now: &str,
    aliases: &BTreeMap<String, String>,
    bindings: &dyn Bindings,
    slice: bool,
) -> Result<Arc<Target>, Rejection> {
    if is_call(rest) {
        resolve_call(rest, now, aliases, bindings, slice)
    } else {
        resolve_uri(rest, now, bindings, slice)
    }
}

/// The parameters of a URI's query, split leniently (for the asof, the pin and a synthetic
/// signature; `parse_reference` checks them properly).
fn params(query: &str) -> Vec<(&str, &str)> {
    if query.is_empty() {
        return vec![];
    }
    query.split('&').map(|p| p.split_once('=').unwrap_or((p, ""))).collect()
}

/// The codes `parse_reference` reports before it reads the signature, or independently of
/// the arguments' types: what a URI whose function the mesh does not know can still be
/// refused with.
fn structural(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::ParseUri
            | ErrorCode::CanonName
            | ErrorCode::CanonArgName
            | ErrorCode::CanonArgDuplicate
            | ErrorCode::CanonArgOrder
            | ErrorCode::CanonPercent
            | ErrorCode::CanonInstant
            | ErrorCode::CanonHex
    )
}

/// Resolves a cell URI's rest.
pub fn resolve_uri(rest: &str, now: &str, bindings: &dyn Bindings, slice: bool) -> Result<Arc<Target>, Rejection> {
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };
    let ps = params(query.unwrap_or(""));
    // The asof the name is looked up at: the last parameter's, when it is an instant; the pin,
    // the last remaining one's when it is named fn_hash and is 64 lowercase hex digits.
    let mut rest_ps: &[(&str, &str)] = &ps;
    let mut asof = now;
    if let Some((("asof", v), before)) = rest_ps.split_last() {
        if lexical::check_instant(v, "$").is_ok() {
            asof = v;
        }
        rest_ps = before;
    }
    let pin = match rest_ps.last() {
        Some(("fn_hash", v)) if lexical::check_hex(v, 64, "$").is_ok() => Some(v.to_string()),
        _ => None,
    };
    let name_ok = lexical::check_function_name(path, "$").is_ok() && !path.contains('#');
    let bound = match (&pin, name_ok) {
        (Some(p), true) => bindings.pinned(p).map_err(Rejection::Error)?,
        (None, true) => bindings.binding(path, asof).map_err(Rejection::Error)?,
        (_, false) => None,
    };
    if let Some(b) = &bound {
        if let Some(t) = bindings.cached(&b.fn_hash, rest) {
            if t.reference.args.values().any(|a| matches!(a, Arg::Slice)) == slice {
                return Ok(t);
            }
        }
    }
    let uri = format!("cell://{rest}");
    let Some(bound) = bound else {
        // No function to parse against: report what the URI's form alone shows (every
        // argument read as a string), else not_covered: the name unbound, or the pin unknown.
        let mut sig = Vec::new();
        for (name, _) in &ps {
            if *name != "asof" && *name != "fn_hash" {
                sig.push((name.to_string(), Value::Object(vec![("type".into(), Value::String("string".into()))])));
            }
        }
        let signature = Value::Object(vec![("args".into(), Value::Object(sig))]);
        return match parse_reference(&uri, &signature) {
            Err(e) if structural(e.code()) => Err(Rejection::BadRequest(e.code())),
            _ if pin.is_some() || !name_ok => Err(Rejection::NotCovered),
            _ => Err(Rejection::Error(ErrorCode::NameUnbound)),
        };
    };
    let reference = parse_reference(&uri, &bound.signature).map_err(|e| {
        if crate::envelope::is_request_code(e.code()) {
            Rejection::BadRequest(e.code())
        } else {
            Rejection::Error(ErrorCode::EvalUnavailable)
        }
    })?;
    let has_slice = reference.args.values().any(|a| matches!(a, Arg::Slice));
    if has_slice && !slice {
        return Err(Rejection::BadRequest(ErrorCode::SliceAtCell));
    }
    if !has_slice && slice {
        // A cell at the slice endpoint: the request names no slice.
        return Err(Rejection::BadRequest(ErrorCode::ParseUri));
    }
    let cell = Cell {
        ns: reference.ns.clone(),
        fn_name: reference.fn_name.clone(),
        fn_hash: bound.fn_hash.clone(),
        args: reference.args.clone(),
    };
    let pin = reference.fn_hash.clone();
    let cell_ref = CellRef { ns: reference.ns, fn_name: reference.fn_name, args: reference.args, asof: reference.asof };
    // format(parse(uri)) = uri (SPEC 5.4): the accepted URI is the canonical one.
    let t = Arc::new(Target {
        uri,
        reference: cell_ref,
        cell,
        id: std::sync::OnceLock::new(),
        key: std::sync::OnceLock::new(),
        bound,
        pin,
    });
    bindings.remember(rest, &t);
    Ok(t)
}

/// Percent-decodes a URL path (any `%XX`, either case; the result must be UTF-8).
fn url_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// [`Bindings`] as the formula crate's call environment: aliases from the mesh's map, names as
/// of the call's asof (or now), descriptors by pin.
struct Env<'a> {
    bindings: &'a dyn Bindings,
    aliases: &'a BTreeMap<String, String>,
    now: &'a str,
}

impl datumesh_formula::call::CallEnv for Env<'_> {
    fn alias(&self, alias: &str) -> Result<Option<String>, String> {
        Ok(self.aliases.get(alias).cloned().or_else(|| self.bindings.alias(alias)))
    }
    fn bound(&self, name: &str, asof: Option<&str>) -> Result<Option<FunctionDescriptor>, String> {
        match self.bindings.binding(name, asof.unwrap_or(self.now)) {
            Ok(b) => Ok(b.map(|b| (*b.descriptor).clone())),
            Err(c) => Err(c.as_str().to_string()),
        }
    }
    fn pinned(&self, fn_hash: &str) -> Result<Option<FunctionDescriptor>, String> {
        match self.bindings.pinned(fn_hash) {
            Ok(b) => Ok(b.map(|b| (*b.descriptor).clone())),
            Err(c) => Err(c.as_str().to_string()),
        }
    }
}

/// Resolves a call (SPEC 5.8).
pub fn resolve_call(
    rest: &str,
    now: &str,
    aliases: &BTreeMap<String, String>,
    bindings: &dyn Bindings,
    slice: bool,
) -> Result<Arc<Target>, Rejection> {
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };
    let mut text = url_decode(path).ok_or(Rejection::BadRequest(ErrorCode::CanonPercent))?;
    if let Some(q) = query {
        text.push('?');
        text.push_str(q);
    }
    let env = Env { bindings, aliases, now };
    let call = datumesh_formula::call::parse_call(text.trim_start(), &env).map_err(|e| match e {
        datumesh_formula::call::CallError::Code(c, _) => match ErrorCode::from_code(c) {
            Some(ErrorCode::NameUnbound) => Rejection::Error(ErrorCode::NameUnbound),
            Some(ErrorCode::FnUnavailable) => Rejection::NotCovered,
            Some(code) if crate::envelope::is_request_code(code) => Rejection::BadRequest(code),
            _ => Rejection::BadRequest(ErrorCode::ParseUri),
        },
        datumesh_formula::call::CallError::Env(m) => {
            Rejection::Error(ErrorCode::from_code(&m).unwrap_or(ErrorCode::InputUnavailable))
        }
        _ => Rejection::Error(ErrorCode::EvalUnavailable),
    })?;
    let uri = call.uri();
    if !slice && call.cell.args.values().any(|a| matches!(a, Arg::Slice)) {
        return Err(Rejection::BadRequest(ErrorCode::SliceAtCell));
    }
    resolve_uri(&uri["cell://".len()..], now, bindings, slice)
}

/// The URI of a reference with its pin (core's `format_reference`).
pub fn format_pinned(reference: &Reference) -> String {
    format_reference(reference)
}
