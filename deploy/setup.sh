#!/bin/sh
# One-time setup of the test mesh's state outside the repository (idempotent: what exists is
# kept): the directories, the Postgres password, the signing key and its key document, the
# API keys (one for the user, one for the tests), the vectors' cells, the /find catalog.
# Nothing secret is printed.
#
#   deploy/setup.sh            (after the image is built: docker build -t datumesh-mesh:0.1.0-local .)
set -e
here="$(cd "$(dirname "$0")" && pwd)"
dir="${DATUMESH_DIR:-/root/datumesh-mesh-test}"
image="${DATUMESH_MESH_IMAGE:-datumesh-mesh:0.1.0-local}"
compose="docker compose -p datumesh-mesh-test -f $here/compose.yml"
umask 077
mkdir -p "$dir/secrets" "$dir/backups" "$dir/status" "$dir/logs"
chmod 700 "$dir" "$dir/secrets" "$dir/backups"
chmod 755 "$dir/status"
# The mesh's user (uid 10001) reads the secrets directory; its files are its own, mode 0600.
chown 10001:10001 "$dir/secrets"
if [ ! -f "$dir/secrets/db.env" ]; then
  pw="$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
  printf 'POSTGRES_PASSWORD=%s\nDATABASE_URL=postgres://postgres:%s@postgres:5432/datumesh?sslmode=disable\n' "$pw" "$pw" > "$dir/secrets/db.env"
  chmod 600 "$dir/secrets/db.env"
fi
[ -f "$dir/catalog.json" ] || { echo '{}' > "$dir/catalog.json"; chmod 644 "$dir/catalog.json"; }
$compose up -d postgres
until $compose exec -T postgres pg_isready -U postgres -d datumesh >/dev/null 2>&1; do sleep 1; done
# The image's tools, on the project's internal network, with the secrets writable.
run() {
  docker run --rm --network datumesh-mesh-test_backend --env-file "$dir/secrets/db.env" \
    -e DATUMESH_HOST=mesh-test.datumesh.com -v "$dir/secrets:/run/secrets" "$image" "$@"
}
run migrate
if [ ! -f "$dir/secrets/mesh.key" ]; then
  run keygen --out /run/secrets/mesh.key --publish
fi
for who in user tests; do
  if [ ! -f "$dir/secrets/$who.api-key" ]; then
    run apikey --principal "$who" --keys-file /run/secrets/api-keys --out "/run/secrets/$who.api-key" > /dev/null
  fi
done
"$here/import.sh"
echo "setup done: $dir"
