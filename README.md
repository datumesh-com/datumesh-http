# datumesh-http

The [Datumesh](https://datumesh.com) engine behind HTTPS ([spec-0.4.1](https://github.com/datumesh-com/datumesh-spec)): cell envelopes (SPEC 5.5), batches, slices as JSON pages or Arrow streams, receipts, descriptors, manifests, blobs, lineage, search, verification, jobs for long evaluations, the key and mesh documents (SPEC 9.2, 9.6), bindings (SPEC 9.7), and cross-mesh resolution of names under other hosts. The library is the router and the operations behind it; the binary `datumesh-mesh` serves it. It runs [`datumesh-engine`](https://crates.io/crates/datumesh-engine) 0.3.1 with the formula runtime of [`datumesh-formula`](https://crates.io/crates/datumesh-formula) 0.1.0 on [`datumesh-store`](https://crates.io/crates/datumesh-store) 0.3.2 and [`datumesh-core`](https://crates.io/crates/datumesh-core) 0.3.5.

## Example

```toml
[dependencies]
datumesh-http = "0.1.0"
datumesh-store = "0.3.2"
datumesh-core = { version = "0.3.5", features = ["sign"] }
```

A mesh for `mesh.example.com` on a fresh database: its key document in the store, a stored function and one value, then the envelope of the cell asked as a call and as a cell URI (the same bytes):

```rust
use std::sync::Arc;

use datumesh_core::json::{self, Value};
use datumesh_core::{Arg, Args, Cell, ReceiptBuilder, Scalar, SigningKey, Type};
use datumesh_http::{Config, Mesh};
use datumesh_store::{migrate, PgStore, Store};

const T: &str = "2026-03-01T09:00:00.000000Z";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(PgStore::connect(&std::env::var("DATABASE_URL")?)?);
    migrate::apply(&*store)?;
    let host = "mesh.example.com";
    let key = SigningKey::from_seed(&[7; 32]);
    if store.namespace(host)?.is_none() {
        let doc = datumesh_http::keydoc::with_key(host, None, &key, "2026-01-01T00:00:00.000000Z")?;
        store.put_namespace(host, &doc, "2026-01-01T00:00:00.000000Z")?;
    }

    // A stored function of the mesh's own namespace, and the value of one of its cells.
    let d = r#"{"code":{"body":"","lang":"stored"},"inputs":[],"signature":{"args":{"item":{"type":"string","valid":false}},"order":["item"],"returns":{"type":"number","unit":"EUR"}}}"#;
    let fn_hash = store.put_descriptor(d.as_bytes())?;
    if store.resolve_name("mesh.example.com/price", T)?.is_none() {
        store.bind_name("mesh.example.com/price", &fn_hash, "2026-01-01T00:00:00.000000Z")?;
    }
    let args: Args = [("item".to_string(), Arg::Value(Scalar::from_lexical(Type::String, "bolt")?))].into();
    let cell = Cell { ns: host.into(), fn_name: "price".into(), fn_hash, args };
    if store.latest(&cell.id())?.is_none() {
        let r = ReceiptBuilder::new()
            .cell(cell.clone())
            .value(Scalar::from_lexical(Type::Number, "12.5")?)
            .valid(Args::new())
            .asof(T)
            .tx(T)
            .issued(T)
            .publisher(host)
            .signer_host(host)
            .licence("CC0-1.0")
            .summary("The price of a bolt.")
            .sign(&key)?;
        let mut members = vec![("sig".to_string(), Value::String(r.sig().to_string()))];
        members.extend(json::parse(r.payload())?.as_object().into_iter().flatten().cloned());
        let hash = store.put_receipt(&Value::Object(members).to_jcs())?;
        store.put_version(&cell.id(), T, &hash)?;
    }

    // The mesh: the router serves these same answers over HTTP (Mesh::serve).
    let shared: Arc<dyn Store + Send + Sync> = store.clone();
    let mesh = Mesh::builder(Config::new(host), shared).signing_key(key).build()?;
    let by_uri = mesh.cell("mesh.example.com/price?item=bolt", None);
    let by_call = mesh.cell("mesh.example.com/price(%22bolt%22)", None);
    assert_eq!(by_uri.body, by_call.body);
    println!("{} {}", by_uri.status, String::from_utf8_lossy(&by_uri.body));
    Ok(())
}
```

It prints `200` and the `ok` envelope with the publisher's receipt (`"trust":"primary"`). Run again, it finds everything in place and prints the same.

## Endpoints

| Endpoint | Answer |
|---|---|
| `GET /cell/<call or uri>` | the envelope; a cell URI's rest byte for byte (`/cell/ecb.europa.eu/fx_rate?base=EUR&date=2024-03-15&quote=USD`, `&asof=` last), or a call (`/cell/rates.example.org/fx_rate(%22EUR%22,%202024-03-15,%20%22USD%22)?asof=...`) |
| `POST /cells` | a JSON array of requests (calls, `cell://` URIs or `{"cell": ...}`), at most 1,000, to an array of envelopes in order (each has its outcome's status; `error`: 500) |
| `GET /slice/<call or uri with *>` | members of a stored function's slice as of the asof, in key order: JSON pages (`limit=`, default 1,000, at most 10,000; `next` carries `after=` the last key) or, with `Accept: application/vnd.apache.arrow.stream`, one Arrow IPC stream (UTF-8 columns: the open arguments, `value`, `type`, `receipt`, `tx`, `asof`) |
| `GET /receipt/<hash>`, `/descriptor/<hash>`, `/manifest/<slice id>/<members hash>`, `/blob/<sha256>` | the bytes, immutable; a manifest also at `/slice/<slice id>/<members hash>` (SPEC 15.13) |
| `GET /explain/<call or uri>` | `{"envelope", "inputs": [{"entry", "envelope", "inputs", "truncated"} or {"entry", "members", "slice"}], "truncated"}`, `depth=` (default 4, at most 16), at most 2,000 nodes |
| `GET /find?q=` | `{"query", "results": [{"name", "fn_hash", "since", "description", "descriptor"}]}` over the catalog's names and descriptions (a stub: pgvector is 2.2) |
| `POST /verify` | the receipt's text as the body: `{"ok", "level", "code", "hash", "computed_by", "path", "detail", "blob_checked"}`, at level `recomputed` with the engine as the resolver (`?level=signed`: steps 1-6; `?blob=<hex>`: step 7.7); `?form=offered` takes `{"receipt": <text>, "recompute": <SPEC 11's object>, "blob_hex"}` and recomputes with what is offered. Free, unauthenticated, 60 a minute per IP |
| `GET /jobs/<id>` | `pending` (202) while the job runs, then the envelope of the original request; `bad_request` `E_JOB_UNKNOWN` (400) when unknown or expired |
| `GET /.well-known/datumesh.json` | the key document: the store's namespace record for the mesh's host |
| `GET /.well-known/datumesh-mesh.json` | the mesh document (SPEC 9.6) |
| `GET /bindings?name=<name>&asof=` | SPEC 9.7, for names under the hosts the mesh owns |
| `GET /health`, `/metrics`, `/openapi.json` | the store's health; Prometheus text; the OpenAPI document (generated from the handlers, utoipa) |

The crate documentation has the details: the call forms accepted in 0.1.0, every outcome and its status, how `Engine::get` maps to envelopes, jobs, cross-mesh resolution and its cache lifetimes, caching headers, the pool and the rate limits.

## Configuration

The server reads the environment; files named there are read at start (secrets only from files with mode 0600).

| Variable | Default | |
|---|---|---|
| `DATABASE_URL` | | PostgreSQL 16 (`?sslmode=require` for TLS; `DATABASE_CA_FILE` adds roots) |
| `DATUMESH_HOST` | | the DNS name the mesh signs as and serves on (`mesh-test.datumesh.com`) |
| `DATUMESH_KEY_FILE` | | the signing key's seed (32 bytes or 64 hex digits), mode 0600; its key must be in the host's key document in the store (`datumesh-mesh keygen --publish`) |
| `DATUMESH_LISTEN` | `127.0.0.1:8080` | where to listen (plain HTTP: Caddy terminates TLS in front) |
| `DATUMESH_NAMESPACES` | the host | the hosts whose names this mesh owns (comma-separated) |
| `DATUMESH_API_KEYS_FILE` | none | the API keys: `<sha256 of the key> <principal> [per minute]` lines, mode 0600 |
| `DATUMESH_POLICY_FILE` | none | `{"<function>": {"principals": [...]} or {"price": {"amount", "currency"}}}` |
| `DATUMESH_ALIASES_FILE`, `DATUMESH_CATALOG_FILE` | none | `{"alias": "<function>"}`; `{"<function>": "description"}` |
| `DATUMESH_BUDGET_MS` | 1000 | the budget before `pending` |
| `DATUMESH_POOL_SIZE` | 32 | the blocking pool, the concurrency limit, and the store's connections |
| `DATUMESH_BATCH_MAX`, `DATUMESH_SLICE_PAGE`, `DATUMESH_SLICE_MAX`, `DATUMESH_EXPLAIN_DEPTH`, `DATUMESH_MAX_JOBS` | 1000, 1000, 1048576, 4, 10000 | bounds |
| `DATUMESH_RATE_LIMITS`, `DATUMESH_RATE_PER_IP`, `DATUMESH_RATE_IP_BURST`, `DATUMESH_RATE_PER_KEY`, `DATUMESH_RATE_VERIFY_PER_IP` | on, 60, 120, 600, 60 | rate limits, per minute |
| `DATUMESH_TRUST_PROXY` | off | the client IP from the last `X-Forwarded-For` entry (behind Caddy) |
| `DATUMESH_CROSS_MESH`, `DATUMESH_CROSS_MESH_HOSTS`, `DATUMESH_PEER_TIMEOUT_MS`, `DATUMESH_PEERS` | on, every host, 10000, none | cross-mesh resolution; the hosts it may fetch from (comma-separated: others are `not_covered`); its time limit; `host=origin` pairs overriding `https://<host>` (tests, private networks) |
| `DATUMESH_TTL_KEY_DOC`, `DATUMESH_TTL_MESH_DOC`, `DATUMESH_TTL_BINDING`, `DATUMESH_TTL_CURRENT`, `DATUMESH_LATEST_MAX_AGE`, `DATUMESH_JOB_TTL` | 300, 300, 60, 60, 10, 300 | seconds |
| `DATUMESH_LICENCE`, `DATUMESH_FUEL` | `CC-BY-4.0`, 10,000,000 | the engine's (the licence of the receipts it signs; fuel per request) |
| `DATUMESH_MIGRATE` | off | apply the store's schema at start |
| `RUST_LOG`, `DATUMESH_LOG_FORMAT` | `info`, `json` | logs (`text` for plain lines) |

The command's other uses: `datumesh-mesh migrate`; `datumesh-mesh keygen --out <file> [--publish] [--valid-from <instant>]` (a new key, mode 0600; `--publish` adds it to the host's key document, rotating the current key out at `--valid-from`); `datumesh-mesh apikey --principal <id> --keys-file <file> --out <file> [--limit <n>]` (the key is written to `--out`, mode 0600, and never printed); `datumesh-mesh import-vectors <dir>` (the spec vectors' cells into the store: `deploy/README.md`); `datumesh-mesh openapi`; `datumesh-mesh health`. The test deployment (compose with Caddy, backups, the restore drill, the resource check, the smoke suite) is in `deploy/`.

## Docker

`Dockerfile` builds the image (`rust:1.99` to `debian:trixie-slim`, user 10001); `docker-compose.yml` runs it with PostgreSQL 16, listening on 127.0.0.1:8080, the key in `./secrets` (owned by uid 10001, mode 0600). Releases push `ghcr.io/datumesh-com/datumesh-mesh:<version>`.

## Development

With a PostgreSQL 16 server whose role may create databases in `DATABASE_URL`, `cargo test` runs everything, each test on a database of its own; the brief's timings are asserted in optimised builds only: `cargo test --release` (the 100,000-member slice test loads its slice first, about a minute). `tools/cargo.sh <toolchain> <args>` runs cargo in a `rust:<toolchain>` container next to a `datumesh-pg` container. `tools/client_check.sh` checks the request-binding cases the tests write (`target-out/binding-cases.jsonl`) with the Python (`datumesh` 0.4.1) and JavaScript (`@datumesh/verify` 0.4.1) libraries; `tools/cache_test.sh` runs nginx (proxy cache) in front of a test mesh and asserts by the origin's counter that repeat gets never reach it; `tools/package_test.sh` is the packaging test (the example above in a new project); `tools/public_api.sh` regenerates `public-api.txt`.

Test numbers on the development host (6 vCPU, 15 GB RAM; release build; `target-out/` has each run's): a warm `GET /cell` of a stored cell 0.6-0.7 ms p50 over loopback; 1,000 cells in one `POST /cells` 5-6 ms (about 8 times one cell); 1,000 concurrent warm gets p50 75 ms, p99 110 ms, no errors, the pool saturated (32 in use); a 100,000-member slice as Arrow (16 MB) 0.1 s warm (6 s cold, from the store); `pending` at 1.0 s for a 10 s evaluation, its job done at 10.05 s, then 2 ms from the memo; a rate-limited client's refusals at 0.6 ms p50 while a client under its limit kept its p99.

Licensed under Apache-2.0.
