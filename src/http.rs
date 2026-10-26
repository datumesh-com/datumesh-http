//! The endpoints: an axum router over a [`Mesh`].

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, MatchedPath, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use datumesh_core::json::{self, Value};
use datumesh_core::{lexical, ErrorCode, Pending, Receipt, Request as CoreRequest};
use tracing::Instrument;
use utoipa::OpenApi;

use crate::apikeys::Principal;
use crate::envelope::{self, Cache, Reply, JSON};
use crate::jobs::Poll;
use crate::limit::Client;
use crate::mesh::{Inner, Memo, Mesh};

/// The principal of a request, for the access log.
#[derive(Clone)]
struct Who(Option<String>);

fn to_response(r: Reply) -> Response {
    let mut resp = Response::new(Body::from(r.body));
    *resp.status_mut() = StatusCode::from_u16(r.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(r.content_type));
    if let Ok(v) = HeaderValue::from_str(&r.cache.header()) {
        h.insert(header::CACHE_CONTROL, v);
    }
    if let Some(c) = r.canonical {
        if let Ok(v) = HeaderValue::from_str(&format!("<{c}>; rel=\"canonical\"")) {
            h.insert(header::LINK, v);
        }
    }
    if let Some(s) = r.retry_after {
        h.insert(header::RETRY_AFTER, HeaderValue::from(s));
    }
    resp
}

fn plain_json(status: u16, v: Value, cache: Cache) -> Response {
    to_response(Reply::json(status, v.to_jcs(), cache))
}

fn not_found(what: &str) -> Response {
    plain_json(404, Value::Object(vec![("error".into(), Value::String(format!("no such {what}")))]), Cache::Public(60))
}

/// The client: its IP and, with a valid API key, its principal. An `Authorization` header
/// that is not a key of this mesh is answered `unlicensed`.
fn who(inner: &Inner, headers: &HeaderMap, peer: IpAddr) -> Result<(Client, Option<Principal>), Reply> {
    let ip = client_ip(inner, headers, peer);
    match headers.get(header::AUTHORIZATION) {
        None => Ok((Client::Ip(ip), None)),
        Some(v) => {
            let p = v
                .to_str()
                .ok()
                .and_then(|s| s.strip_prefix("Bearer "))
                .and_then(|k| inner.config.api_keys.principal(k.trim()));
            match p {
                Some(p) => Ok((Client::Key(p.id.clone(), p.per_minute), Some(p.clone()))),
                None => Err(Reply::json(403, envelope::unlicensed(), Cache::Private)),
            }
        }
    }
}

fn client_ip(inner: &Inner, headers: &HeaderMap, peer: IpAddr) -> IpAddr {
    if inner.config.trust_proxy {
        if let Some(ip) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.rsplit(',').next())
            .and_then(|s| s.trim().parse::<IpAddr>().ok())
        {
            return ip;
        }
    }
    peer
}

fn peer_of(req: &Request) -> IpAddr {
    req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip()).unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
}

/// `rest` of a `/<prefix>/<rest>` request, query included, as received.
fn rest_of(uri: &Uri, prefix: &str) -> String {
    let path = uri.path().strip_prefix(prefix).unwrap_or("");
    match uri.query() {
        Some(q) => format!("{path}?{q}"),
        None => path.to_string(),
    }
}

/// Runs `f` on the pool within the request's budget: its reply, or, past the budget, a
/// `pending` envelope whose job gets the reply when `f` finishes. `f` reports the request
/// description as soon as it knows it.
async fn within_budget(
    inner: Arc<Inner>,
    f: impl FnOnce(&Inner, &mut dyn FnMut(&crate::call::Target)) -> Reply + Send + 'static,
) -> Reply {
    let deadline = tokio::time::Instant::now() + inner.config.budget;
    let (tx, mut rx) = tokio::sync::watch::channel::<Option<CoreRequest>>(None);
    let pool = inner.pool.clone();
    let i2 = inner.clone();
    let mut task = tokio::spawn(async move {
        pool.run(move || {
            let mut on = |t: &crate::call::Target| {
                if let Ok(r) = CoreRequest::new(t.cell_id(), t.asof()) {
                    let _ = tx.send(Some(r));
                }
            };
            f(&i2, &mut on)
        })
        .await
    });
    match tokio::time::timeout_at(deadline, &mut task).await {
        Ok(Ok(Some(r))) => r,
        Ok(_) => envelope::error_reply(ErrorCode::EvalUnavailable),
        Err(_) => {
            let request = rx.borrow_and_update().clone();
            let Some(request) = request else {
                // Past the budget before the request was even resolved (the store or another
                // mesh is slow): the mesh cannot say what it is computing.
                return envelope::error_reply(ErrorCode::EvalUnavailable);
            };
            pend(inner, request, task)
        }
    }
}

/// A job for a request still running in `task`.
fn pend(inner: Arc<Inner>, request: CoreRequest, task: tokio::task::JoinHandle<Option<Reply>>) -> Reply {
    let Some(id) = inner.jobs.create(request.clone()) else {
        return envelope::error_reply(ErrorCode::EvalUnavailable);
    };
    let poll = format!("{}/jobs/{id}", inner.config.origin());
    let est = inner.jobs.estimate(inner.config.budget.as_secs());
    let i2 = inner.clone();
    let id2 = id.clone();
    tokio::spawn(async move {
        let r = match task.await {
            Ok(Some(r)) if r.status != 202 => r,
            _ => envelope::error_reply(ErrorCode::EvalUnavailable),
        };
        let mut r = r;
        r.cache = Cache::NoStore;
        r.canonical = None;
        i2.jobs.complete(&id2, r);
    });
    match Pending::new(&id, est, &poll, request) {
        Ok(p) => Reply::json(202, envelope::pending(&p), Cache::NoStore),
        Err(_) => envelope::error_reply(ErrorCode::EvalUnavailable),
    }
}

