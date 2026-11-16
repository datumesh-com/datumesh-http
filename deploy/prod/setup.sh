#!/bin/sh
# One-time setup of the production mesh on quran-box, run there as `datumesh` in /srv/datumesh
# (idempotent: what exists is kept). Nothing secret is printed.
#
#   ./setup.sh            (needs .env with MESH_DIGEST and POSTGRES_DIGEST, and compose.yml here)
#
# Makes the directories, the Postgres password, the store's schema, and the production signing
# key, generated here by the image's `keygen` (it never exists anywhere else), with its key
# document published into the store; then one API key for the user. The store is otherwise empty.
set -e
dir=/srv/datumesh
cd "$dir"
umask 077
. ./.env
image="ghcr.io/datumesh-com/datumesh-mesh@sha256:$MESH_DIGEST"
mkdir -p env secrets backups status logs pgdata
chmod 700 env secrets backups pgdata
chmod 755 status logs
if [ ! -f env/db.env ]; then
  pw="$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
  printf 'POSTGRES_PASSWORD=%s\nDATABASE_URL=postgres://postgres:%s@datumesh-postgres:5432/datumesh?sslmode=disable\n' "$pw" "$pw" > env/db.env
  chmod 600 env/db.env
fi
[ -f catalog.json ] || { echo '{}' > catalog.json; chmod 644 catalog.json; }
docker compose -f compose.yml up -d datumesh-postgres
until docker compose -f compose.yml exec -T datumesh-postgres pg_isready -U postgres -d datumesh >/dev/null 2>&1; do sleep 1; done
# secrets/ belongs to the mesh's user (uid 10001), mode 0700: the datumesh user cannot list it.
# The chown runs in a throwaway container (the docker group's root), on this directory only.
docker run --rm --user 0 -v "$dir/secrets:/s" "postgres:16@sha256:$POSTGRES_DIGEST" sh -c 'chown 10001:10001 /s && chmod 700 /s'
# The image's tools on the project's internal network, as the image's own user (uid 10001).
run() {
  docker run --rm --network datumesh_backend --env-file env/db.env -e DATUMESH_HOST=datumesh.com \
    -v "$dir/secrets:/run/secrets" "$image" "$@"
}
run migrate
# What exists is kept: a file's presence is tested as root in a throwaway container.
has() { docker run --rm --user 0 -v "$dir/secrets:/s:ro" "postgres:16@sha256:$POSTGRES_DIGEST" test -f "/s/$1"; }
has mesh.key || run keygen --out /run/secrets/mesh.key --publish
has user.api-key || \
  run apikey --principal user --keys-file /run/secrets/api-keys --out /run/secrets/user.api-key > /dev/null
# Modes and owners only (names, never contents).
docker run --rm --user 0 -v "$dir/secrets:/s:ro" "postgres:16@sha256:$POSTGRES_DIGEST" ls -ln /s | sed 1d | awk '{print $1, $3, $4, $9}'
echo "setup done: $dir"
