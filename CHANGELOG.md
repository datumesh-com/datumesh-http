# Changelog

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
