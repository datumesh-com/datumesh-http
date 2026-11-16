# Runbook: the production mesh at datumesh.com

The site (`www.datumesh.com` and every other path of `datumesh.com`) is the Worker `datumesh-holding` (repository datumesh-site).

The mesh answers the paths below, on **quran-box** (Contabo, 5.189.160.91, Debian 12):
- **Location:** Compose project `datumesh` in `/srv/datumesh`, run by the user `datumesh`.
- **Reached through:** the Caddy of the app `khatm` (`khatm-caddy-1`), on the Docker network `khatm_default`, as `datumesh-mesh:8080`.
- **Published host ports:** none.

## How a request reaches the mesh
1. **DNS:** `A datumesh.com → 5.189.160.91`, proxied (orange cloud).
2. **Cloudflare Worker routes:**
   - `datumesh.com/*` and `www.datumesh.com/*` run the site's Worker.
   - These routes have **no script**, so requests on them skip the Worker and go to the origin:
     - `datumesh.com/cell/*`, `/cells*`, `/slice/*`, `/receipt/*`, `/descriptor/*`, `/manifest/*`, `/versions/*`, `/blob/*`;
     - `/explain/*`, `/find*`, `/verify*`, `/jobs/*`, `/bindings*`, `/describe/*`, `/openapi.json*`;
     - `/.well-known/datumesh.json*`, `/.well-known/datumesh-mesh.json*`, `/.well-known/acme-challenge/*`.
   - A route whose path can carry a query string ends in `*`.
   - `/health` and `/metrics` have no route, so the public sees the site's 404.
   - `cloudflare.py plan` shows the state, and `cloudflare.py routes` / `undo-routes` add or remove exactly these routes.
3. **khatm's Caddy:**
   - The site block for `datumesh.com` (`site-block.caddy`) refuses anything not from Cloudflare.
   - It sets `X-Forwarded-For` from `CF-Connecting-IP`, the mesh's rate-limit address (`DATUMESH_TRUST_PROXY=1`, the last entry).
   - It proxies to `datumesh-mesh:8080`.
   - **The block lives in khatm's source on the operator's machine.** khatm's `deploy.sh` rsyncs that source with `--delete`, so a block edited only on the server is lost on khatm's next deploy.
   - The maintenance agent applies a change and reloads: `cd /srv/khatm && docker compose exec caddy caddy reload --config /etc/caddy/Caddyfile`.
4. **Certificate:**
   - Let's Encrypt, obtained by Caddy with the HTTP-01 challenge through Cloudflare (that's why `/.well-known/acme-challenge/*` has its own route).
   - There's no `import cf`: khatm's Cloudflare token isn't known to cover `datumesh.com`.
   - Zone SSL mode: `full`. It may go to `full (strict)` once the certificate is issued.

## The Cloudflare IP list goes stale
The site block lists Cloudflare's address ranges. Cloudflare changes them rarely, but it does change them. A new Cloudflare range missing from the list gets **403** for every request through it, and the mesh looks down from some places.
- **Monthly**, or whenever `uptime.sh` or a user reports 403s from the mesh while the site works: run `deploy/prod/cf_ips.sh` here.
  - It compares the block with https://api.cloudflare.com/client/v4/ips (the list at https://www.cloudflare.com/ips/).
  - If they differ, it prints the new `@notcf` line.
- **To refresh:**
  1. Put the line in `site-block.caddy` here, with the new etag and date in its comment.
  2. Send the whole block to the operator, to replace the `datumesh.com` block in khatm's source.
  3. The maintenance agent applies it and reloads Caddy (the command above).
  4. Check that `cf_ips.sh` passes and that `uptime.sh` shows OK.

## State on quran-box (`/srv/datumesh`, owner `datumesh`, mode 750)

| Path | What | Owner / mode |
|---|---|---|
| `compose.yml`, `.env` | the project; `.env` holds the image digests only (`MESH_DIGEST`, `POSTGRES_DIGEST`, `PYTHON_DIGEST`) | datumesh |
| `env/db.env` | `POSTGRES_PASSWORD`, `DATABASE_URL` | datumesh, 0600 (dir 0700) |
| `env/backup_ed25519` | the key that can only write into one directory on vmi630775 | datumesh, 0600 |
| `secrets/mesh.key` | **the production signing key**, generated here by `keygen`; never copied, never printed, not backed up | uid 10001, 0600 (dir 0700) |
| `secrets/api-keys`, `secrets/user.api-key` | API key hashes, and the user's key | uid 10001, 0600 |
| `pgdata/` | Postgres data | Postgres's uid, 0700 |
| `catalog.json` | the `/find` catalog (names and descriptions) | datumesh, 0644 |
| `backups/` | nightly dumps, the 14 newest | datumesh, 0700 |
| `bin/`, `logs/`, `status/` | installed scripts, cron logs, check and drill results | datumesh |

## Jobs
- **quran-box, the `datumesh` user's crontab** (`install_cron.sh`):
  - `backup.sh` at 03:15 UTC: a dump, then a copy to vmi630775 at `/root/datumesh-prod-backups` (user `dmbackup`, `rrsync -wo`, no shell);
  - `check.sh` every 5 minutes, the error alert: a new 5xx or `error` envelope, a container down, the mesh unreachable, or disk, memory or Postgres over threshold.
- **vmi630775, root's crontab:** `uptime.sh` every 5 minutes. Through Cloudflare, the key document and Genesis 1:1 must answer; two failures in a row are an ALERT.
- **Alerts** are log lines and status files (`status/check.txt`, `/root/datumesh-prod/status/uptime.txt`), as for mesh-test. No e-mail is configured.
- **Restore drill:** `bin/restore_drill.sh`, by hand. It restores the newest dump into a scratch Postgres with a second mesh, compares the answers with the live mesh's, and times the restore (`status/drill.json`).

## Writes
- The only write path is `datumesh-mesh publish-stored` (http ≥ 0.3.1, which writes nothing from a refused file), run on quran-box against the project's network.
- The scripture load is `scripture_prod.py`, checked by `check_scripture_prod.py`, in the files' order: 00, 10, 20, 30.
- There is no network write endpoint.
