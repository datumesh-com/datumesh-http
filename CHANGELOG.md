# Changelog

## 0.1.1 (unreleased)

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