fn admit(inner: &Inner, client: &Client, cost: u32) -> Result<(), Reply> {
    inner.limiter.admit(client, cost).map_err(envelope::rate_limited_reply)
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "datumesh-mesh",
        description = "The Datumesh engine behind HTTPS (spec-0.4.1): cell envelopes (SPEC 5.5), batches, slices, receipts, descriptors, manifests, blobs, lineage, search, verification, jobs, key and mesh documents (SPEC 9.2, 9.6), bindings (SPEC 9.7).",
        license(name = "Apache-2.0")
    ),
    paths(
        cell,
        cells,
        slice,
        receipt,
        descriptor,
        manifest,
        blob,
        explain,
        find,
        verify_receipt,
        job,
        key_document,
        mesh_document,
        bindings,
        health,
        metrics,
        openapi_json
    )
)]
struct ApiDoc;

/// The OpenAPI document (generated from the handlers' annotations).
pub fn openapi() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}

/// The envelope of a cell.
#[utoipa::path(get, path = "/cell/{cell}",
    params(("cell" = String, Path, description = "A cell URI's rest (SPEC 5.3), byte for byte, `asof` included; or a call (SPEC 15.3's read syntax, URL-encoded) with an optional `?asof=`.")),
    responses(
        (status = 200, description = "`ok` or `stale`", content_type = "application/json"),
        (status = 202, description = "`pending`: poll the job (SPEC 5.5)", content_type = "application/json"),
        (status = 400, description = "`bad_request`", content_type = "application/json"),
        (status = 402, description = "`payment_required`", content_type = "application/json"),
        (status = 403, description = "`unlicensed`", content_type = "application/json"),
        (status = 404, description = "`not_covered` (with `fault` when the evaluation faulted)", content_type = "application/json"),
        (status = 406, description = "the Accept header excludes JSON"),
        (status = 429, description = "`rate_limited`, with `Retry-After`", content_type = "application/json"),
        (status = 500, description = "`error`", content_type = "application/json"),
        (status = 503, description = "`error`: the store or another mesh is unavailable", content_type = "application/json"),
    ))]
async fn cell(State(mesh): State<Mesh>, uri: Uri, headers: HeaderMap, req: Request) -> Response {
    let inner = mesh.inner.clone();
    let (client, principal) = match who(&inner, &headers, peer_of(&req)) {
        Ok(x) => x,
        Err(r) => return to_response(r),
    };
    let mut resp = match admit(&inner, &client, 1) {
        Err(r) => to_response(r),
        Ok(()) => {
            let rest = rest_of(&uri, "/cell/");
            let p = principal.clone();
            let r = within_budget(inner.clone(), move |i, on| {
                let now = i.now();
                i.answer(&rest, p.as_ref(), i, &now, on)
            })
            .await;
            if let Some(o) = r.outcome() {
                inner.metrics.outcome("/cell/{cell}", &o);
            }
            to_response(r)
        }
    };
    resp.extensions_mut().insert(Who(principal.map(|p| p.id)));
    resp
}

/// A batch of cells.
#[utoipa::path(post, path = "/cells",
    request_body(content = String, description = "A JSON array of requests, each a call or a cell URI (`cell://...`) as a string, or `{\"cell\": <that>}`; at most the configured bound (1,000).", content_type = "application/json"),
    responses(
        (status = 200, description = "A JSON array of envelopes, in order; each has the HTTP status its outcome has (`error`: 500)", content_type = "application/json"),
        (status = 400, description = "Not such an array, or too long", content_type = "application/json"),
        (status = 429, description = "`rate_limited` (a batch of n cells counts as n / 50 requests)", content_type = "application/json"),
    ))]
async fn cells(State(mesh): State<Mesh>, headers: HeaderMap, req: Request) -> Response {
    let inner = mesh.inner.clone();
    let peer = peer_of(&req);
    let (client, principal) = match who(&inner, &headers, peer) {
        Ok(x) => x,
        Err(r) => return to_response(r),
    };
    let body = match axum::body::to_bytes(req.into_body(), 64 << 20).await {
        Ok(b) => b,
        Err(_) => return batch_error("body too large"),
    };
    let items = match parse_batch(&body, inner.config.batch_max) {
        Ok(i) => i,
        Err(e) => return batch_error(&e),
    };
    if let Err(r) = admit(&inner, &client, inner.limiter.batch_cost(items.len())) {
        return to_response(r);
    }
    let replies = batch(inner.clone(), items, principal.clone()).await;
    let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for r in &replies {
        if let Some(o) = r.outcome() {
            *counts.entry(o).or_default() += 1;
        }
    }
    for (o, k) in counts {
        inner.metrics.outcomes("/cells", &o, k);
    }
    let mut out = Vec::with_capacity(replies.iter().map(|r| r.body.len() + 1).sum::<usize>() + 2);
    out.push(b'[');
    for (i, r) in replies.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        out.extend_from_slice(&r.body);
    }
    out.push(b']');
    let mut resp =
        to_response(Reply::json(200, out, if principal.is_some() { Cache::Private } else { Cache::NoStore }));
    resp.extensions_mut().insert(Who(principal.map(|p| p.id)));
    resp
}

fn batch_error(e: &str) -> Response {
    plain_json(400, Value::Object(vec![("error".into(), Value::String(e.to_string()))]), Cache::NoStore)
}

fn parse_batch(body: &[u8], max: usize) -> Result<Vec<String>, String> {
    // The body is a list of requests, not a protocol object: plain JSON.
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|e| format!("not JSON: {e}"))?;
    let serde_json::Value::Array(items) = v else { return Err("not a JSON array".into()) };
    if items.len() > max {
        return Err(format!("{} requests: at most {max}", items.len()));
    }
    items
        .into_iter()
        .map(|i| {
            let s = match i {
                serde_json::Value::String(s) => s,
                serde_json::Value::Object(mut o) => match o.remove("cell") {
                    Some(serde_json::Value::String(s)) => s,
                    _ => return Err("an object without a string cell".to_string()),
                },
                _ => return Err("an item that is neither a string nor an object".to_string()),
            };
            Ok(match s.strip_prefix("cell://") {
                Some(r) => r.to_string(),
                None => s,
            })
        })
        .collect()
}

