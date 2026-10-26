#!/bin/sh
# The HTTPS smoke suite (brief test 12): smoke.py (Python datumesh 0.4.1) then smoke.mjs
# (JavaScript @datumesh/verify 0.4.1) against https://mesh-test.datumesh.com, in containers
# named datumesh-mesh-test-smoke-*. Results to $DATUMESH_DIR/status/smoke.json.
#
#   deploy/smoke.sh            the real certificate, through DNS
#   deploy/smoke.sh --local    before DNS: the name mapped to this host, Caddy's internal CA
set -e
here="$(cd "$(dirname "$0")" && pwd)"
dir="${DATUMESH_DIR:-/root/datumesh-mesh-test}"
spec="$(cd "$here/../spec" && pwd)"
base=https://mesh-test.datumesh.com
extra=""; ca=""; nodeca=""
if [ "$1" = "--local" ]; then
  docker cp datumesh-mesh-test-caddy-1:/data/caddy/pki/authorities/local/root.crt "$dir/caddy-internal-root.crt" >/dev/null
  chmod 644 "$dir/caddy-internal-root.crt"
  extra="--add-host mesh-test.datumesh.com:host-gateway -v $dir/caddy-internal-root.crt:/ca.crt:ro"
  ca=/ca.crt; nodeca="-e NODE_EXTRA_CA_CERTS=/ca.crt"
fi
work="$(mktemp -d)"
chmod 755 "$work"
trap 'rm -rf "$work"' EXIT
# shellcheck disable=SC2086
docker run --rm --name datumesh-mesh-test-smoke-py $extra -v "$spec:/spec:ro" -v "$here:/s:ro" -v "$work:/out" \
  -v "$dir/secrets/tests.api-key:/key:ro" python:3.13-slim sh -c \
  "pip install -q --root-user-action=ignore --disable-pip-version-check datumesh==0.4.1 jsonschema==4.25.1 && python -I /s/smoke.py $base /spec /key $ca > /out/smoke.json; rc=\$?; cp /tmp/receipt.json /out/ 2>/dev/null; exit \$rc" \
  || { cat "$work/smoke.json"; cp "$work/smoke.json" "$dir/status/smoke.json"; exit 1; }
# shellcheck disable=SC2086
docker run --rm --name datumesh-mesh-test-smoke-js $extra $nodeca -v "$here:/s:ro" -v "$work:/out" node:22-slim sh -c \
  "mkdir -p /tmp/x && cp /s/smoke.mjs /tmp/x/ && cd /tmp/x && npm install -s --no-fund --no-audit @datumesh/verify@0.4.1 >/dev/null && node smoke.mjs $base /out/receipt.json" > "$work/js.json" || { cat "$work/js.json"; echo "javascript part failed" >&2; exit 1; }
python3 -c "
import json,sys
d=json.load(open('$work/smoke.json')); d['javascript']=json.load(open('$work/js.json')); d['run']='$(date -u +%FT%TZ)'
json.dump(d, open('$dir/status/smoke.json','w'), indent=1)
print(d['checks'], 'checks,', d['failed'], 'failed; javascript:', d['javascript'])"
