//! The [Datumesh](https://datumesh.com) engine behind HTTPS: the endpoints of a mesh
//! (spec-0.4.1) over [`datumesh_engine::Engine`], and the `datumesh-mesh` server.
//!
//! A [`Mesh`] holds the engine, its store and what the endpoints share; [`Mesh::router`] is
//! the axum router of every endpoint and [`Mesh::serve`] serves it. The blocking operations
//! the endpoints are built on are library calls too ([`Mesh::cell`], [`Mesh::cells`],
//! [`Mesh::resolve`], [`Mesh::verify`], [`call::resolve`], the cross-mesh resolver
//! [`foreign::Foreign`]).
//!
//! # Endpoints
//!
//! | Endpoint | Answer |
//! |---|---|
//! | `GET /cell/<call or uri>` | the envelope (SPEC 5.5); `?asof=` for a call |
//! | `POST /cells` | a JSON array of requests (calls or `cell://` URIs) to an array of envelopes, at most 1,000 |
//! | `GET /slice/<call or uri with *>` | the members of a stored function's slice: JSON pages by key, or an Arrow IPC stream |
//! | `GET /receipt/<hash>`, `/descriptor/<hash>`, `/manifest/<slice id>/<members hash>`, `/blob/<sha256>` | the bytes, immutable (also `/slice/<slice id>/<members hash>`, SPEC 15.13) |
//! | `GET /explain/<call or uri>` | the lineage as nested envelopes, depth-bounded (`depth=`, default 4, at most 16) |
//! | `GET /find?q=` | functions of the catalog by keyword (a stub until 2.2) |
//! | `POST /verify` | the core verifier on a submitted receipt with this mesh's keys |
//! | `GET /jobs/<id>` | a job: `pending`, or the envelope of the original request |
//! | `GET /.well-known/datumesh.json`, `/.well-known/datumesh-mesh.json` | the key document (the store's namespace record for the host, SPEC 9.2) and the mesh document (SPEC 9.6) |
//! | `GET /bindings?name=&asof=` | which function a name denotes (SPEC 9.7), for names this mesh owns |
//! | `GET /health`, `/metrics`, `/openapi.json` | liveness and the store; Prometheus text; the OpenAPI document generated from the handlers |
//!
//! `Accept` is honoured: JSON (and, at `/slice`, `application/vnd.apache.arrow.stream`); a
//! request whose `Accept` excludes JSON is `406`, never an HTML page. Every response has a
//! request id (`X-Request-Id`, taken from the request when it is a short token) and
//! `Server-Timing`.
//!
//! # Requests and envelopes
//!
//! [`call`] says exactly which forms `/cell` accepts: a cell URI's rest byte for byte, or a
//! read of SPEC 15.3 with literal arguments (positional by `signature.order`, or labelled),
//! the callee a full name or an `@alias` of the configured map. A call is parsed by the
//! formula crate's parser and written as its cell URI, so both are one request and give
//! byte-identical envelopes; short names never appear in a response. Method chaining and
//! key literals (SPEC 15.15, 1.3b) are `E_PARSE_URI` in 0.1.0.
//!
//! Every envelope is canonical JSON (JCS). Outcomes:
//!
//! | Outcome | When |
//! |---|---|
//! | `ok` | the engine's receipt; `trust` `primary` (signer's host is the namespace's, no inputs), `verified` (computed by this mesh), else `unverified` |
//! | `stale` | the store or another mesh failed, and the mesh served a receipt for the cell before (`age_seconds` since) that answers the request |
//! | `not_covered` | the name is not bound, the cell has no value; with `fault` when its evaluation faulted |
//! | `unlicensed` | the configured policy names principals and the requester is not one; or the `Authorization` header is not a key of this mesh |
//! | `payment_required` | the policy prices the function (a stub until 3.3) |
//! | `rate_limited` | a rate limit refused the request (`retry_after_seconds`, `Retry-After`) |
//! | `bad_request` | the request does not parse (every code of SPEC 5.4); `E_JOB_UNKNOWN` at a job URL |
//! | `error` | the mesh failed: 503 for `E_INPUT_UNAVAILABLE`, `E_KEY_UNAVAILABLE`, `E_MESH_DOC`, `E_EVAL_UNAVAILABLE`; 500 otherwise |
//! | `pending` | the answer took longer than the budget (1 s): poll `/jobs/<id>` |
//!
//! ## `Engine::get` as this crate uses it
//!
//! The mesh resolves the name itself (as of the request's asof, or now: the bindings it
//! looked up are the cell's function hash) and calls `Engine::get_cell(cell, Some(A))`, A the
//! request's asof or the clock reading at the request. The answer maps as the engine's table
//! says: `Answer::Receipt` is `ok`; `Answer::NotCovered` `not_covered` (with `fault`);
//! `EngineError::Invalid` with a code of SPEC 5.4 `bad_request`, with another code (a
//! cross-mesh failure) `error` with that code; `NameUnresolved` `error` `E_FN_UNAVAILABLE`;
//! `UndeclaredInput` `E_INPUT_UNDECLARED`; `Sign` its code (`E_TIME_ORDER`); `Store`
//! `E_INPUT_UNAVAILABLE` (or `stale`); every other failure (`NoRuntime`, `Runtime`, `Cycle`,
//! `DepthExceeded`, `BudgetExceeded`, `WorkExceeded`) `E_EVAL_UNAVAILABLE`.
//!
//! # Pending and jobs
//!
//! A request runs on the pool; if it has not finished within the budget, the mesh answers
//! `pending` (202) with a job id of 256 bits from the operating system's CSPRNG,
//! `estimate_seconds`, `poll` (`https://<host>/jobs/<id>`) and the original request, and the
//! work goes on. `GET /jobs/<id>` answers `pending` again while it runs, then the envelope the
//! `/cell` request would have had, with its status, bound to the original request. Results are
//! kept 300 s after they are first served (and 300 s after completion if never served); an
//! unknown or expired id is `bad_request` `E_JOB_UNKNOWN`. Jobs are held in memory: a
//! restart forgets them, and the client's remedy for `E_JOB_UNKNOWN` (send the original
//! request again) is then answered from the store, where the evaluation's receipt was kept.
//! A batch's requests past the budget become jobs one by one.
//!
//! # Cross-mesh resolution
//!
//! See [`foreign`]: names under a host this mesh does not own are imported into the store
//! (bindings, descriptors, the cells' receipts, verified against the signer's key document),
//! through a layer around every runtime, so the engine reads foreign cells as it reads its
//! own. Lifetimes: key documents 300 s (a key id not in the cached document refetches it at
//! once), mesh documents 300 s, a foreign name's bindings and a foreign cell's latest
//! version confirmed again after 60 s; descriptors and receipts forever (content-addressed);
//! answers as of an instant before the last confirmation never change. The other mesh down:
//! `error` (`E_INPUT_UNAVAILABLE`), or `stale` for a cell answered before; never
//! `not_covered`.
//!
//! # Caching
//!
//! `ok` as of a past instant, and receipts, descriptors, manifests and blobs:
//! `Cache-Control: public, max-age=31536000, immutable`. Answers about now: `public,
//! max-age=10` and `Link: <https://<host>/cell/<uri with asof>>; rel="canonical"`, the as-of
//! form of the same answer. Answers that depend on who asks (a policy applies): `private,
//! no-store`. Errors, refusals and jobs: `no-store`. Responses are gzipped when the client
//! asks (`Accept-Encoding: gzip`).
//!
//! # Concurrency and rate limits
//!
//! axum on tokio: the engine and the store are synchronous, so every request's work runs on
//! a `spawn_blocking` thread holding one of [`Config::pool_size`] permits (default 32). The
//! pool's size is the concurrency limit: it is the store's connection pool size too (the
//! server connects with that many connections), large enough that store latency (about
//! 0.2 ms a query) does not idle the 6 cores of the target VM and small enough that
//! PostgreSQL's default 100 connections are never approached; it is reported in `/metrics`
//! (`datumesh_pool_size`, `_in_use`, `_peak`, `_waited_total`). Rate limits are token buckets
//! checked before any pool work, per IP (60 a minute, burst 120), per API key (600 a minute)
//! and for `POST /verify` per IP (60 a minute); a refused request is answered at once, never
//! queued.
//!
//! # Secrets and logs
//!
//! The signing key and the API keys file are read only from files with mode 0600
//! ([`config::read_secret_file`]); API keys are held as SHA-256 hashes. Logs (`tracing`,
//! JSON by default in the server) carry a request id, the route template, the status, the
//! latency, the client IP and the principal id; never a key, a job id or a path.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod apikeys;
pub mod call;
pub mod config;
pub mod envelope;
pub mod foreign;
mod http;
mod jobs;
pub mod keydoc;
mod limit;
mod mesh;
mod metrics;
mod pool;
pub mod slice;

pub use config::Config;
pub use http::openapi;
pub use mesh::{BuildError, Mesh, MeshBuilder, Verified};