/// Answers a batch within the budget: the requests are answered in chunks across the pool
/// (one chunk per core; bindings looked up once per name and asof within a chunk). Those not
/// answered within the budget become jobs: each is resolved again to the request it
/// describes (a store read), and its chunk's answer completes the job.
async fn batch(inner: Arc<Inner>, items: Vec<String>, principal: Option<Principal>) -> Vec<Reply> {
    let n = items.len();
    let deadline = tokio::time::Instant::now() + inner.config.budget;
    let cores = std::thread::available_parallelism().map(|c| c.get()).unwrap_or(4);
    let chunks = n.div_ceil(64).clamp(1, inner.config.pool_size.min(cores));
    let per = n.div_ceil(chunks).max(1);
    let now = inner.now();
    let items = Arc::new(items);
    // Each chunk's answers so far; once the budget has passed (`late`), its answers go to
    // the channel instead, to complete their jobs.
    let shared: Arc<Vec<Mutex<(bool, Vec<(usize, Reply)>)>>> =
        Arc::new((0..chunks).map(|_| Mutex::new((false, Vec::new()))).collect());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(usize, Reply)>();
    let seen = Arc::new(crate::mesh::Seen::default());
    let mut tasks = Vec::new();
    for c in 0..chunks {
        let (lo, hi) = (c * per, ((c + 1) * per).min(n));
        if lo >= hi {
            break;
        }
        let (i2, items, tx, p, now, shared) =
            (inner.clone(), items.clone(), tx.clone(), principal.clone(), now.clone(), shared.clone());
        let seen = seen.clone();
        let pool = inner.pool.clone();
        tasks.push(tokio::spawn(async move {
            pool.run(move || {
                let memo = Memo::shared(&i2, &seen);
                for k in lo..hi {
                    let reply = i2.answer_with(&items[k], p.as_ref(), &memo, &now, &mut |_| {}, false);
                    let mut slot = shared[c].lock().unwrap_or_else(|p| p.into_inner());
                    if slot.0 {
                        let _ = tx.send((k, reply));
                    } else {
                        slot.1.push((k, reply));
                    }
                }
            })
            .await
        }));
    }
    drop(tx);
    let all = futures_join(&mut tasks, deadline).await;
    let mut done: Vec<Option<Reply>> = vec![None; n];
    for slot in shared.iter() {
        let mut s = slot.lock().unwrap_or_else(|p| p.into_inner());
        s.0 = true;
        for (k, r) in s.1.drain(..) {
            done[k] = Some(r);
        }
    }
    if all {
        return done
            .into_iter()
            .map(|d| d.unwrap_or_else(|| envelope::error_reply(ErrorCode::EvalUnavailable)))
            .collect();
    }
    // Past the budget: every request not answered yet becomes a job, resolved again to the
    // request it describes.
    let open: Vec<usize> = (0..n).filter(|k| done[*k].is_none()).collect();
    let (i2, items2, open2, now2) = (inner.clone(), items.clone(), open.clone(), now.clone());
    let requests: Vec<Option<CoreRequest>> = inner
        .pool
        .run(move || {
            let memo = Memo::new(&i2);
            open2
                .iter()
                .map(|k| {
                    crate::call::resolve(&items2[*k], &now2, &i2.config.aliases, &memo, false)
                        .ok()
                        .and_then(|t| CoreRequest::new(t.cell_id(), t.asof()).ok())
                })
                .collect()
        })
        .await
        .unwrap_or_default();
    let poll_base = format!("{}/jobs/", inner.config.origin());
    let est = inner.jobs.estimate(inner.config.budget.as_secs());
    let mut jobs: Vec<Option<String>> = vec![None; n];
    for (i, k) in open.iter().enumerate() {
        let reply = match requests.get(i).cloned().flatten() {
            Some(req) => match inner.jobs.create(req.clone()) {
                Some(id) => {
                    let p = Pending::new(&id, est, &format!("{poll_base}{id}"), req);
                    jobs[*k] = Some(id);
                    match p {
                        Ok(p) => Reply::json(202, envelope::pending(&p), Cache::NoStore),
                        Err(_) => envelope::error_reply(ErrorCode::EvalUnavailable),
                    }
                }
                None => envelope::error_reply(ErrorCode::EvalUnavailable),
            },
            None => envelope::error_reply(ErrorCode::EvalUnavailable),
        };
        done[*k] = Some(reply);
    }
    let i3 = inner.clone();
    tokio::spawn(async move {
        while let Some((k, mut reply)) = rx.recv().await {
            if let Some(id) = &jobs[k] {
                reply.cache = Cache::NoStore;
                reply.canonical = None;
                i3.jobs.complete(id, reply);
            }
        }
        for t in tasks {
            let _ = t.await;
        }
        // Jobs whose task ended without an answer (a panic) are given up.
        for id in jobs.into_iter().flatten() {
            if let Poll::Running(..) = i3.jobs.poll_peek(&id) {
                i3.jobs.complete(&id, envelope::error_reply(ErrorCode::EvalUnavailable));
            }
        }
    });
    done.into_iter().map(|d| d.unwrap_or_else(|| envelope::error_reply(ErrorCode::EvalUnavailable))).collect()
}

/// Waits for every task until `deadline`: whether all finished.
async fn futures_join(tasks: &mut [tokio::task::JoinHandle<Option<()>>], deadline: tokio::time::Instant) -> bool {
    for t in tasks.iter_mut() {
        if t.is_finished() {
            continue;
        }
        if tokio::time::timeout_at(deadline, t).await.is_err() {
            return false;
        }
    }
    true
}

/// The members of a slice.
#[utoipa::path(get, path = "/slice/{cell}",
    params(
        ("cell" = String, Path, description = "A slice: a cell URI's rest or a call with `*` for open arguments, of a stored function; `?asof=`, `limit=` (JSON, default 1,000, at most 10,000), `after=` (the cursor of the previous page)."),
    ),
    responses(
        (status = 200, description = "JSON page of members, or with `Accept: application/vnd.apache.arrow.stream` an Arrow IPC stream", content_type = "application/json"),
        (status = 400, description = "`bad_request`", content_type = "application/json"),
        (status = 404, description = "`not_covered`", content_type = "application/json"),
    ))]
