#!/bin/sh
# Request binding through the client libraries: the envelope cases tests/binding.rs wrote
# (responses of the real endpoint, each with the request it is checked against and the
# expected result) checked with the Python library datumesh 0.4.1 (PyPI) and the JavaScript
# library @datumesh/verify 0.4.1 (npm). The Rust core 0.3.5 checks them in the test itself.
#
#   tools/client_check.sh [cases.jsonl]            in python:3.13-slim and node:22-slim containers
#   tools/client_check.sh --native [cases.jsonl]   with the local python3/pip and node/npm (CI)
set -e
root="$(cd "$(dirname "$0")/.." && pwd)"
native=false
if [ "$1" = "--native" ]; then native=true; shift; fi
cases="$(cd "$(dirname "${1:-$root/target-out/binding-cases.jsonl}")" && pwd)/$(basename "${1:-$root/target-out/binding-cases.jsonl}")"
if $native; then
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  python3 -m venv "$work/venv"
  "$work/venv/bin/pip" install -q --disable-pip-version-check datumesh==0.4.1
  "$work/venv/bin/python" -I "$root/tests/clients/check.py" "$cases"
  cp "$root/tests/clients/check.mjs" "$work/"
  (cd "$work" && npm install -s --no-fund --no-audit @datumesh/verify@0.4.1 >/dev/null && node check.mjs "$cases")
else
  dir="$(dirname "$cases")"
  name="$(basename "$cases")"
  docker run --rm --name "datumesh-http-py-$$" -v "$dir:/d:ro" -v "$root/tests/clients:/s:ro" python:3.13-slim \
    sh -c "pip install -q --disable-pip-version-check --root-user-action=ignore datumesh==0.4.1 && python -I /s/check.py /d/$name"
  docker run --rm --name "datumesh-http-js-$$" -v "$dir:/d:ro" -v "$root/tests/clients:/s:ro" node:22-slim \
    sh -c "mkdir -p /tmp/x && cp /s/check.mjs /tmp/x/ && cd /tmp/x && npm install -s --no-fund --no-audit @datumesh/verify@0.4.1 >/dev/null && node check.mjs /d/$name"
fi
