#!/bin/sh
# Smoke test of the datumesh-mesh image: a database of its own on datumesh-pg, a key made
# with `keygen --publish` (in a directory owned by the image's user, mode 0600), the server
# in a container named datumesh-http-smoke on the datumesh network (no port published),
# then its health, key document, mesh document and OpenAPI document answered 200 from
# inside the container. Everything is removed afterwards.
#
#   tools/image_smoke.sh [image]   (default datumesh-http-image:test, built from Dockerfile)
set -e
image="${1:-datumesh-http-image:test}"
db="dm_http_smoke_$$"
secrets="$(mktemp -d)"
url="postgres://postgres:postgres@datumesh-pg:5432/$db?sslmode=disable"
cleanup() {
  docker rm -f datumesh-http-smoke >/dev/null 2>&1 || true
  docker exec datumesh-pg psql -U postgres -qc "DROP DATABASE IF EXISTS $db WITH (FORCE)" >/dev/null 2>&1 || true
  docker run --rm -v "$secrets:/s" debian:trixie-slim rm -rf /s/mesh.key >/dev/null 2>&1 || true
  rm -rf "$secrets"
}
trap cleanup EXIT
docker exec datumesh-pg psql -U postgres -qc "CREATE DATABASE $db"
chown 10001:10001 "$secrets"
env="-e DATABASE_URL=$url -e DATUMESH_HOST=mesh-test.datumesh.com -e DATUMESH_KEY_FILE=/run/secrets/mesh.key"
docker run --rm --network datumesh $env "$image" migrate
docker run --rm --network datumesh $env -v "$secrets:/run/secrets" "$image" keygen --out /run/secrets/mesh.key --publish --valid-from 2026-01-01T00:00:00.000000Z
docker run -d --name datumesh-http-smoke --network datumesh $env -e DATUMESH_LISTEN=0.0.0.0:8080 -v "$secrets:/run/secrets:ro" "$image" >/dev/null
for i in $(seq 1 30); do
  if docker exec datumesh-http-smoke datumesh-mesh health >/dev/null 2>&1; then break; fi
  sleep 1
done
for p in /health /.well-known/datumesh.json /.well-known/datumesh-mesh.json /openapi.json; do
  docker exec datumesh-http-smoke datumesh-mesh health "http://127.0.0.1:8080$p"
done
docker exec datumesh-http-smoke id
docker logs datumesh-http-smoke 2>&1 | head -3
echo "image smoke test passed ($image)"