async fn slice(State(mesh): State<Mesh>, uri: Uri, headers: HeaderMap, req: Request) -> Response {
    let inner = mesh.inner.clone();
    let (client, _) = match who(&inner, &headers, peer_of(&req)) {
        Ok(x) => x,
        Err(r) => return to_response(r),
    };
    if let Err(r) = admit(&inner, &client, 1) {
        return to_response(r);
    }
    let rest = rest_of(&uri, "/slice/");
    // A manifest at /slice/<slice id>/<members hash> (SPEC 15.13).
    if let Some((a, b)) = rest.split_once('/') {
        if lexical::check_hex(a, 64, "$").is_ok() && lexical::check_hex(b, 64, "$").is_ok() {
            return manifest_of(inner, format!("slice:{a}:{b}")).await;
        }
    }
    let arrow = accepts(&headers, crate::slice::ARROW);
    let r = inner
        .pool
        .clone()
        .run(move || {
            let now = inner.now();
            inner.slice(&rest, &now, arrow)
        })
        .await
        .unwrap_or_else(|| envelope::error_reply(ErrorCode::EvalUnavailable));
    to_response(r)
}

fn accepts(headers: &HeaderMap, media: &str) -> bool {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.split(',').any(|p| p.split(';').next().unwrap_or("").trim() == media))
}

/// Whether the client takes JSON (no `Accept`, `*/*`, `application/*` or
/// `application/json`); for slices also Arrow.
fn acceptable(headers: &HeaderMap, also: Option<&str>) -> bool {
    let vals: Vec<&str> = headers.get_all(header::ACCEPT).iter().filter_map(|v| v.to_str().ok()).collect();
    if vals.is_empty() {
        return true;
    }
    vals.iter().flat_map(|v| v.split(',')).any(|p| {
        let mut parts = p.split(';');
        let m = parts.next().unwrap_or("").trim();
        let q0 = parts.any(|x| {
            let x = x.trim();
            x == "q=0" || x == "q=0.0" || x == "q=0.00" || x == "q=0.000"
        });
        !q0 && (m == "*/*" || m == "application/*" || m == JSON || also == Some(m))
    })
}

async fn object(
    inner: Arc<Inner>,
    what: &'static str,
    f: impl FnOnce(&Inner) -> Option<Vec<u8>> + Send + 'static,
) -> Response {
    let i2 = inner.clone();
    match inner.pool.run(move || f(&i2)).await.flatten() {
        Some(bytes) => {
            let ct = if what == "blob" { "application/octet-stream" } else { JSON };
            to_response(Reply {
                status: 200,
                body: bytes.into(),
                content_type: ct,
                cache: Cache::Immutable,
                canonical: None,
                retry_after: None,
            })
        }
        None => not_found(what),
    }
}

fn ip_admit(inner: &Inner, headers: &HeaderMap, req: &Request) -> Result<(), Response> {
    let (client, _) = who(inner, headers, peer_of(req)).map_err(to_response)?;
    admit(inner, &client, 1).map_err(to_response)
}

/// A receipt by its hash (canonical bytes; gzip when asked).
#[utoipa::path(get, path = "/receipt/{hash}", params(("hash" = String, Path, description = "The receipt hash (SPEC 8.3).")),
    responses((status = 200, description = "The receipt's canonical JSON; immutable", content_type = "application/json"), (status = 404, description = "Not held")))]
async fn receipt(State(mesh): State<Mesh>, Path(hash): Path<String>, headers: HeaderMap, req: Request) -> Response {
    if let Err(r) = ip_admit(&mesh.inner, &headers, &req) {
        return r;
    }
    if lexical::check_hex(&hash, 64, "$").is_err() {
        return not_found("receipt");
    }
    object(mesh.inner.clone(), "receipt", move |i| i.store.receipt(&hash).map(|c| c.into_owned())).await
}

/// A descriptor by its function hash.
#[utoipa::path(get, path = "/descriptor/{hash}", params(("hash" = String, Path, description = "The function hash.")),
    responses((status = 200, description = "The descriptor's canonical JSON; immutable", content_type = "application/json"), (status = 404, description = "Not held")))]
async fn descriptor(State(mesh): State<Mesh>, Path(hash): Path<String>, headers: HeaderMap, req: Request) -> Response {
    if let Err(r) = ip_admit(&mesh.inner, &headers, &req) {
        return r;
    }
    if lexical::check_hex(&hash, 64, "$").is_err() {
        return not_found("descriptor");
    }
    object(mesh.inner.clone(), "descriptor", move |i| i.store.descriptor(&hash).map(|c| c.into_owned())).await
}

async fn manifest_of(inner: Arc<Inner>, entry: String) -> Response {
    object(inner, "manifest", move |i| i.store.manifest(&entry).map(|c| c.into_owned())).await
}

/// A slice manifest (SPEC 15.13), by its entry's slice id and members hash.
#[utoipa::path(get, path = "/manifest/{slice_id}/{members_hash}",
    params(("slice_id" = String, Path, description = "The slice id."), ("members_hash" = String, Path, description = "The members hash.")),
    responses((status = 200, description = "The manifest's canonical JSON; immutable", content_type = "application/json"), (status = 404, description = "Not held")))]
async fn manifest(
    State(mesh): State<Mesh>,
    Path((slice_id, members)): Path<(String, String)>,
    headers: HeaderMap,
    req: Request,
) -> Response {
    if let Err(r) = ip_admit(&mesh.inner, &headers, &req) {
        return r;
    }
    if lexical::check_hex(&slice_id, 64, "$").is_err() || lexical::check_hex(&members, 64, "$").is_err() {
        return not_found("manifest");
    }
    manifest_of(mesh.inner.clone(), format!("slice:{slice_id}:{members}")).await
}

/// Blob content by SHA-256.
#[utoipa::path(get, path = "/blob/{sha256}", params(("sha256" = String, Path, description = "The content's SHA-256.")),
    responses((status = 200, description = "The bytes; immutable", content_type = "application/octet-stream"), (status = 404, description = "Not held")))]
