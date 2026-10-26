//! What a request names: a cell URI's `<rest>` (SPEC 5.3) or a call (SPEC 15's read syntax),
//! resolved to one canonical reference.
//!
//! # The forms `/cell` accepts in 0.1.0
//!
//! * **A cell URI's rest**, byte for byte (SPEC 5.3): `/cell/ecb.europa.eu/fx_rate?base=EUR&date=2024-03-15&quote=USD`,
//!   optionally ending `&asof=<instant>` (or `?asof=` when there are no arguments). It is
//!   parsed by the core's `parse_uri` against the signature of the function the name is
//!   bound to as of that asof (or now), so every code of SPEC 5.4 is reported as it defines.
//! * **A call**: a read as in SPEC 15.3, percent-encoded as any URL path is
//!   (`/cell/rates.example.org/fx_rate(base:%20%22EUR%22,%20...)`), with an optional query
//!   `?asof=<instant>` and nothing else. The callee is a fully qualified name or an `@alias`
//!   of the configured alias map. Arguments are positional (bound by the callee's
//!   `signature.order`, SPEC 15.5) or labelled (`name: value`), and each value is a literal
//!   of the dialect: a number (a leading `-` allowed), a string in double quotes (quotes
//!   doubled), a date, an instant, `true` or `false`; for a blob argument, a string holding
//!   the blob's lexical form; `*` only at `/slice`. A call is parsed by the formula crate's
//!   parser itself, then written as its cell URI and parsed as above: a call and the URI of
//!   the same cell are the same request.
//!
//!   Not accepted in 0.1.0 (reserved by SPEC 15.15 for classes, 1.3b): method chaining
//!   (`f(...).g(...)`), key literals (`EUR`, `1M`) and class-typed keys; nor expressions,
//!   ranges, units on numbers or reads as arguments. They are `bad_request` with
//!   `E_PARSE_URI`, as is any other text that is not such a call.
//!
//! A call that does not parse is `E_PARSE_URI`; an unknown label `E_ARG_UNKNOWN`, a missing
//! argument `E_ARG_MISSING`, a label given twice `E_CANON_ARG_DUPLICATE`; a literal the
//! dialect's lexer refuses, or that is not canonical for its argument's type, that type's
//! code (`E_CANON_NUMBER`, `E_CANON_STRING`, `E_CANON_DATE`, `E_CANON_INSTANT`,
//! `E_CANON_BLOB`); a literal of the wrong kind for its argument `E_CANON_TYPE`. A
//! name, or an alias, that is not bound as of the asof is `not_covered`: the mesh does not
//! serve that function.

use std::collections::BTreeMap;
use std::sync::Arc;

use datumesh_core::json::{self, Value};
use datumesh_core::{
    format_uri, lexical, parse_uri, Arg, Args, Cell, CellRef, ErrorCode, FunctionDescriptor, Scalar, Type,
};
use datumesh_formula::{canonical_ast, Code, Env, Node, Op, ReadArg, Signature as FSignature};

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
    /// The binding.
    pub bound: Arc<Bound>,
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
    /// `not_covered`: the name (or alias) is not bound as of the asof.
    NotCovered,
    /// `error` with this code: looking the name up failed.
    Error(ErrorCode),
}

/// Whether `rest` (the text after `/cell/`, query included, as received) is a call rather
/// than a cell URI's rest: its path (before the first `?`) has a `(` or a `%`, which a cell
/// URI's path (namespace and name) never has.
pub fn is_call(rest: &str) -> bool {
    let path = rest.split('?').next().unwrap_or("");
    path.contains('(') || path.contains('%') || path.starts_with('@')
}

/// Resolves a request: a call or a cell URI's rest (see the module documentation). `now` is
/// the asof used to look the name up when the request has none. `slice` allows `*`.
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

/// The parameters of a URI's query, split leniently (for the asof and a synthetic
/// signature; `parse_uri` checks them properly).
fn params(query: &str) -> Vec<(&str, &str)> {
    if query.is_empty() {
        return vec![];
    }
    query.split('&').map(|p| p.split_once('=').unwrap_or((p, ""))).collect()
}

