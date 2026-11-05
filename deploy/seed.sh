#!/bin/sh
# The seed data (contract 0.1.1, section 4): download the scripture sources (checked against
# seed/SOURCES) into $DATUMESH_DIR/seed/dl, generate the JSON lines and blobs into
# $DATUMESH_DIR/seed/out (seed/scripture.py, seed/ecb.py, in python:3.13-slim), load them with
# `datumesh-mesh publish-stored` (signed with the mesh's key; idempotent), then run the seed
# checks over HTTPS (seed/check_seed.py with Python datumesh 0.4.3, seed/check_seed.mjs with
# @datumesh/verify 0.4.3). Reports to $DATUMESH_DIR/status/seed-*.json. Nothing secret is printed.
#
#   deploy/seed.sh [--checks] [--local]   (--checks: the checks only, of what was loaded;
#                                          --local: before DNS, Caddy's internal CA, as smoke.sh)
set -e
here="$(cd "$(dirname "$0")" && pwd)"
dir="${DATUMESH_DIR:-/root/datumesh-mesh-test}"
image="${DATUMESH_MESH_IMAGE:-$(docker inspect datumesh-mesh-test-mesh-1 --format '{{.Config.Image}}')}"
spec="$(cd "$here/../spec" && pwd)"
host=mesh-test.datumesh.com
base="https://$host"
checks_only=false
if [ "$1" = "--checks" ]; then checks_only=true; shift; fi
mkdir -p "$dir/seed/dl" "$dir/seed/out"
if ! $checks_only; then
chmod 755 "$dir/seed" "$dir/seed/out"
"$here/seed/download.sh" "$dir/seed/dl"
t0=$(date +%s)
docker run --rm --name datumesh-mesh-test-seed-gen -v "$dir/seed/dl:/dl:ro" -v "$spec:/spec:ro" -v "$here/seed:/s:ro" \
  -v "$dir/seed/out:/out" python:3.13-slim sh -c \
  "python -I /s/scripture.py /dl /spec /out $host > /dev/null && python -I /s/ecb.py /out $host > /out/ecb-report.json"
chmod -R a+rX "$dir/seed/out"
t1=$(date +%s)
load() {
  docker run --rm --name datumesh-mesh-test-seed-load --network datumesh-mesh-test_backend --env-file "$dir/secrets/db.env" \
    -e DATUMESH_HOST=$host -e DATUMESH_KEY_FILE=/run/secrets/mesh.key -v "$dir/secrets:/run/secrets:ro" \
    -v "$dir/seed/out:/seed:ro" "$image" publish-stored "/seed/$1"
}
ecb="$(load ecb.jsonl)"
scripture="$(load scripture.jsonl)"
t2=$(date +%s)
python3 -c "
import json
r = {'generated_seconds': $t1 - $t0, 'loaded_seconds': $t2 - $t1, 'ecb': json.loads('''$ecb'''), 'scripture': json.loads('''$scripture'''),
     'sources': json.load(open('$dir/seed/out/scripture-report.json')), 'ecb_cells': json.load(open('$dir/seed/out/ecb-report.json'))}
json.dump(r, open('$dir/status/seed-load.json', 'w'), indent=1)
print(json.dumps({k: r[k] for k in ('generated_seconds', 'loaded_seconds', 'ecb', 'scripture')}))"
# The /find catalog gains the seed's functions; the mesh reads it at start.
"$here/import.sh" > /dev/null
docker restart datumesh-mesh-test-mesh-1 > /dev/null
until [ "$(docker inspect datumesh-mesh-test-mesh-1 --format '{{.State.Health.Status}}')" = healthy ]; do sleep 2; done
fi
extra=""; ca=""; nodeca=""
if [ "$1" = "--local" ]; then
  extra="--add-host $host:host-gateway -v $dir/caddy-internal-root.crt:/ca.crt:ro"; ca=/ca.crt; nodeca="-e NODE_EXTRA_CA_CERTS=/ca.crt"
fi
work="$(mktemp -d)"; chmod 755 "$work"
trap 'rm -rf "$work"' EXIT
# shellcheck disable=SC2086
docker run --rm --name datumesh-mesh-test-seed-check-py $extra -v "$here/seed:/s:ro" -v "$dir/seed/out:/seed:ro" -v "$work:/out" \
  -v "$dir/secrets/tests.api-key:/key:ro" python:3.13-slim sh -c \
  "pip install -q --root-user-action=ignore --disable-pip-version-check datumesh==0.4.3 && python -I /s/check_seed.py $base /seed /key /out $ca > /out/seed-check.json" \
  || { cp "$work/seed-check.json" "$dir/status/seed-check.json"; echo "seed checks failed: $dir/status/seed-check.json" >&2; exit 1; }
# shellcheck disable=SC2086
docker run --rm --name datumesh-mesh-test-seed-check-js $extra $nodeca -v "$here/seed:/s:ro" -v "$work:/out" -v "$dir/secrets/tests.api-key:/key:ro" \
  node:22-slim sh -c "mkdir -p /tmp/x && cp /s/check_seed.mjs /tmp/x/ && cd /tmp/x && npm install -s --no-fund --no-audit @datumesh/verify@0.4.3 >/dev/null && node check_seed.mjs $base /out/verses.json /key" \
  > "$work/js.json" || { cat "$work/js.json"; echo "javascript part failed" >&2; exit 1; }
python3 -c "
import json
d = json.load(open('$work/seed-check.json')); d['javascript'] = json.load(open('$work/js.json'))
json.dump(d, open('$dir/status/seed-check.json', 'w'), indent=1, ensure_ascii=False)
print(d['checks'], 'checks,', d['failed'], 'failed,', d['seconds'], 's; javascript:', json.dumps(d['javascript']))"
