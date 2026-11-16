#!/bin/sh
# The restore drill (quran-box): the newest dump restored into a scratch Postgres 16 (its own
# container and internal network, nothing published), then a second mesh started on it with the
# live mesh's image, key and configuration, and the answers of the two compared byte for byte for a
# sample of cells (drill_check.py). Times the restore. The drill's containers and network are
# removed afterwards. Result in status/drill.json.
set -e
dir=/srv/datumesh
cd "$dir"
. ./.env
image="ghcr.io/datumesh-com/datumesh-mesh@sha256:$MESH_DIGEST"
pg="postgres:16@sha256:$POSTGRES_DIGEST"
net=datumesh-drill
dump="$(ls -1t backups/datumesh-*.dump | head -n 1)"
[ -n "$dump" ] || { echo "no dump in backups/" >&2; exit 1; }
cleanup() {
  docker rm -f datumesh-drill-check datumesh-drill-mesh datumesh-drill-pg >/dev/null 2>&1 || true
  docker network rm "$net" >/dev/null 2>&1 || true
}
trap cleanup EXIT
cleanup
pw="$(head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n')"
docker network create --internal "$net" >/dev/null
docker run -d --name datumesh-drill-pg --network "$net" -e POSTGRES_PASSWORD="$pw" -e POSTGRES_DB=datumesh "$pg" >/dev/null
until docker exec datumesh-drill-pg pg_isready -U postgres -d datumesh >/dev/null 2>&1; do sleep 1; done
sleep 2
t0=$(date +%s.%N)
docker exec -i datumesh-drill-pg pg_restore -U postgres -d datumesh --no-owner < "$dump"
t1=$(date +%s.%N)
# The live mesh's configuration (DATUMESH_* only; no migration), its own database, nothing published.
env="$(docker inspect datumesh-datumesh-mesh-1 --format '{{range .Config.Env}}{{println .}}{{end}}' | grep '^DATUMESH_' | grep -v '^DATUMESH_MIGRATE=' | sed 's/^/-e /' | tr '\n' ' ')"
# shellcheck disable=SC2086
docker run -d --name datumesh-drill-mesh --network "$net" $env \
  -e DATABASE_URL="postgres://postgres:$pw@datumesh-drill-pg:5432/datumesh?sslmode=disable" \
  -v "$dir/secrets:/run/secrets:ro" -v "$dir/catalog.json:/etc/datumesh/catalog.json:ro" "$image" >/dev/null
until docker exec datumesh-drill-mesh datumesh-mesh health >/dev/null 2>&1; do sleep 1; done
t2=$(date +%s.%N)
# The comparing container joins the drill's network and the live project's backend network only.
docker create --name datumesh-drill-check -v "$dir/bin/drill_check.py:/c.py:ro" python:3.13-slim@sha256:$PYTHON_DIGEST \
  python -I /c.py http://datumesh-mesh:8080 http://datumesh-drill-mesh:8080 >/dev/null
docker network connect "$net" datumesh-drill-check
docker network connect datumesh_backend datumesh-drill-check
set +e
docker start -a datumesh-drill-check > status/drill-compare.json
rc=$?
set -e
printf '{"at": "%s", "dump": "%s", "dump_bytes": %s, "restore_seconds": %.1f, "mesh_up_seconds": %.1f, "compare_exit": %s}\n' \
  "$(date -u +%FT%TZ)" "$(basename "$dump")" "$(wc -c < "$dump")" "$(awk "BEGIN{print $t1 - $t0}")" "$(awk "BEGIN{print $t2 - $t1}")" "$rc" \
  | tee status/drill.json
exit $rc
