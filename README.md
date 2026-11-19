# datumesh-http

The [Datumesh](https://datumesh.com) engine behind HTTPS ([spec-0.5.0](https://github.com/datumesh-com/datumesh-spec): since 0.2.2, classes, method calls, `resolved`, describe and short ids, `E_NAMESPACE_SUSPENDED`; since 0.2.1, licence inheritance and `declarations` in the key document): cell envelopes (SPEC 5.5), batches, slices as JSON pages or Arrow streams, receipts, descriptors, manifests, blobs, lineage, search, verification, jobs for long evaluations, the key and mesh documents (SPEC 9.2, 9.6), bindings (SPEC 9.7), versions (SPEC 9.8), cross-mesh resolution of names under other hosts, and a store-less client for other meshes (`client`). The library is the router and the operations behind it; the binary `datumesh-mesh` serves it. It runs [`datumesh-engine`](https://crates.io/crates/datumesh-engine) 0.4.2 with the formula runtime of [`datumesh-formula`](https://crates.io/crates/datumesh-formula) 0.3.0 on [`datumesh-store`](https://crates.io/crates/datumesh-store) 0.4.2 (PostgreSQL: the store with `default-features = false`, so the binary and the image compile no SQLite; an application that enables the store's `sqlite` can run a `Mesh` on a `SqliteStore`) and [`datumesh-core`](https://crates.io/crates/datumesh-core) 0.3.8.

## Example

```toml
[dependencies]
datumesh-http = "0.3.1"
datumesh-store = "0.4.2"
datumesh-core = { version = "0.3.8", features = ["sign"] }
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
| `GET /cell/<call or uri>` | the envelope; a cell URI's rest byte for byte (`/cell/ecb.europa.eu/fx_rate?base=EUR&date=2024-03-15&quote=USD`, `&fn_hash=` then `&asof=` last), or a call (`/cell/rates.example.org/fx_rate(%22EUR%22,%202024-03-15,%20%22USD%22)?asof=...`, pinned `name@<fn_hash>(...)`); every answer echoes `fn_hash` (a `pending` one in its `request`); trust `computed` for receipts this mesh signed; a slice marker is `E_SLICE_AT_CELL`, an unbound name `not_covered` with `E_NAME_UNBOUND` |
| `POST /cells` | a JSON array of requests (calls, `cell://` URIs or `{"cell": ...}`), at most 1,000, to `[{"envelope", "status"}]` in request order (SPEC 5.7), or one envelope refusing the whole batch (`bad_request` for a body that is not such an array or too long) |
| `GET /slice/<call or uri with *>` | members of a stored function's slice as of the asof, in key order: page envelopes (SPEC 5.6: `slice`, `members` as `ok` envelopes, `next`; `limit=`, default 1,000, at most 10,000; `next` carries `after=` the last key) or, with `Accept: application/vnd.apache.arrow.stream`, one Arrow IPC stream (UTF-8 columns: the open arguments, `value`, `type`, `receipt`, `tx`, `asof`) |
| `GET /receipt/<hash>`, `/descriptor/<hash>`, `/manifest/<slice id>/<members hash>`, `/blob/<sha256>` | the bytes, immutable |
| `GET /explain/<call or uri>` | `{"envelope", "inputs": [{"entry", "envelope", "inputs", "truncated"} or {"entry", "members", "slice"}], "truncated"}`, `depth=` (default 4, at most 16), at most 2,000 nodes |
| `GET /find?q=` | `{"query", "vector", "results": [...], "resolution"?}`: the hybrid search of the catalogue (since 0.4.0, `limit` 10 by default, 50 at most). Each result has `kind` (`function`, `method` or `class`), `name`, `label`, `description`, `call`, `match` (`exact`, `text` or `vector`) and `score`, with the members of its kind (`descriptor`, `fn_hash`, `since`; a class has `class_hash` and `methods`; a method has `class`). `resolution` reads the query as a request: `exact` (the filled `call`), `ambiguous` or `range` (`E_RANGE_REFUSED`). `vector` is true when the embedding service (`DATUMESH_EMBED_URL`) answered. `GET /find?summary=1` is the catalogue summary. |
| `POST /verify` | the receipt's text as the body: `{"ok", "level", "code", "hash", "computed_by", "path", "detail", "blob_checked"}`, at level `recomputed` with the engine as the resolver (`?level=signed`: steps 1-6; `?blob=<hex>`: step 7.7); `?form=offered` takes `{"receipt": <text>, "recompute": <SPEC 11's object>, "blob_hex"}` and recomputes with what is offered. Free, unauthenticated, 60 a minute per IP |
| `GET /jobs/<id>` | `pending` (202) while the job runs, then the envelope of the original request; `bad_request` `E_JOB_UNKNOWN` (400) when unknown or expired |
| `GET /.well-known/datumesh.json` | the key document: the store's namespace record for the mesh's host |
| `GET /.well-known/datumesh-mesh.json` | the mesh document (SPEC 9.6) |
| `GET /bindings?name=<name>&asof=` | SPEC 9.7, for names under the hosts the mesh owns (a class name answers `class_hash`, since 0.2.2) |
| `GET /describe/<id>` | since 0.2.2, SPEC 16.8: `{id}` is a class name (`describe-class`), an object's key encoding, an object id, a cell id or a short id (`describe-object`); a class hash is none of them (404); a short id naming more than one object or cell is `bad_request` `E_SHORT_ID_AMBIGUOUS` (400); the objects the mesh knows are those of a bound class with a recorded cell of any of its methods, stored or computed (since 0.3.0, CONTRACT-1.8b H4; at most 1,000,000 cells per method, read from the store; nothing is evaluated) |
| `POST /whatif` | since 0.4.0, spec-0.6.0 5.9: the body `{"cell", "asof"?, "overrides", "solve"?}` (value overrides `{cell, value}`, slice overrides `{slice, add, remove}`, at most 64; `solve` with `vary`, `target`, `equals`, `lo`, `hi`): a scenario `{asof, cell, overrides, type, value[, solved]}` with no receipt, never stored and never a fact; `bad_request` codes as 5.9 (`E_CANON_TYPE`, `E_SLICE_MEMBER`, `E_CANON_ARG_DUPLICATE`, `E_CANON_LIMIT`, `E_SCHEMA`, ...); `not_covered` (404) with the fault, or `E_WHATIF_NO_SOLUTION` |
| `GET /versions/<uri>` | SPEC 9.8: a cell's versions `{"cell", "versions": [{"tx", "asof", "receipt"}], "next"}`, oldest first, 1,000 a page (`from=`, `to=` instants) |
| `GET /health`, `/metrics`, `/openapi.json` | the store's health; Prometheus text; the OpenAPI document (generated from the handlers, utoipa) |

Since 0.2.2 a call at `/cell` may be a method call (`N(keys).m(args)`), take unquoted values by position type, labels (`N[label]`), the `asof` default and bare names through the alias map, and the envelope then carries `resolved` (SPEC 5.5); `shadows` is never emitted (the test mesh has no reference classes). The crate documentation has the details: the call forms accepted (pins since 0.1.2), every outcome and its status, how `Engine::get` maps to envelopes, jobs, cross-mesh resolution and its cache lifetimes, caching headers, the pool and the rate limits.

## The client

`datumesh_http::client` reads other meshes without a store (for `datumesh-local` and other tools): `Client::new(ClientConfig)` (the hosts it may fetch from, `host=origin` peers, the timeout and the key and mesh documents' lifetimes); `resolve(host)` (the mesh document, checked with the key document, SPEC 9.6), `key_document`, `binding`, `cell(host, rest, &Request)` and `reference(host, &Reference)` (a call, a cell URI or a pin; the envelope checked with core's `check_answer`, signatures through the key document), `slice_page`/`slice` (pages checked with `check_slice_answer`), `batch` (`check_batch`), `receipt`, `descriptor`, `manifest` and `blob` (each checked against its hash), `versions`. It is blocking (`ureq`): call it from a blocking thread in async code. Errors are `ClientError { code, detail }`, the code the mesh would answer with. The mesh's cross-mesh import (`foreign`) is a thin layer on it that puts what the client fetched and checked into the store.

## Configuration

The server reads the environment; files named there are read at start (secrets only from files with mode 0600).

| Variable | Default | |
|---|---|---|
| `DATABASE_URL` | | PostgreSQL 16 (`?sslmode=require` for TLS; `DATABASE_CA_FILE` adds roots) |
| `DATUMESH_HOST` | | the DNS name the mesh signs as and serves on (`mesh-test.datumesh.com`) |
| `DATUMESH_BASE_URL` | none | where the mesh is served when not at `https://<host>`: every URL of the mesh document is on it (`http://127.0.0.1:8787` for a mesh on this machine); `https://` with any host; `http://` only for an unrooted `DATUMESH_HOST` (under `.invalid`, e.g. `local.invalid`) and only on `127.0.0.1`, `[::1]` or `localhost` (local use: such a document is not valid under spec-0.4.3, spec-0.4.4 will allow it); an optional port, no path, query, fragment or trailing slash. The key document, `host` and the URLs in envelopes (`poll`, `next`, `canonical`) stay on `https://<host>` |
| `DATUMESH_KEY_FILE` | | the signing key's seed (32 bytes or 64 hex digits), mode 0600; its key must be in the host's key document in the store (`datumesh-mesh keygen --publish`) |
| `DATUMESH_LISTEN` | `127.0.0.1:8080` | where to listen (plain HTTP: Caddy terminates TLS in front) |
| `DATUMESH_NAMESPACES` | the host | the hosts whose names this mesh owns (comma-separated); the owned list of `publish-stored`, `import-signed` and `import-vectors` (0.3.0) |
| `DATUMESH_BACKFILL_LOG` | none | since 0.3.0: the operator log of `publish-stored --backfill` (created with mode 0600; one JCS line appended per run that writes) |
| `DATUMESH_PUBLISH_LANES` | `min(4, available_parallelism)`, at least 1 | since 0.3.3: `publish_stored_lanes` (and the `publish-stored` CLI, which calls it) writes the cells of its second pass on this many lanes (a positive integer; a cell's lane is its id's first hex digit mod the count, so a cell's versions keep their file order); `1` writes as 0.3.2. `publish_stored`, `publish_stored_in` and `publish_stored_with` always write on one lane, in file order, and ignore this variable |
| `DATUMESH_API_KEYS_FILE` | none | the API keys: `<sha256 of the key> <principal> [per minute]` lines, mode 0600 |
| `DATUMESH_POLICY_FILE` | none | `{"<function>": {"principals": [...]} or {"price": {"amount", "currency"}}}` |
| `DATUMESH_ALIASES_FILE`, `DATUMESH_CATALOG_FILE` | none | `{"alias": "<function>"}`; `{"<function>": "description"}` |
| `DATUMESH_BUDGET_MS` | 1000 | the budget before `pending` |
| `DATUMESH_POOL_SIZE` | 32 | the blocking pool, the concurrency limit, and the store's connections |
| `DATUMESH_BATCH_MAX`, `DATUMESH_SLICE_PAGE`, `DATUMESH_SLICE_MAX`, `DATUMESH_EXPLAIN_DEPTH`, `DATUMESH_MAX_JOBS` | 1000, 1000, 1048576, 4, 10000 | bounds |
| `DATUMESH_TEST_CELLS` | off | since 0.4.0, mesh-test only: each held function test is a computed cell `<host>/test_case(function_hash, case) -> boolean`, signed by the mesh (production names none) |
| `DATUMESH_RATE_LIMITS`, `DATUMESH_RATE_PER_IP`, `DATUMESH_RATE_IP_BURST`, `DATUMESH_RATE_PER_KEY`, `DATUMESH_RATE_VERIFY_PER_IP` | on, 60, 120, 600, 60 | rate limits, per minute |
| `DATUMESH_TRUST_PROXY` | off | the client IP from the last `X-Forwarded-For` entry (behind Caddy) |
| `DATUMESH_CROSS_MESH`, `DATUMESH_CROSS_MESH_HOSTS`, `DATUMESH_PEER_TIMEOUT_MS`, `DATUMESH_PEERS` | on, every host, 10000, none | cross-mesh resolution; the hosts it may fetch from (comma-separated: others are `not_covered`); its time limit; `host=origin` pairs overriding `https://<host>` (tests, private networks) |
| `DATUMESH_TTL_KEY_DOC`, `DATUMESH_TTL_MESH_DOC`, `DATUMESH_TTL_BINDING`, `DATUMESH_TTL_CURRENT`, `DATUMESH_LATEST_MAX_AGE`, `DATUMESH_JOB_TTL` | 300, 300, 60, 60, 10, 300 | seconds |
| `DATUMESH_LICENCE`, `DATUMESH_FUEL` | `CC-BY-4.0`, 10,000,000 | the engine's (the mesh's own licence: since 0.2.1, that of a computed receipt with no inputs, the others inheriting theirs (SPEC 8.5); fuel per request) |
| `DATUMESH_MIGRATE` | off | apply the store's schema at start |
| `RUST_LOG`, `DATUMESH_LOG_FORMAT` | `info`, `json` | logs (`text` for plain lines) |

