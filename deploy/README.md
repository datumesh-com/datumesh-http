# The test mesh: https://mesh-test.datumesh.com

Compose project `datumesh-mesh-test` (`compose.yml`): PostgreSQL 16 (its own volume
`datumesh-mesh-test_pgdata`), the mesh (`datumesh-mesh` image), Caddy 2.10.2 (pinned by digest)
as the TLS front on 80 and 443 (automatic HTTPS, Let's Encrypt HTTP-01, no e-mail; HSTS; HTTP/1.1
and HTTP/2). Postgres and the mesh are on an internal network with no route out and publish no
port; only Caddy does. Restart policies, health checks and resource limits on all three.

State and secrets are outside the repository, in `/root/datumesh-mesh-test` (`DATUMESH_DIR`):

| Path | |
|---|---|
| `secrets/` (0700, uid 10001) | `mesh.key` (the signing key, 0600), `api-keys` (hashes, 0600), `user.api-key` and `tests.api-key` (the keys themselves, 0600, never printed), `db.env` (the Postgres password and `DATABASE_URL`, root, 0600) |
| `backups/` (0700) | nightly `pg_dump -Fc`, 14 kept |
| `status/` | `resources.json` and `resources.txt` (the 15-minute check), `import.json`, `drill.json`, `smoke.json`, `vectors.json` |
| `logs/` | `backup.log`, `resources.log` (cron) |
| `bin/` | the scripts cron runs (copies, `install_cron.sh`) |
| `catalog.json` | the `/find` catalog: the imported names |

## Setting up and running

```sh
docker build -t datumesh-mesh:0.1.2-local .      # until GHCR has 0.1.2: DATUMESH_MESH_IMAGE=datumesh-mesh:0.1.2-local
deploy/setup.sh                                   # directories, password, key (keygen --publish), API keys, import
deploy/seed.sh                                    # the seed data (below): download, generate, publish-stored, checks
CADDYFILE=Caddyfile.internal docker compose -p datumesh-mesh-test -f deploy/compose.yml up -d   # before DNS
deploy/go_live.sh                                 # once the A record resolves: Let's Encrypt
deploy/install_cron.sh                            # nightly backup 03:15 UTC, resource check every 15 minutes
deploy/smoke.sh [--local]                         # the HTTPS smoke suite (--local: before DNS, internal CA)
deploy/restore_drill.sh                           # restore the newest dump next to the live mesh and compare
```

`compose.yml` names the published image by digest; `DATUMESH_MESH_IMAGE` overrides it (0.1.2
runs as `datumesh-mesh:0.1.2-local` until its image is on GHCR, then by digest). The other
scripts use the image of the running mesh container unless `DATUMESH_MESH_IMAGE` says otherwise.

Configuration (in `compose.yml`): host `mesh-test.datumesh.com`; the vectors' hosts served as
the mesh's own namespaces; cross-mesh resolution fetching from no host
(`DATUMESH_CROSS_MESH_HOSTS` empty: the vectors' hosts are examples, nothing real to fetch);
pool 32; budget 1 s; rate limits 60/min per IP (burst 120), 600/min per key, 60/min for
`/verify`; client IPs from Caddy's `X-Forwarded-For`. Logs: the mesh's JSON access log has the
route template, status, latency, IP and principal; Caddy's has no query strings, no headers and
no job ids.

## The vectors' cells (`import.sh`, `datumesh-mesh import-vectors`)

Besides the seed data (below), the store holds what the spec-0.4.2 vectors define: the key
documents of their hosts (those of the receipt vectors and the formula vectors' test keys,
merged per host), every descriptor the vectors give (235: every formula-eval and excel
vector's function, valid or not, so that each answers by pinned hash), every receipt (886, all
accepted: each verified against its signer's document on the way in), the world blobs, the
slice manifests (18), the name bindings and the stored cells' versions. What answers
(`check_vectors.py`, recorded in `status/vectors.json`):

- **by pinned hash** (`&fn_hash=`, spec-0.4.2): all 243 formula-eval and excel vectors (those
  in `vectors/invalid/` included) answer with the pin echoed; 222 as the vector says. Of the
  other 21, 19 read names or cells that other vectors bind or fill differently (one name means
  one function at a time; each answers as it says on a mesh holding its world alone, as
  `tests/vectors.rs` shows), one gives its value where the reference evaluator's depth limit
  stops (the mesh evaluates cell by cell), and one depends on the vector's own fuel;
- the stored cells of the formula and excel vectors' worlds: 73 of 87 receipts answer by name
  as of their own asof with exactly that receipt; the other 14 are of names the vectors bind
  to several functions;
- receipt vectors: the 2 stored cells whose descriptor the vector gives and whose name is bound
  to it answer with the vector's receipt; 5 computed ones are evaluated by the mesh (its own
  receipt) and 4 give the vector's value; 6 are `error` `E_EVAL_UNAVAILABLE` (the `expr`
  language has no runtime here, or inputs whose functions the vectors do not describe);
  11 have no descriptor in the vector (their receipts are at `/receipt/<hash>`, not by name);