/// Resolves a cell URI's rest.
pub fn resolve_uri(rest: &str, now: &str, bindings: &dyn Bindings, slice: bool) -> Result<Arc<Target>, Rejection> {
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };
    // The asof the name is looked up at: the last parameter's, when it is an instant.
    let last = query.map(|q| q.rsplit('&').next().unwrap_or(q));
    let asof = match last.and_then(|p| p.strip_prefix("asof=")) {
        Some(v) if lexical::check_instant(v, "$").is_ok() => v,
        _ => now,
    };
    let bound = if lexical::check_function_name(path, "$").is_ok() && !path.contains('#') {
        bindings.binding(path, asof).map_err(Rejection::Error)?
    } else {
        None
    };
    if let Some(b) = &bound {
        if let Some(t) = bindings.cached(&b.fn_hash, rest) {
            if t.reference.args.values().any(|a| matches!(a, Arg::Slice)) == slice {
                return Ok(t);
            }
        }
    }
    let uri = format!("cell://{rest}");
    let ps = params(query.unwrap_or(""));
    let Some(bound) = bound else {
        // No function to parse against: report what the URI's form alone shows (every
        // argument read as a string), else not_covered.
        let mut sig = Vec::new();
        for (name, _) in &ps {
            if *name != "asof" {
                sig.push((name.to_string(), Value::Object(vec![("type".into(), Value::String("string".into()))])));
            }
        }
        let signature = Value::Object(vec![("args".into(), Value::Object(sig))]);
        return match parse_uri(&uri, &signature) {
            Err(e)
                if matches!(
                    e.code(),
                    ErrorCode::ParseUri
                        | ErrorCode::CanonName
                        | ErrorCode::CanonArgName
                        | ErrorCode::CanonArgDuplicate
                        | ErrorCode::CanonArgOrder
                        | ErrorCode::CanonPercent
                        | ErrorCode::CanonInstant
                ) =>
            {
                Err(Rejection::BadRequest(e.code()))
            }
            _ => Err(Rejection::NotCovered),
        };
    };
    let reference = parse_uri(&uri, &bound.signature).map_err(|e| {
        if crate::envelope::is_request_code(e.code()) {
            Rejection::BadRequest(e.code())
        } else {
            Rejection::Error(ErrorCode::EvalUnavailable)
        }
    })?;
    let has_slice = reference.args.values().any(|a| matches!(a, Arg::Slice));
    if has_slice != slice {
        // A slice at /cell, or a cell at /slice: the request names no single cell (or no
        // slice). SPEC 5.2: omission is an error; a slice marker where a cell is asked is not
        // a cell URI of this endpoint.
        return Err(Rejection::BadRequest(ErrorCode::ParseUri));
    }
    let cell = Cell {
        ns: reference.ns.clone(),
        fn_name: reference.fn_name.clone(),
        fn_hash: bound.fn_hash.clone(),
        args: reference.args.clone(),
    };
    // format(parse(uri)) = uri (SPEC 5.4): the accepted URI is the canonical one.
    let t = Arc::new(Target {
        uri,
        reference,
        cell,
        id: std::sync::OnceLock::new(),
        key: std::sync::OnceLock::new(),
        bound,
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

fn formula_code(c: Code) -> ErrorCode {
    match c {
        Code::ArgUnknown => ErrorCode::ArgUnknown,
        Code::ArgMissing => ErrorCode::ArgMissing,
        Code::ArgDuplicate => ErrorCode::CanonArgDuplicate,
        Code::Number => ErrorCode::CanonNumber,
        Code::String => ErrorCode::CanonString,
        Code::Date => ErrorCode::CanonDate,
        Code::Instant => ErrorCode::CanonInstant,
        _ => ErrorCode::ParseUri,
    }
}

/// The lexical form of a literal argument for an argument of type `ty`.
fn literal(ast: &datumesh_formula::Ast, id: datumesh_formula::NodeId, ty: Type) -> Result<Scalar, ErrorCode> {
    let (kind, text) = match ast.node(id) {
        Node::Num { value, unit: None } => (Type::Number, value.clone()),
        Node::Op { op: Op::Neg, args } if args.len() == 1 => match ast.node(args[0]) {
            Node::Num { value, unit: None } => (Type::Number, format!("-{value}")),
            _ => return Err(ErrorCode::ParseUri),
        },
        Node::Str(s) => (if ty == Type::Blob { Type::Blob } else { Type::String }, s.clone()),
        Node::Bool(b) => (Type::Boolean, b.to_string()),
        Node::Date(d) => (Type::Date, d.clone()),
        Node::Instant(t) => (Type::Instant, t.clone()),
        _ => return Err(ErrorCode::ParseUri),
    };
    if kind != ty {
        return Err(ErrorCode::CanonType);
    }
    Scalar::from_lexical(ty, &text).map_err(|e| e.code())
}

/// Resolves a call.
pub fn resolve_call(
    rest: &str,
    now: &str,
    aliases: &BTreeMap<String, String>,
    bindings: &dyn Bindings,
    slice: bool,
) -> Result<Arc<Target>, Rejection> {
    let bad = Rejection::BadRequest(ErrorCode::ParseUri);
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };
    let asof = match query {
        None => None,
        Some(q) => match q.strip_prefix("asof=") {
            Some(v) if !v.contains('&') => {
                lexical::check_instant(v, "$").map_err(|e| Rejection::BadRequest(e.code()))?;
                Some(v.to_string())
            }
            _ => return Err(bad),
        },
    };
    let text = url_decode(path).ok_or(Rejection::BadRequest(ErrorCode::CanonPercent))?;
    let open = text.find('(').ok_or(bad)?;
    let callee = text[..open].trim();
    let name = match callee.strip_prefix('@') {
        Some(alias) => match aliases.get(alias) {
            Some(n) => n.clone(),
            None => return Err(Rejection::NotCovered),
        },
        None => {
            lexical::check_function_name(callee, "$").map_err(|_| Rejection::BadRequest(ErrorCode::CanonName))?;
            callee.to_string()
        }
    };
    let at = asof.as_deref().unwrap_or(now);
    let bound = bindings.binding(&name, at).map_err(Rejection::Error)?.ok_or(Rejection::NotCovered)?;
    // The environment: the callee's descriptor (for positional arguments and labels) and the
    // aliases, under a name no call can read.
    let sig = FSignature::from_value(
        &json::parse(br#"{"args":{},"returns":{"type":"number","unit":"EUR"}}"#)
            .map_err(|_| Rejection::Error(ErrorCode::EvalUnavailable))?,
    )
    .map_err(|_| Rejection::Error(ErrorCode::EvalUnavailable))?;
    let mut env = Env::new("datumesh.invalid/call", sig);
    env.add_function(&name, &bound.descriptor.to_value()).map_err(|_| Rejection::Error(ErrorCode::EvalUnavailable))?;
    for (a, n) in aliases {
        env.aliases.insert(a.clone(), n.clone());
    }
    let ast = canonical_ast(&text, Some(&env)).map_err(|e| Rejection::BadRequest(formula_code(e.code)))?;
    let Node::Read { fn_name, args } = ast.node(ast.root()) else {
        return Err(bad);
    };
    if *fn_name != name {
        return Err(bad);
    }
    let (ns, f) = name.rsplit_once('/').ok_or(bad)?;
    let mut out = Args::new();
    for (arg, a) in args {
        let ty = bound
            .descriptor
            .signature
            .args
            .get(arg)
            .map(|d| d.ty)
            .ok_or(Rejection::BadRequest(ErrorCode::ArgUnknown))?;
        let v = match a {
            ReadArg::Open if slice => Arg::Slice,
            ReadArg::Open | ReadArg::Range(..) => return Err(bad),
            ReadArg::Expr(id) => Arg::Value(literal(&ast, *id, ty).map_err(Rejection::BadRequest)?),
        };
        out.insert(arg.clone(), v);
    }
    let reference = CellRef { ns: ns.to_string(), fn_name: f.to_string(), args: out, asof };
    let uri = format_uri(&reference);
    resolve_uri(&uri["cell://".len()..], now, bindings, slice)
}