The command's other uses: `datumesh-mesh migrate`; `datumesh-mesh publish-stored <file.jsonl> [--backfill]` (stored functions, formula functions, since 0.2.2 classes, and their cells, blobs included, signed with `DATUMESH_KEY_FILE` as `DATUMESH_HOST`; idempotent; the format is in the `publish` module's documentation; since 0.3.1 a `{"descriptor": ...}` line puts a stored function's descriptor and binds nothing, a cell line may leave out `tx` and `asof` (the run's clock now), and a file is refused as a whole, with nothing written, when any of its writes would fail; since 0.3.0 no binding may start before now (rule A, refused with the file and line, nothing written), and `--backfill` (with `DATUMESH_BACKFILL_LOG`) lifts that for a file whose names and namespaces have no bindings yet (a namespace with any binding refuses it, store 0.4.2's `has_bindings_under`; the log records each run that writes)); `datumesh-mesh keygen --out <file> [--publish] [--valid-from <instant>]` (a new key, mode 0600; `--publish` adds it to the host's key document, rotating the current key out at `--valid-from`); `datumesh-mesh declare --fn <name> --licence <licence> [--valid-from <instant>] [--valid-to <instant>]` (since 0.2.1; since 0.3.0 `--valid-from` defaults to now, and a start or end earlier than now is refused, rule A: a declaration, SPEC 9.2, in the key document of `DATUMESH_HOST` in the store, which the mesh serves: `<name>`, a function under that host, may be computed over inputs under the restricted `<licence>`; idempotent; `--valid-to` closes an open entry, entries are never removed; a mesh that serves declarations is unverifiable by clients on core < 0.3.7, datumesh < 0.4.3 or @datumesh/verify < 0.4.3); `datumesh-mesh validate <host> | --all` (domain validation: fetches `https://<host>/.well-known/datumesh.json`, checks it as SPEC 9.2 says and records the outcome in the store; a valid document replaces the stored one, the second consecutive failure suspends the namespace so that new writes under it are refused with the store's `NamespaceSuspended`, the next success reinstates it; one JSON line per host, exit 1 when any is invalid; the mesh's own namespaces are skipped; weekly by cron, `deploy/validate.sh`); `datumesh-mesh apikey --principal <id> --keys-file <file> --out <file> [--limit <n>]` (the key is written to `--out`, mode 0600, and never printed); `datumesh-mesh import-signed <bundle.json>` (since 0.3.0: a bundle 1, `docs/bundle-1.md`: a namespace's descriptors, names and signed receipts, checked and written when every check passes, printed as one JCS line); `datumesh-mesh import-vectors <dir>` (the spec vectors' cells into the store, `deploy/README.md`; since 0.3.0 refused as a whole when a name it would bind is under the mesh's own namespaces); `datumesh-mesh openapi`; `datumesh-mesh health`. The test deployment (compose with Caddy, backups, the restore drill, the resource check, the smoke suite) is in `deploy/`.

## Docker

`Dockerfile` builds the image (`rust:1.99` to `debian:trixie-slim`, user 10001); `docker-compose.yml` runs it with PostgreSQL 16, listening on 127.0.0.1:8080, the key in `./secrets` (owned by uid 10001, mode 0600). Releases push `ghcr.io/datumesh-com/datumesh-mesh:<version>`.

## Development

With a PostgreSQL 16 server whose role may create databases in `DATABASE_URL`, `cargo test` runs everything, each test on a database of its own; the brief's timings are asserted in optimised builds only: `cargo test --release` (the 100,000-member slice test loads its slice first, about a minute). `tools/cargo.sh <toolchain> <args>` runs cargo in a `rust:<toolchain>` container next to a `datumesh-pg` container. `tools/client_check.sh` checks the request-binding cases the tests write (`target-out/binding-cases.jsonl`) with the Python (`datumesh` 0.4.2) and JavaScript (`@datumesh/verify` 0.4.2) libraries (envelopes, pins, `computed`, a page and a batch); `tools/cache_test.sh` runs nginx (proxy cache) in front of a test mesh and asserts by the origin's counter that repeat gets never reach it; `tools/package_test.sh` is the packaging test (the example above in a new project); `tools/public_api.sh` regenerates `public-api.txt`.

Test numbers on the development host (6 vCPU, 15 GB RAM; release build; `target-out/` has each run's): a warm `GET /cell` of a stored cell 0.6-0.7 ms p50 over loopback; 1,000 cells in one `POST /cells` 5-6 ms (about 8 times one cell); 1,000 concurrent warm gets p50 75 ms, p99 110 ms, no errors, the pool saturated (32 in use); a 100,000-member slice as Arrow (16 MB) 0.1 s warm (6 s cold, from the store); `pending` at 1.0 s for a 10 s evaluation, its job done at 10.05 s, then 2 ms from the memo; a rate-limited client's refusals at 0.6 ms p50 while a client under its limit kept its p99.

Licensed under Apache-2.0.