async fn blob(State(mesh): State<Mesh>, Path(sha): Path<String>, headers: HeaderMap, req: Request) -> Response {
    if let Err(r) = ip_admit(&mesh.inner, &headers, &req) {
        return r;
    }
    if lexical::check_hex(&sha, 64, "$").is_err() {
        return not_found("blob");
    }
    object(mesh.inner.clone(), "blob", move |i| i.store.blob(&sha).ok().flatten().map(|c| c.into_owned())).await
}

/// A cell's lineage as nested envelopes.
#[utoipa::path(get, path = "/explain/{cell}",
    params(("cell" = String, Path, description = "As for /cell; `depth=` (default 4, at most 16) bounds the walk.")),
    responses(
        (status = 200, description = "`{\"envelope\", \"inputs\": [{\"entry\", \"envelope\", \"inputs\"} | {\"entry\", \"slice\", \"members\"}], \"truncated\"}`", content_type = "application/json"),
        (status = 202, description = "`pending`: the cell is being computed; ask again", content_type = "application/json"),
        (status = 400, description = "`bad_request`", content_type = "application/json"),
        (status = 404, description = "`not_covered`", content_type = "application/json"),
    ))]
async fn explain(State(mesh): State<Mesh>, uri: Uri, headers: HeaderMap, req: Request) -> Response {
    let inner = mesh.inner.clone();
    let (client, principal) = match who(&inner, &headers, peer_of(&req)) {
        Ok(x) => x,
        Err(r) => return to_response(r),
    };
    if let Err(r) = admit(&inner, &client, 1) {
        return to_response(r);
    }
    let mut rest = rest_of(&uri, "/explain/");
    let mut depth = inner.config.explain_depth;
    // `depth=` is the explain's own parameter: taken off the query.
    if let Some((p, q)) = rest.clone().split_once('?') {
        let kept: Vec<&str> = q.split('&').filter(|x| !x.starts_with("depth=")).collect();
        if let Some(d) = q.split('&').find_map(|x| x.strip_prefix("depth=")) {
            depth = d.parse::<usize>().unwrap_or(depth);
        }
        rest = if kept.is_empty() { p.to_string() } else { format!("{p}?{}", kept.join("&")) };
    }
    let depth = depth.min(16);
    let r = within_budget(inner, move |i, on| {
        let now = i.now();
        let r = i.answer(&rest, principal.as_ref(), i, &now, on);
        if r.status != 200 {
            return r;
        }
        let env = json::parse(&r.body).unwrap_or(Value::Null);
        let mut budget = 2_000usize;
        let v = i.explain_node(env, depth, &mut budget);
        Reply::json(200, v.to_jcs(), Cache::NoStore)
    })
    .await;
    to_response(r)
}

impl Inner {
    /// An envelope with its inputs' envelopes, `depth` levels down, at most `budget` nodes.
    fn explain_node(&self, envelope: Value, depth: usize, budget: &mut usize) -> Value {
        let inputs: Vec<String> = envelope
            .get("receipt")
            .and_then(|r| r.get("inputs"))
            .and_then(|i| {
                if let Value::Array(a) = i {
                    Some(a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let mut truncated = false;
        let mut out = Vec::new();
        for e in inputs {
            if depth == 0 || *budget == 0 {
                truncated = true;
                break;
            }
            *budget -= 1;
            if e.starts_with("slice:") {
                let m = self.store.manifest(&e).and_then(|m| datumesh_core::Manifest::from_json(&m).ok());
                let mut node = vec![("entry".to_string(), Value::String(e.clone()))];
                if let Some(m) = m {
                    node.push(("members".to_string(), envelope::num(m.members().len() as u64)));
                    node.push(("slice".to_string(), m.slice().to_value()));
                }
                out.push(Value::Object(node));
                continue;
            }
            let child = match self.store.receipt(&e) {
                Some(bytes) => {
                    let trust = Receipt::from_json(&bytes).map(|r| envelope::trust_of(&r, &self.config.host)).ok();
                    match trust {
                        Some(t) => self.explain_node(
                            json::parse(&envelope::ok(&bytes, t)).unwrap_or(Value::Null),
                            depth - 1,
                            budget,
                        ),
                        None => Value::Null,
                    }
                }
                None => json::parse(&envelope::not_covered(None)).unwrap_or(Value::Null),
            };
            let mut node = vec![("entry".to_string(), Value::String(e.clone()))];
            if let Value::Object(m) = child {
                node.extend(m);
            }
            out.push(Value::Object(node));
        }
        let mut node = vec![("envelope".to_string(), envelope.clone()), ("inputs".to_string(), Value::Array(out))];
        node.push(("truncated".to_string(), Value::Bool(truncated)));
        Value::Object(node)
    }
}

#[derive(serde::Deserialize)]
struct FindQuery {
    q: Option<String>,
    limit: Option<usize>,
}

/// Functions by keyword (a stub until 2.2).
#[utoipa::path(get, path = "/find",
    params(("q" = String, Query, description = "Keywords, all of which must appear (case-insensitive) in the function's name or catalog description."), ("limit" = Option<usize>, Query, description = "At most this many results (default 20, at most 100).")),
    responses((status = 200, description = "`{\"query\", \"results\": [{\"name\", \"fn_hash\", \"since\", \"description\", \"descriptor\"}]}`", content_type = "application/json")))]
async fn find(State(mesh): State<Mesh>, Query(q): Query<FindQuery>, headers: HeaderMap, req: Request) -> Response {
    if let Err(r) = ip_admit(&mesh.inner, &headers, &req) {
        return r;
    }
    let inner = mesh.inner.clone();
    let query = q.q.unwrap_or_default();
    let limit = q.limit.unwrap_or(20).clamp(1, 100);
    let r = inner
        .pool
        .clone()
        .run(move || {
            let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
            let now = inner.now();
            let mut results = Vec::new();
            for (name, desc) in &inner.config.catalog {
                let hay = format!("{} {}", name.to_lowercase(), desc.to_lowercase());
                if !words.iter().all(|w| hay.contains(w.as_str())) {
                    continue;
                }
                let Ok(Some(b)) = inner.store.resolve_binding(name, &now) else { continue };
                let d = inner.store.descriptor(&b.fn_hash).and_then(|d| json::parse(&d).ok()).unwrap_or(Value::Null);
                results.push(Value::Object(vec![
                    ("description".into(), Value::String(desc.clone())),
                    ("descriptor".into(), d),
                    ("fn_hash".into(), Value::String(b.fn_hash)),
                    ("name".into(), Value::String(name.clone())),
                    ("since".into(), Value::String(b.tx)),
                ]));
                if results.len() >= limit {
                    break;
                }
            }
            Value::Object(vec![("query".into(), Value::String(query)), ("results".into(), Value::Array(results))])
        })
        .await
        .unwrap_or(Value::Null);
    plain_json(200, r, Cache::Public(60))
}

#[derive(serde::Deserialize)]
struct VerifyQuery {
    level: Option<String>,
    blob: Option<String>,
    form: Option<String>,
}

/// Verification as a service: the core verifier on a submitted receipt, with this mesh's
/// keys; free, unauthenticated, rate-limited per IP.
#[utoipa::path(post, path = "/verify",
    params(("level" = Option<String>, Query, description = "`recomputed` (default: recompute with the engine as the resolver) or `signed` (steps 1-6)."), ("blob" = Option<String>, Query, description = "Blob content for step 7.7, as hex (at most 1 MiB)."), ("form" = Option<String>, Query, description = "`offered`: the body is `{\"receipt\": <its JSON text>, \"recompute\"?: <SPEC 11's recompute object>, \"blob_hex\"?}` and step 7 uses what is offered instead of this mesh's engine.")),
    request_body(content = String, description = "The receipt's JSON text, exactly as received (or the offered form).", content_type = "application/json"),
    responses(
        (status = 200, description = "`{\"ok\", \"level\", \"code\", \"hash\", \"computed_by\", \"path\", \"detail\", \"blob_checked\"}`", content_type = "application/json"),
        (status = 429, description = "`rate_limited`", content_type = "application/json"),
    ))]
