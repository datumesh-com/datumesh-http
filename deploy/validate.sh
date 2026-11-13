#!/bin/sh
# The weekly domain re-check (cron, deploy/install_cron.sh): `datumesh-mesh validate --all` in a
# one-off container of the running mesh's image. Every namespace in the store but the mesh's own
# is fetched from its own host (https://<host>/.well-known/datumesh.json) and checked; the
# outcome is recorded in the store: a valid document replaces the stored one (a key rotation is
# picked up), the second consecutive failure suspends the namespace (new writes under it are
# refused; reads and existing receipts are not affected), the next success reinstates it.
# One JSON line per host to stdout (the cron log); exits 1 when any host is invalid.
#
# The mesh's backend network has no route out, so the container is also on the frontend
# network (egress) for the fetches. Nothing secret is printed. `validate.sh <host>` checks one.
set -e
dir="${DATUMESH_DIR:-/root/datumesh-mesh-test}"
image="${DATUMESH_MESH_IMAGE:-$(docker inspect datumesh-mesh-test-mesh-1 --format '{{.Config.Image}}')}"
host=mesh-test.datumesh.com
if [ "$#" -eq 0 ]; then set -- --all; fi
name="datumesh-mesh-test-validate-$$"
docker create --name "$name" --network datumesh-mesh-test_backend --env-file "$dir/secrets/db.env" \
  -e DATUMESH_HOST=$host -e DATUMESH_NAMESPACES="${DATUMESH_NAMESPACES:-$(docker inspect datumesh-mesh-test-mesh-1 --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^DATUMESH_NAMESPACES=//p')}" \
  "$image" validate "$@" > /dev/null
trap 'docker rm -f "$name" > /dev/null 2>&1 || true' EXIT
docker network connect datumesh-mesh-test_frontend "$name"
echo "$(date -u +%FT%TZ) validate $*"
docker start -a "$name"
