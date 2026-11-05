# Changelog

## 0.2.1 (unreleased)

For spec-0.4.3: licence inheritance and `declarations` in the key document. On
`datumesh-engine` 0.4.1 and `datumesh-core` 0.3.7 (were 0.4.0 and 0.3.6). Additive.

**A mesh that serves `declarations` is unverifiable by clients on core < 0.3.7 or datumesh <
0.4.3; publishers should wait for their consumers to upgrade before declaring.** The same holds
for @datumesh/verify < 0.4.3.

- **Licence inheritance (SPEC 8.5).** The engine (0.4.1) signs a computed receipt under the
  licence its input receipts give; where the rule refuses (`EngineError::Unlicensed`), the mesh
  answers `unlicensed` (403, no new code, the function hash echoed; `public, max-age` of
  `DATUMESH_LATEST_MAX_AGE`, never immutable: a declaration may come later). A cell that reads
  a refused cell is refused too. `DATUMESH_LICENCE` is now the licence of a computed receipt
  with no inputs. Computed receipts signed before 0.2.1 are signed again under the inherited
  licence (or refused) on their next request; nothing needs migrating.
- **Declarations (SPEC 9.2).** `/.well-known/datumesh.json` is the store's key document, as
  before, so the declarations it holds are served; without any, the bytes are 0.2.0's.
  `keydoc::with_declaration(host, existing, fn_name, licence, valid_from, valid_to)` adds an
  entry, leaves an identical one unchanged, or closes an open one (`keydoc::Declared`); entries
  are never removed. The binary's `datumesh-mesh declare --fn <name> --licence <licence>
  --valid-from <instant> [--valid-to <instant>]` applies it to `DATUMESH_HOST`'s document in the
  store. `keydoc::with_key` (key rotation, `keygen --publish`) keeps the declarations.
- **Another mesh's function, computed here.** Before evaluating a function under a host it does
  not own, the mesh puts that publisher's key document in the store (the key document cache's
  TTL, `DATUMESH_TTL_KEY_DOC`), where the engine reads its declarations
  (`foreign::Foreign::ensure_publisher`, also in the cross-mesh runtime layer). A publisher that
  gives no HTTP response then is `error` `E_FOREIGN_UNAVAILABLE` (spec-0.4.3: 502, with
  `detail` `dns`, `tls`, `timeout`, `refused` or `network`), or `stale` with a receipt served
  before. `client::Client::publisher_key_document` and `ClientError::foreign_detail` (the
  detail is the start of `ClientError::detail`). The other cross-mesh failures keep their
  0.2.0 codes.
- **Deployment (`deploy/`).** The seed binds `ecb/fx_cross_undeclared` (defined like
  `fx_cross`, not declared); `seed/declarations.py` lists the test mesh's two declarations
  (`ecb/fx_cross` over the restricted test licence, `scripture/strip_niqqud` over the UXLC
  licence); `licences.sh` applies both once after the image upgrade (idempotent); `smoke.sh`
  checks fx_cross over NOK (restricted licence, entitled), fx_cross_undeclared over NOK
  (`unlicensed`) and strip_niqqud (UXLC licence, entitled). The smoke, seed and vector checks
  use datumesh 0.4.3 and @datumesh/verify 0.4.3. The spec submodule is at `spec-0.4.3`.
- Public API: additions only (`public-api.txt`).

## 0.2.0 (unreleased)

On `datumesh-store` 0.4.0, `datumesh-engine` 0.4.0 and `datumesh-formula` 0.2.0 (were 0.3.2,
0.3.2 and 0.1.1), and a configurable base URL for the mesh document.

- **The store 0.4 move.** Breaking only because the public API names the store's, engine's
  and formula's types (`Mesh::builder` takes an `Arc<dyn Store + Send + Sync>`, `Mesh::engine`
  gives the engine, `BuildError::Engine` carries its error), which are now store 0.4's, engine
  0.4's and formula 0.2's: users move to the four together. No code change was needed for it.
  Store 0.4.0 adds a SQLite backend (`SqliteStore`), which a `Mesh` can run on (the store's
  `Store` is the same trait) when the application enables it (`datumesh-store`'s default
  feature `sqlite`, or `datumesh-engine/sqlite`). **The image's backend is unchanged:
  PostgreSQL** (`DATABASE_URL` names a PostgreSQL 16 database, as before): this crate depends on
  the store with `default-features = false`, and on engine and formula without their `sqlite`
  features, so the mesh binary and the image compile no SQLite (`cargo tree -e features` has no
  `rusqlite` or `libsqlite3-sys`; `Cargo.lock` holds neither).
- **A base URL for the mesh document** (`Config::base_url`, `Config::with_base_url`,
  `DATUMESH_BASE_URL`): when set, every URL of `/.well-known/datumesh-mesh.json` (`bindings`
  and the endpoints) is on it instead of `https://<host>`, so a mesh on this machine (`datumesh
  serve`) can advertise `http://127.0.0.1:<port>`. `https://` with any host. `http://` only
  when the mesh's host is unrooted (under `.invalid`, SPEC 2.5, e.g. `local.invalid`), and
  then only on `127.0.0.1`, `[::1]` or `localhost`; a rooted host is never served over
  `http://`. An optional port; no path, query, fragment or trailing slash. Another is refused
  when the mesh is built, `BuildError::BaseUrl` with a `config::BaseUrlError` (`Scheme`,
  `HttpRootedHost`, `HttpNotLoopback`, `Host`, `Port`, `Path`, `TrailingSlash`, `Query`,
  `Fragment`), or by `Config::from_env` and `Config::check` as a `ConfigError`;
  `config::check_base_url(host, url)` checks one alone, `config::is_unrooted(host)` tells an
  unrooted host. The key document and `host` do not change, nor do the URLs in envelopes and
  headers (`poll`, a page's `next`, the `canonical` link), which stay on `https://<host>`
  (`Config::origin`): core's `Pending` takes an `https://` `poll` only. Unset (the default),
  the mesh document is 0.1.2's byte for byte.
- **An `http://` mesh document is local-only.** It is not valid under spec-0.4.3 (9.6: every
  URL `https://` on the host), and core 0.3.6's `resolve_mesh` (so `client::Client::resolve`)
  refuses it with `E_MESH_DOC`. spec-0.4.4 will allow exactly this: for an unrooted host,
  mesh-document URLs and `poll` may be `http://` on a loopback address only (`127.0.0.1`,
  `[::1]`, `localhost`); rooted hosts stay `https://`-only. datumesh-core 0.3.8 will accept it.
- Additive otherwise: `Config::base_url`, `Config::with_base_url`, `Config::mesh_document_base`,
  `config::BaseUrlError`, `config::check_base_url`, `config::is_unrooted`,
  `BuildError::BaseUrl`.

## 0.1.2

The first published release of this work. The tag `v0.1.1` marks a release run that stopped before publishing (the batch was over 10 times one cell on CI; fixed below), so 0.1.1 was never published.

spec-0.4.2 (datumesh-core 0.3.6, datumesh-engine 0.3.2, datumesh-formula 0.1.1; store 0.3.2).

- Pins: `/cell/name@<fn_hash>(args)` and `&fn_hash=` in a cell URI; every answer echoes
  `fn_hash`, `pending` in its `request`. An unknown pin is `not_covered`.
- Trust `computed` for receipts this mesh signed (not `primary`); envelopes built with core's
  `Answer` builder.
- `GET /slice` JSON answers are page envelopes (SPEC 5.6); Arrow is unchanged.
- `POST /cells` answers `[{"envelope", "status"}]`, or one envelope refusing the whole batch
  (SPEC 5.7).
- `POST /cells` is as fast as in 0.1.0 again (1,000 cells under 10 times one cell, the
  brief's criterion 8): each chunk writes and counts its own answers on the pool, a batch
  looks each binding up once (not once per chunk), and the core count is read once.
- `E_SLICE_AT_CELL` (`bad_request`) for a slice marker at `/cell`; `E_NAME_UNBOUND`
  (`not_covered` with `error`) for an unbound name.
- One manifest path, `/manifest/<slice id>/<members hash>` (`/slice/<hex>/<hex>` is gone).
- `GET /versions/<uri>` (SPEC 9.8), listed in the mesh document; the job lifetime text is
  "while the mesh runs".
- `client`: a store-less, blocking client for other meshes, every answer checked with core;
  `foreign` (cross-mesh import) is now a thin layer on it, and uses `versions` when a mesh
  lists it. `foreign::ForeignError` is now a type alias of `client::ClientError` (the same
  fields and traits).
- `publish`: `publish_stored` and `datumesh-mesh publish-stored <file.jsonl>`: stored and
  formula functions and their cells (blobs included) signed as the mesh, idempotently.
- `vectors`: `import-vectors` loads every formula and excel vector's function (by hash), so
  each answers by pinned hash; names only invalid vectors bind are bound too.
- Additive: `call::Target::pin`, `pinned_reference`, `call::format_pinned`, the provided
  `Bindings::pinned` and `Bindings::alias`.
- The test mesh (`deploy/`): the seed data (`deploy/seed.sh`: the synthetic ECB-shaped cells
  and the scripture corpora), the smoke suite and the drill for 0.1.1.

## 0.1.0 (unreleased)

The first release: the Datumesh engine (datumesh-engine 0.3.1, the formula runtime of
datumesh-formula 0.1.0, datumesh-store 0.3.2, datumesh-core 0.3.5) behind HTTPS, spec-0.4.1.

- Endpoints: `GET /cell/<call or uri>`, `POST /cells`, `GET /slice/...` (JSON pages by key,
  Arrow IPC), `GET /receipt|descriptor|manifest|blob/...`, `GET /explain/...`, `GET /find`
  (a stub), `POST /verify`, `GET /jobs/<id>`, the key and mesh documents, `GET /bindings`,
  `GET /health`, `/metrics`, `/openapi.json`.
- Envelopes of every outcome of SPEC 5.5, `pending` included (HTTP 202, jobs kept 300 s),
  canonical JSON; a call and its cell URI give the same bytes.
- Cross-mesh resolution, minimal form: foreign names imported into the store (bindings,
  descriptors, receipts verified against their signer's key document).
- Rate limits that refuse, never queue; caching headers; API keys hashed in a 0600 file.
- `datumesh-mesh`: serve, migrate, keygen (rotation), apikey, openapi, health; the Docker
  image and `docker-compose.yml`.