async fn verify_receipt(
    State(mesh): State<Mesh>,
    Query(q): Query<VerifyQuery>,
    headers: HeaderMap,
    req: Request,
) -> Response {
    let inner = mesh.inner.clone();
    let ip = client_ip(&inner, &headers, peer_of(&req));
    if let Err(s) = inner.limiter.admit_verify(ip) {
        return to_response(envelope::rate_limited_reply(s));
    }
    let body: Bytes = match axum::body::to_bytes(req.into_body(), 4 << 20).await {
        Ok(b) => b,
        Err(_) => return batch_error("body too large"),
    };
    if q.form.as_deref() == Some("offered") {
        let Ok(v) = json::parse(&body) else { return batch_error("form=offered: not strict JSON") };
        let Some(receipt) = v.get("receipt").and_then(Value::as_str).map(|s| s.as_bytes().to_vec()) else {
            return batch_error("form=offered: no receipt text");
        };
        let blob = match v.get("blob_hex").and_then(Value::as_str).map(hex) {
            None => None,
            Some(Some(b)) if b.len() <= 1 << 20 => Some(b),
            Some(_) => return batch_error("blob_hex: hex of at most 1 MiB"),
        };
        let m = mesh.clone();
        let rc = v.get("recompute").cloned();
        let r = inner.pool.run(move || m.verify_offered(&receipt, rc.as_ref(), blob.as_deref())).await;
        return match r {
            Some(v) => to_response(Reply::json(200, v.body, Cache::NoStore)),
            None => to_response(envelope::error_reply(ErrorCode::EvalUnavailable)),
        };
    }
    let signed = q.level.as_deref() == Some("signed");
    let blob = match q.blob.as_deref().map(hex) {
        None => None,
        Some(Some(b)) if b.len() <= 1 << 20 => Some(b),
        Some(_) => return batch_error("blob: hex of at most 1 MiB"),
    };
    let m = mesh.clone();
    let r = inner.pool.run(move || m.verify(&body, signed, blob.as_deref())).await;
    match r {
        Some(v) => to_response(Reply::json(200, v.body, Cache::NoStore)),
        None => to_response(envelope::error_reply(ErrorCode::EvalUnavailable)),
    }
}

fn hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// A job: `pending` again while it runs, then the envelope of the original request.
#[utoipa::path(get, path = "/jobs/{job}", params(("job" = String, Path, description = "The job id of a `pending` envelope.")),
    responses(
        (status = 202, description = "`pending`, the same job and request", content_type = "application/json"),
        (status = 400, description = "`bad_request` with `E_JOB_UNKNOWN`: unknown or expired", content_type = "application/json"),
        (status = 200, description = "The completed envelope (any outcome but pending, with its status), bound to the original request", content_type = "application/json"),
    ))]
async fn job(State(mesh): State<Mesh>, Path(id): Path<String>, headers: HeaderMap, req: Request) -> Response {
    let inner = mesh.inner.clone();
    let (client, _) = match who(&inner, &headers, peer_of(&req)) {
        Ok(x) => x,
        Err(r) => return to_response(r),
    };
    if let Err(r) = admit(&inner, &client, 1) {
        return to_response(r);
    }
    let unknown = || to_response(Reply::json(400, envelope::bad_request(ErrorCode::JobUnknown), Cache::NoStore));
    let valid = (32..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !valid {
        return unknown();
    }
    match inner.jobs.poll(&id) {
        Poll::Unknown => unknown(),
        Poll::Done(r) => to_response(r),
        Poll::Running(request, elapsed) => {
            let poll = format!("{}/jobs/{id}", inner.config.origin());
            match Pending::new(&id, inner.jobs.estimate(elapsed), &poll, request) {
                Ok(p) => to_response(Reply::json(202, envelope::pending(&p), Cache::NoStore)),
                Err(_) => to_response(envelope::error_reply(ErrorCode::EvalUnavailable)),
            }
        }
    }
}

/// The key document (SPEC 9.2): the store's namespace record for this mesh's host.
#[utoipa::path(get, path = "/.well-known/datumesh.json", responses((status = 200, description = "The key document", content_type = "application/json"), (status = 404, description = "None stored")))]
async fn key_document(State(mesh): State<Mesh>) -> Response {
    let m = mesh.clone();
    match mesh.inner.pool.run(move || m.key_document()).await.flatten() {
        Some(doc) => to_response(Reply::json(200, doc, Cache::Public(300))),
        None => not_found("key document"),
    }
}

/// The mesh document (SPEC 9.6).
#[utoipa::path(get, path = "/.well-known/datumesh-mesh.json", responses((status = 200, description = "The mesh document", content_type = "application/json")))]
async fn mesh_document(State(mesh): State<Mesh>) -> Response {
    to_response(Reply::json(200, mesh.mesh_document(), Cache::Public(300)))
}

/// Which function each name denotes as of an instant (SPEC 9.7).
#[utoipa::path(get, path = "/bindings",
    params(("name" = String, Query, description = "A function name, percent-encoded as in SPEC 5.1; repeat for several."), ("asof" = Option<String>, Query, description = "An instant (default: now).")),
    responses((status = 200, description = "`{\"asof\", \"bindings\": {name: {\"fn_hash\", \"since\"}}}`", content_type = "application/json"), (status = 400, description = "A malformed name or asof", content_type = "application/json")))]
