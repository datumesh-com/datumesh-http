#!/bin/sh
# The restore drill: the newest dump in $DATUMESH_DIR/backups restored into a fresh Postgres 16
# container, a second mesh started on it (same image, key and configuration; no port
# published, on a network of its own), then drill.py compares its answers with the live
# mesh's byte for byte. The drill's containers and network are removed afterwards; the result
# goes to $DATUMESH_DIR/status/drill.json.
set -e
here="$(cd "$(dirname "$0")" && pwd)"
dir="${DATUMESH_DIR:-/root/datumesh-mesh-test}"
image="${DATUMESH_MESH_IMAGE:-ghcr.io/datumesh-com/datumesh-mesh@sha256:7b295a2c94b458d7d7a088859b91a3863fcd6fa32b90c292f9d19389642f5441}"
net=datumesh-mesh-test-drill
dump="$(ls -1t "$dir"/backups/datumesh-*.dump | head -n 1)"
[ -n "$dump" ] || { echo "no dump in $dir/backups" >&2; exit 1; }
cleanup() {
  docker rm -f datumesh-mesh-test-drill-check datumesh-mesh-test-drill-mesh datumesh-mesh-test-drill-pg >/dev/null 2>&1 || true
  docker network rm "$net" >/dev/null 2>&1 || true
}
trap cleanup EXIT
cleanup
pw="$(head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n')"
docker network create --internal "$net" >/dev/null
docker run -d --name datumesh-mesh-test-drill-pg --network "$net" -e POSTGRES_PASSWORD="$pw" -e POSTGRES_DB=datumesh postgres:16 >/dev/null
until docker exec datumesh-mesh-test-drill-pg pg_isready -U postgres -d datumesh >/dev/null 2>&1; do sleep 1; done
sleep 2
docker exec -i datumesh-mesh-test-drill-pg pg_restore -U postgres -d datumesh --no-owner < "$dump"
# The live mesh's configuration, but its own database and nothing published.
env="$(docker inspect datumesh-mesh-test-mesh-1 --format '{{range .Config.Env}}{{println .}}{{end}}' | grep '^DATUMESH_' | grep -v '^DATUMESH_MIGRATE=' | sed 's/^/-e /' | tr '\n' ' ')"
# shellcheck disable=SC2086
docker run -d --name datumesh-mesh-test-drill-mesh --network "$net" $env \
  -e DATABASE_URL="postgres://postgres:$pw@datumesh-mesh-test-drill-pg:5432/datumesh?sslmode=disable" \
  -v "$dir/secrets:/run/secrets:ro" -v "$dir/catalog.json:/etc/datumesh/catalog.json:ro" "$image" >/dev/null
until docker exec datumesh-mesh-test-drill-mesh datumesh-mesh health >/dev/null 2>&1; do sleep 1; done
vectors="$(cd "$here/../spec/vectors" && pwd)"
# The comparing container: on the default bridge (to install the Python library) and on both
# internal networks, the drill's and the live project's.
docker rm -f datumesh-mesh-test-drill-check >/dev/null 2>&1 || true
docker create --name datumesh-mesh-test-drill-check -v "$vectors:/v:ro" -v "$here:/s:ro" python:3.13-slim \
  sh -c 'pip install -q --root-user-action=ignore --disable-pip-version-check datumesh==0.4.1 2>/dev/null; python -I /s/drill.py /v http://mesh:8080 http://datumesh-mesh-test-drill-mesh:8080' >/dev/null
docker network connect "$net" datumesh-mesh-test-drill-check
docker network connect datumesh-mesh-test_backend datumesh-mesh-test-drill-check
set +e
docker start -a datumesh-mesh-test-drill-check > "$dir/status/drill.json"
rc=$?
docker rm -f datumesh-mesh-test-drill-check >/dev/null
set -e
echo "{\"drill\": \"$(date -u +%FT%TZ)\", \"dump\": \"$(basename "$dump")\", \"exit\": $rc}" > "$dir/status/drill-run.json"
cat "$dir/status/drill.json"
exit $rc
