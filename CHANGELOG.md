# Changelog

## 0.3.3 (unreleased)

Loading is batched; no change to receipts. `publish-stored` makes its reads in chunks of up to 1,000 cell lines (one `latest_many` and one `receipts_many` per chunk, the store's binding, descriptor and namespace answers kept for the run) and writes consecutive cells with `put_cells` in calls of up to 1,000 (datumesh-store 0.4.3). The receipts, versions, counts and errors are those of 0.3.2: the golden files of `tests/perf_load.rs` were recorded with 0.3.2 and still match byte for byte. Signing runs on up to `available_parallelism()` threads. The earliest failing line still wins, with its line number and text.

`publish-stored` writes the cells of its second pass on parallel lanes (`DATUMESH_PUBLISH_LANES`, default `min(4, available_parallelism)`; a cell's lane is its id's first hex digit mod the lane count, and a failing call stops the other lanes from starting new ones).

A latest slice first page (no `asof`) reuses the cached members of its slice when no member changed since the cached instant (`slice_changed_after`), so a repeated first page is a cache hit; its `next` links pin that instant. The page contents are unchanged.

## 0.3.2 (2026-11-18)

For CONTRACT-1.7a-text (datumesh-http part): describe gives each method its description. Additive: no
signature changes, no change to the hash, the other members, or the cache header.

- **Method descriptions (H1).** `GET /describe/{class}` gives each method of the class a `description`
  taken from the catalog entry `{class}/{method}` when that entry is 1 to 1,024 bytes (UTF-8 bytes,
  never truncated). An entry that is absent, empty or longer than that gives no `description` member.
  The class itself still has none (16.8). `/find` is unchanged.

## 0.3.1 (2026-11-16)

For CONTRACT-1.7a (datumesh-http part): descriptor lines, no partial writes, `/find` returns classes
and methods, and cells on the run's clock. Additive: the public API gains members and no signature
changes; `publish::Published` gains the public field `descriptors`, which `Published`'s `#[non_exhaustive]`
allows. A file that was accepted in 0.3.0 and is refused now is one whose writes would have failed
part way (see below).

- **Descriptor lines (H1).** `publish-stored` takes `{"descriptor": <function descriptor>}`: the
  descriptor is put (`put_descriptor`, its hash is the core's `FunctionDescriptor::hash`) and nothing
  is bound. Only a stored function is accepted (`code.lang` `"stored"`, `code.body` `""`, `inputs`
  `[]`); any other is refused at its line, in the first pass. A class later in the file, or in a later
  run, may name the hash as a method. `Published::descriptors` counts the descriptors newly put by
  descriptor lines (a re-run counts 0). A line with both `class` and `descriptor` is a class line.
- **No partial writes (H2).** The first pass now plans every write the second makes, and refuses the
  file before any write when one would fail: a bind whose start is not later than the latest binding of
  its name (the store's, or one planned earlier in the file, so a function line and a class line binding
  one name at one instant are refused, W1b); a name bound as the other kind (a class as a function, or the
  reverse); a suspended namespace; a cell whose function is not bound as of its `tx` (counting the file's
  earlier binds); a blob file that is missing or unreadable; `tx <= asof <= issued`; and an existing
  version at or after the cell's `tx` that is not this receipt. Receipts are signed in the first pass
  (signing is deterministic), so their hashes are known there. The store is compared before and after
  in the tests (`tests/publish_atomic.rs`).
- **Found by the production rehearsal of step 1.7a.** A class whose steward namespace has a path
  (`datumesh.com/scripture`) is accepted: the first pass asks about the steward namespace's host,
  which is what a suspension row is. A cell line without `tx` (it takes the run's clock) whose
  cell's latest version states the same thing (type, value, valid, licence, summary) is skipped, so
  a re-run at a later clock changes nothing; a changed value is still a new version.
  `datumesh-mesh publish-stored` prints the `descriptors` count.
- **Cells on the run's clock (H4).** A cell line may leave out `tx` and `asof`; each defaults to the run's
  clock now, independently, the same instant as the default binding start. `issued` defaults as before
  (the later of `asof` and the key's start). One file can bind and publish with one clock reading.
- **`/find` returns classes and methods (H3).** The catalog may list class names and method names
  (`N/m`). Each result has a `kind`: `function` (as before, plus `kind`), `class` (`class_hash`, the
  flattened `descriptor`, `methods` with each method's `fn`, `fn_hash`, `args` (the arguments after the
  keys) and `returns`, and `description` where the catalog has one, and `since`), or `method` (`class`,
  `fn_hash`, `descriptor`, `since`). A class is matched by its own name and description and by its
  methods' names and catalog descriptions. Order and `limit` are as before; a catalog name that is
  unbound as of now is skipped. `/openapi.json` documents the members.
- **Version 0.3.1 (H5).** The README and the `lib` endpoint table describe the new members.

## 0.3.0 (2026-11-16)

For CONTRACT-1.8b (item 9): rule A, `--backfill` with its operator log, `import-signed` and bundle 1, the
describe index over computed cells, and Windows resolver errors. `cargo public-api` against 0.2.2 shows additions only (`public-api.txt`),
but `publish::PublishError` gains the public field `kind`, and a binding or declaration that starts
before the clock is now refused where it used to bind (see below): a minor release, not a patch.

- **Rule A (CONTRACT-1.8b H1).** No binding (a name's, a class's or a class method's) and no
  declaration (`valid_from`, `valid_to`) starts earlier than the mesh clock. A start left out is
  now. `publish::publish_stored_with(store, key, host, owned, path, &PublishOptions)`, where
  `PublishOptions { clock, backfill }`; `PublishOptions::system()`. The run reads the file in two
  passes: the first works out the binds and checks their starts, and nothing is written unless
  every check passes. `PublishError` gains `kind: PublishErrorKind` (`Line`, `StartBeforeNow`,
  `Backfill`, `OwnedVectorName`). `keydoc::declare_at` (typed `DeclareError`). `datumesh-mesh
  declare`: `--valid-from` is optional.
- **`--backfill` (H1).** `publish-stored <file> --backfill` with `DATUMESH_BACKFILL_LOG` (mode 0600,
  one JCS line per run that writes: `{"at","namespaces","starts"}`). Starts are not checked, but
  a backfill is refused when a name it would bind has a binding, or a namespace it would write
  under has any binding (store 0.4.2's `has_bindings_under`, CONTRACT-1.8b R2; the log records the
  run and is no longer read for the check).
- **`import-vectors` (H1).** `vectors::import_in(store, dir, now, host, owned)`: refused as a whole,
  before any write, when a name it would bind is under the mesh's own namespaces
  (`PublishErrorKind::OwnedVectorName`). `vectors::import` is unchanged.
- **Bundle 1 and `import-signed` (H2).** `datumesh_http::bundle`: `build` (signed with the
  32-byte seed: RFC 8032 Ed25519 with curve25519-dalek and sha2, now direct dependencies),
  `manifest_bytes`, `import_signed` (checks in contract order, `BundleError`, `Imported`). Writes
  run descriptors, then receipts and versions, then the bindings last, so a crash leaves nothing
  answerable and a re-run completes the import. `docs/bundle-1.md`. The `datumesh-mesh
  import-signed <file>` command prints the result as one JCS line.
- **The describe index covers computed cells (H4).** `GET /describe/{id}` names the objects of
  every recorded cell of each method of a class, stored (`slice_values`) and computed
  (`computed_cells`, store 0.4.2; at most 1,000,000 per method, one warning when reached). A formula
  cell that is computed is announced to the index, so a short id of its object resolves. Nothing is
  evaluated to build the index. Store 0.4.2 is required (`datumesh-store` 0.4.2, with `sqlite` in
  the dev-dependencies).
- **Windows resolver errors are `dns` (H3).** `client::transport_detail`: the Windows resolver codes
  11001 to 11004, and the text "No such host is known", are `dns` (they were `network`).

## 0.2.2 (unreleased; no release yet)

For spec-0.5.0 (`c8896ad`): classes, describe, `resolved`, `E_NAMESPACE_SUSPENDED`; with the
domain validation (step 1.5) and the `E_FOREIGN_UNAVAILABLE` fetch codes of this branch.
On `datumesh-core` 0.3.8, `datumesh-store` 0.4.1 (the namespace checks and classes),
`datumesh-engine` 0.4.2 and `datumesh-formula` 0.3.0 (all four unpublished: the branch is built
against local patches; `Cargo.toml` names the minima). **Additions only** (`cargo public-api`
against 0.2.1: 59 lines added, none removed or changed: `API-CHANGES-0.2.2.md`), so a patch
release; none of formula's types is in the public API.

- **Classes in the mesh (16).** `publish-stored` takes a class line, `{"class": "<ns>/<name>",
  "descriptor": {...}, "bound": "<instant>"}`: defined with formula's `define_class` (the
  owner rule: the class is under one of the mesh's own namespaces, else `E_CLASS_OWNER`),
  then `put_class` and `bind_class` from `bound` (the methods are bound with it), unless the
  name is already bound to that class. Function lines take `classes`, `default` and
  `key_normalisation` in their signatures; the order is method functions, classes, the
  formulas that dispatch. `publish::publish_stored_in(store, key, host, owned, path)`;
  `Published::classes`. `vectors::import` imports a vector world's classes (and skips
  `invalid/formula-eval-class-dispatch-rebound`, which no store can load).
- **`/bindings` (9.7)** answers a class name with `class_hash` in place of `fn_hash`; a method
  `N/m` answers as a function.
- **Requests by call form (5.8, 16.3, 16.7).** At `/cell` and `/cells` a call is a method call
  (`N(keys).m(args)`, the same cell and receipt as the plain read), takes unquoted values by
  the position's type, labels (`N[label]`, through the class's `lookup`), the `asof` default
  (the call's, or the mesh's now, written in the cell URI) and bare names through the alias
  map. `call::Bindings` gains the provided methods `class`, `lookup` and `aliases_tx`;
  `Target::resolved`. A key not canonical under its rule is `bad_request`
  `E_KEY_NORMALISATION`, a malformed encoding `E_KEY_ENCODING` (`envelope::CALL_CODES`;
  `BAD_REQUEST_CODES` stays the 5.4 set).
- **`resolved` (5.5)** is in the envelope of every outcome that may carry it (`ok`, `stale`,
  `not_covered`, `unlicensed`, `payment_required`, `error`) whenever something was resolved
  (`envelope::with_resolved`); a plain or pinned call of a full name and a cell URI have none.
  An `@alias` call or a pinned alias now carries it. `aliases_tx` is absent on the mesh (its
  alias map has no snapshot).
- **describe (16.8, 9.6).** `GET /describe/{id}`, listed in the mesh document: `{id}` is a class
  name (`describe-class`: the flattened descriptor and the method table, `args` and `returns`
  from the method functions; no `description`, a function descriptor has none), an object's key
  encoding (`describe-object`, the keys checked against the class), an object id, a cell id (the
  object whose method cell it is) or a short id; never a class hash (404). Known objects are
  those of a class bound now that have a recorded cell of one of its stored methods (the
  store's slice reads with every argument open), kept in an in-memory index built on the first
  describe or short-id request and again when a stored version is announced
  (`Mesh::stored_version_added`), when a class is heard of, or after 30 s. A short id matching
  more than one object or cell is `bad_request` `E_SHORT_ID_AMBIGUOUS` (400). `Mesh::describe`.
  **Limits:** the store cannot list its classes or report its latest transaction time, so the
  index covers the classes the mesh has heard of (a stored version announced under them, a
  request or describe that resolved them, the classes of the configured aliases) and is
  rebuilt on a timer for versions another process adds; a class published by another process
  and never named is not covered until then. `same_as` is not emitted. `/cell/{id}` by cell id
  is not served.
- **`E_NAMESPACE_SUSPENDED` across meshes (9.9, 5.5).** A cross-mesh import the store refuses
  because the namespace is suspended is `not_covered` with that `error` (404, never an `error`
  envelope); `is_error_code` refuses it, the class codes and `CALL_CODES`.
- **Faults.** `E_FORMULA_KEY` and `E_FORMULA_DISPATCH_KEY` are `not_covered` with `fault`
  (engine 0.4.2; a guard test). `super` reads go through the engine's pinned reads
  (`foreign::Through` forwards `Inputs::read_pinned`).
- **`shadows` (16.9)** is never emitted: the test mesh has no reference classes.

Domain validation (step 1.5), as before:

- **`domain` module.** `Validator::validate(store, host, now)` fetches
  `https://<host>/.well-known/datumesh.json` and checks it with core 0.3.7's rules
  (`key_document_from_response`, declarations included). Valid: `put_namespace` (so
  `validated_at` is recorded and a rotation replaces the stored key document through the store's
  normal path), then `record_namespace_success` (the failure count back to zero, a suspension
  lifted). Invalid: `record_namespace_failure` with `SUSPEND_AFTER` = 2, so the second
  consecutive failure suspends the namespace. `validate_all` does every namespace in the store;
  the mesh's own namespaces are skipped (never suspended). `Outcome`, `Validation`.
- **Suspension** refuses new writes under the namespace with the store's
  `Error::NamespaceSuspended`: `put_receipt` of a receipt signed by the host, `put_version` of
  such a receipt, `bind_name` under the host (so `define`, `publish-stored` and the import of a
  foreign host's bindings and cells too). Reads, verification, the key document the mesh serves,
  and receipts already stored are not affected. A cross-mesh import the store refuses is
  `not_covered` with `E_NAMESPACE_SUSPENDED` (spec-0.5.0).
- **`datumesh-mesh validate <host> | --all`**: one JSON line per host (`host`, `outcome`
  `valid`/`invalid`/`skipped`, `failures`, `suspended`, `suspended_now`, `reinstated`,
  `rotated`, and for an invalid host `code` and `detail`); exit 1 when any host is invalid.
  `DATUMESH_PEERS` maps a host to another origin, as for cross-mesh resolution.
- **Every cross-mesh fetch that gets no HTTP response is `E_FOREIGN_UNAVAILABLE`** (502, with
  core 0.3.7's `ForeignDetail`: `network`, `dns`, `tls`, `timeout`, `refused`), not only the
  publisher's key-document refresh: key and mesh documents, bindings, descriptors, receipts,
  manifests, blobs, cells, slices, batches and versions fetched from another mesh
  (`Client`, so `foreign.rs`), and a name that cannot be resolved because its host cannot be
  reached. A mesh that answers keeps its own outcome (`E_FN_UNAVAILABLE`, a bad status, ...).
  A cell served before is still `stale`. The system resolver's "failed to lookup address
  information" is `dns`. Changed: `Client::key_document` of an unreachable host was
  `E_KEY_UNAVAILABLE`; `Client` methods that were `E_INPUT_UNAVAILABLE` for it are
  `E_FOREIGN_UNAVAILABLE` (the error's `foreign_detail()` says why).
- **A suspended namespace across meshes** is `E_NAMESPACE_SUSPENDED` (see above; it was
  `E_INPUT_UNAVAILABLE` behind `foreign::NAMESPACE_SUSPENDED` until core 0.3.8).
- **Weekly re-check**: `deploy/validate.sh`, a cron entry (Sundays 04:30 UTC) installed by
  `deploy/install_cron.sh`.

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