async fn bindings(State(mesh): State<Mesh>, uri: Uri, headers: HeaderMap, req: Request) -> Response {
    if let Err(r) = ip_admit(&mesh.inner, &headers, &req) {
        return r;
    }
    let inner = mesh.inner.clone();
    let mut names = Vec::new();
    let mut asof = None;
    for p in uri.query().unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (k, v) = p.split_once('=').unwrap_or((p, ""));
        let Some(v) = pct_decode(v) else { return batch_error("percent-encoding (SPEC 5.1)") };
        match k {
            "name" if lexical::check_function_name(&v, "$").is_ok() => names.push(v),
            "asof" if lexical::check_instant(&v, "$").is_ok() && asof.is_none() => asof = Some(v),
            _ => return batch_error(&format!("parameter {k}")),
        }
    }
    let r = inner
        .pool
        .clone()
        .run(move || {
            let at = asof.unwrap_or_else(|| inner.now());
            let mut out = Vec::new();
            for n in names {
                if !inner.config.owns(crate::foreign::host_of(&n)) {
                    continue;
                }
                match inner.store.resolve_binding(&n, &at) {
                    Ok(Some(b)) => out.push((
                        n,
                        Value::Object(vec![
                            ("fn_hash".into(), Value::String(b.fn_hash)),
                            ("since".into(), Value::String(b.tx)),
                        ]),
                    )),
                    Ok(None) => {}
                    Err(_) => return None,
                }
            }
            Some(Value::Object(vec![("asof".into(), Value::String(at)), ("bindings".into(), Value::Object(out))]))
        })
        .await
        .flatten();
    match r {
        Some(v) => plain_json(200, v, Cache::Public(inner_latest(&mesh))),
        None => to_response(envelope::error_reply(ErrorCode::InputUnavailable)),
    }
}

fn inner_latest(mesh: &Mesh) -> u64 {
    mesh.inner.config.ttl.latest_max_age.as_secs()
}

/// Strict percent-decoding (SPEC 5.1): literal characters as themselves, `%` and two
/// uppercase hex digits for other bytes, UTF-8.
fn pct_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'%' {
            let h = b.get(i + 1..i + 3)?;
            if !h.iter().all(|x| x.is_ascii_digit() || (b'A'..=b'F').contains(x)) {
                return None;
            }
            let v = u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()?;
            if v.is_ascii_alphanumeric() || matches!(v, b'-' | b'.' | b'_' | b'~' | b':') {
                return None;
            }
            out.push(v);
            i += 3;
        } else if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~' | b':') {
            out.push(c);
            i += 1;
        } else {
            return None;
        }
    }
    String::from_utf8(out).ok()
}

/// Liveness and the store's health.
#[utoipa::path(get, path = "/health", responses((status = 200, description = "`{\"status\": \"ok\"}`", content_type = "application/json"), (status = 503, description = "The store does not answer", content_type = "application/json")))]
async fn health(State(mesh): State<Mesh>) -> Response {
    let m = mesh.clone();
    let ok = tokio::time::timeout(
        Duration::from_secs(5),
        mesh.inner.pool.run(move || m.inner.store.namespace(&m.inner.config.host).is_ok()),
    )
    .await
    .ok()
    .flatten()
    .unwrap_or(false);
    let v = Value::Object(vec![("status".into(), Value::String(if ok { "ok" } else { "store unavailable" }.into()))]);
    plain_json(if ok { 200 } else { 503 }, v, Cache::NoStore)
}

/// Prometheus metrics.
#[utoipa::path(get, path = "/metrics", responses((status = 200, description = "Prometheus text", content_type = "text/plain")))]
async fn metrics(State(mesh): State<Mesh>) -> Response {
    let mut r = Reply::json(200, metrics_text(&mesh.inner).into_bytes(), Cache::NoStore);
    r.content_type = "text/plain; version=0.0.4";
    to_response(r)
}

/// This document.
#[utoipa::path(get, path = "/openapi.json", responses((status = 200, description = "The OpenAPI document", content_type = "application/json")))]
async fn openapi_json() -> Response {
    let mut r = Reply::json(200, openapi().to_json().unwrap_or_default().into_bytes(), Cache::Public(300));
    r.content_type = JSON;
    to_response(r)
}

