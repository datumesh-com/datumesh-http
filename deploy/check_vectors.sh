#!/bin/sh
# Which vectors' cells the test mesh answers (check_vectors.py, with the tests' API key):
# $DATUMESH_DIR/status/vectors.json.   deploy/check_vectors.sh [--local]
set -e
here="$(cd "$(dirname "$0")" && pwd)"
dir="${DATUMESH_DIR:-/root/datumesh-mesh-test}"
extra=""; ca=""
if [ "$1" = "--local" ]; then
  extra="--add-host mesh-test.datumesh.com:host-gateway -v $dir/caddy-internal-root.crt:/ca.crt:ro"; ca=/ca.crt
fi
# shellcheck disable=SC2086
docker run --rm --name datumesh-mesh-test-vectors $extra -v "$here/../spec/vectors:/v:ro" -v "$here:/s:ro" \
  -v "$dir/secrets/tests.api-key:/k:ro" python:3.13-slim sh -c \
  "pip install -q --root-user-action=ignore --disable-pip-version-check datumesh==0.4.1 && python -I /s/check_vectors.py /v https://mesh-test.datumesh.com $ca /k" \
  > "$dir/status/vectors.json"
python3 -c "import json; print(json.load(open('$dir/status/vectors.json'))['summary'])"