- by name, the formula-eval and excel vectors are cells of `calc.example.net/e` (or
  `sheet.example.net/case`), which the vectors bind to 108 (78) different functions: the first
  is bound and its vector answers by name; the others answer by pinned hash.

## The seed data (`seed.sh`, `seed/`)

The first live cells, all under `mesh-test.datumesh.com`, loaded with `datumesh-mesh
publish-stored` (signed with the mesh's key; idempotent) from JSON lines that `seed/ecb.py` and
`seed/scripture.py` write into `$DATUMESH_DIR/seed/out` (blobs in `seed/out/blobs/`):

- `ecb/fx_rate(base, quote, date)` (EUR in USD, GBP, JPY, CHF, 30 weekdays to 2026-10-30,
  CC-BY-4.0, and one EUR/NOK cell under `https://mesh-test.datumesh.com/licences/restricted-test`,
  a page Caddy serves), `ecb/holiday(calendar, date)` (TARGET 2026, CC0-1.0) and the formula
  `ecb/fx_cross(base, quote, date)`: synthetic, every summary says test data.
- `scripture/verse_text(corpus, book, chapter, verse, edition)`: a `text/plain` blob of the
  source's bytes: `tanakh`/`uxlc` (UXLC 2.6, build 27.7, OSIS book keys), `nt`/`sblgnt` (SBLGNT
  v1.2, Faithlife/SBLGNT d7f35ad), `quran`/`uthmani` and `quran`/`simple` (Tanzil 1.1, XML;
  book the surah number, chapter 1); `chapter_count(corpus, book)` and `verse_count(corpus,
  book, chapter)` (unit `count`); the formula `strip_niqqud(book, chapter, verse)`, the derived
  edition `uxlc-consonantal`, evaluated on demand (trust `computed`). `seed/scripture.py`'s
  documentation says exactly how each verse's bytes are taken from its source.

`seed/download.sh` fetches the sources into `$DATUMESH_DIR/seed/dl` and checks them against
`seed/SOURCES`; the downloads stay out of the repository and are only read (never run).
`seed.sh` then regenerates the `/find` catalog with the seed's functions, restarts the mesh, and
runs `seed/check_seed.py` (Python datumesh 0.4.2) and `seed/check_seed.mjs` (@datumesh/verify
0.4.2) over HTTPS: `status/seed-load.json` (rows, bytes, non-NFC counts, timings) and
`status/seed-check.json`. `seed.sh --checks` runs the checks only.

## Backups and the drill

`backup.sh` (cron, 03:15 UTC) dumps the database with `pg_dump -Fc` from the Postgres container
into `backups/`, keeping 14. `restore_drill.sh` restores the newest dump into a fresh
`postgres:16` container, starts a second mesh on it (same image, key and configuration, on a
network of its own, no port), and `drill.py` sends the same requests to both meshes (every
stored cell of the vectors as of its receipt's asof, the documents, the bindings, and, once the
seed is loaded, every ECB cell, every tenth scripture cell and a hundred blobs): every answer
must be identical byte for byte. The containers and the network are removed after.

## The resource check

`resource_check.sh` (cron, every 15 minutes) writes `status/resources.json` and the one-line
`status/resources.txt`, appends the line to `logs/resources.log`, and exits 1 when a threshold
is exceeded: disk use of `/` (70%), memory in use (80%, `MemTotal - MemAvailable`), the
Postgres databases' size (50 GB), and egress this month (200 GB). Egress is the VM's whole
outbound traffic (`tx_bytes` of the default route's interface), so it over-counts the mesh;
the month's total is kept in `status/egress.state`, adding the counter's growth each run, or
the whole counter after a reboot reset it. No e-mail (the user's decision 3a).