pub(crate) fn metrics_text(inner: &Inner) -> String {
    use std::fmt::Write;
    use std::sync::atomic::Ordering::Relaxed;
    let mut out = String::new();
    inner.metrics.write(&mut out);
    let p = &inner.pool;
    let g = |out: &mut String, name: &str, help: &str, kind: &str, v: String| {
        let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {v}");
    };
    g(&mut out, "datumesh_pool_size", "The blocking pool's size: the concurrency limit.", "gauge", p.size.to_string());
    g(&mut out, "datumesh_pool_in_use", "Pool threads busy now.", "gauge", p.in_use.load(Relaxed).to_string());
    g(&mut out, "datumesh_pool_peak", "The most pool threads busy at once.", "gauge", p.peak.load(Relaxed).to_string());
    g(&mut out, "datumesh_pool_runs_total", "Tasks run on the pool.", "counter", p.runs.load(Relaxed).to_string());
    g(
        &mut out,
        "datumesh_pool_waited_total",
        "Tasks that found the pool saturated and waited for a thread.",
        "counter",
        p.waited.load(Relaxed).to_string(),
    );
    let (held, running) = inner.jobs.counts();
    g(&mut out, "datumesh_jobs_held", "Jobs held (running or done).", "gauge", held.to_string());
    g(&mut out, "datumesh_jobs_running", "Jobs running.", "gauge", running.to_string());
    g(
        &mut out,
        "datumesh_jobs_created_total",
        "Requests answered pending.",
        "counter",
        inner.jobs.created.load(Relaxed).to_string(),
    );
    g(
        &mut out,
        "datumesh_rate_limited_total",
        "Requests refused by a rate limit.",
        "counter",
        inner.limiter.refused.load(Relaxed).to_string(),
    );
    g(
        &mut out,
        "datumesh_peer_fetches_total",
        "Requests to other meshes.",
        "counter",
        inner.foreign.fetches.load(Relaxed).to_string(),
    );
    g(
        &mut out,
        "datumesh_peer_failures_total",
        "Requests to other meshes that failed.",
        "counter",
        inner.foreign.failures.load(Relaxed).to_string(),
    );
    g(
        &mut out,
        "datumesh_peer_imported_total",
        "Foreign versions imported into the store.",
        "counter",
        inner.foreign.imported.load(Relaxed).to_string(),
    );
    let m = inner.engine.memo_stats();
    g(&mut out, "datumesh_memo_entries", "Engine memo entries.", "gauge", m.entries.to_string());
    g(&mut out, "datumesh_memo_bytes", "Engine memo size, approximately.", "gauge", m.approx_bytes.to_string());
    g(
        &mut out,
        "datumesh_uptime_seconds",
        "Seconds since the mesh started.",
        "gauge",
        inner.started.elapsed().as_secs().to_string(),
    );
    out
}

/// Request ids, the access log, metrics and `Accept`.
async fn around(State(mesh): State<Mesh>, req: Request, next: Next) -> Response {
    let start = Instant::now();
    let route = req.extensions().get::<MatchedPath>().map(|m| m.as_str().to_string()).unwrap_or_else(|| "other".into());
    let route = match route.as_str() {
        "/cell/{*rest}" => "/cell/{cell}".to_string(),
        "/slice/{*rest}" => "/slice/{cell}".to_string(),
        "/explain/{*rest}" => "/explain/{cell}".to_string(),
        _ => route,
    };
    let id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .map(str::to_string)
        .unwrap_or_else(|| {
            let mut b = [0u8; 8];
            let _ = getrandom::fill(&mut b);
            lexical::to_hex(&b)
        });
    let ip = client_ip(&mesh.inner, req.headers(), peer_of(&req));
    let method = req.method().clone();
    let also = if route == "/slice/{cell}" { Some(crate::slice::ARROW) } else { None };
    let mut resp = if route != "/blob/{sha256}" && route != "/metrics" && !acceptable(req.headers(), also) {
        let mut r = Reply::json(
            406,
            Value::Object(vec![("error".into(), Value::String("not acceptable: application/json only".into()))])
                .to_jcs(),
            Cache::NoStore,
        );
        r.content_type = JSON;
        to_response(r)
    } else {
        next.run(req).instrument(tracing::info_span!("request", request_id = %id)).await
    };
    let elapsed = start.elapsed();
    let status = resp.status().as_u16();
    mesh.inner.metrics.record(&route, status, elapsed);
    let principal = resp.extensions().get::<Who>().and_then(|w| w.0.clone());
    tracing::info!(target: "datumesh_http::access", request_id = %id, %method, route = %route, status, ms = elapsed.as_secs_f64() * 1000.0, ip = %ip, principal = principal.as_deref().unwrap_or("-"), "request");
    if let Ok(v) = HeaderValue::from_str(&id) {
        resp.headers_mut().insert("x-request-id", v);
    }
    resp.headers_mut().insert(header::VARY, HeaderValue::from_static("Accept, Accept-Encoding, Authorization"));
    if let Ok(v) = HeaderValue::from_str(&format!("app;dur={:.3}", elapsed.as_secs_f64() * 1000.0)) {
        resp.headers_mut().insert("server-timing", v);
    }
    resp
}

async fn fallback() -> Response {
    not_found("endpoint")
}

impl Mesh {
    /// The router of every endpoint. Serve it with
    /// `into_make_service_with_connect_info::<SocketAddr>()` (the client IP comes from it).
    pub fn router(&self) -> Router {
        Router::new()
            .route("/cell/{*rest}", get(cell))
            .route("/cells", post(cells))
            .route("/slice/{*rest}", get(slice))
            .route("/receipt/{hash}", get(receipt))
            .route("/descriptor/{hash}", get(descriptor))
            .route("/manifest/{slice_id}/{members_hash}", get(manifest))
            .route("/blob/{sha256}", get(blob))
            .route("/explain/{*rest}", get(explain))
            .route("/find", get(find))
            .route("/verify", post(verify_receipt))
            .route("/jobs/{job}", get(job))
            .route("/.well-known/datumesh.json", get(key_document))
            .route("/.well-known/datumesh-mesh.json", get(mesh_document))
            .route("/bindings", get(bindings))
            .route("/health", get(health))
            .route("/metrics", get(metrics))
            .route("/openapi.json", get(openapi_json))
            .fallback(fallback)
            .layer(middleware::from_fn_with_state(self.clone(), around))
            .layer(tower_http::compression::CompressionLayer::new().gzip(true))
            .with_state(self.clone())
    }

    /// Serves the router on `listener` until `shutdown` resolves.
    pub async fn serve(
        &self,
        listener: tokio::net::TcpListener,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        axum::serve(listener, self.router().into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(shutdown)
            .await
    }
}

impl IntoResponse for Reply {
    fn into_response(self) -> Response {
        to_response(self)
    }
}
