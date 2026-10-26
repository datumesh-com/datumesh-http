#!/bin/sh
# Imports the spec-0.4.1 vectors' cells into the test mesh's store (datumesh-mesh
# import-vectors: key documents, descriptors, receipts, manifests, bindings, versions; no
# other data) and writes the /find catalog of the names bound. Idempotent. The report goes to
# $DATUMESH_DIR/status/import.json.
set -e
here="$(cd "$(dirname "$0")" && pwd)"
dir="${DATUMESH_DIR:-/root/datumesh-mesh-test}"
vectors="$(cd "$here/../spec/vectors" && pwd)"
image="${DATUMESH_MESH_IMAGE:-datumesh-mesh:0.1.0-local}"
docker run --rm --network datumesh-mesh-test_backend --env-file "$dir/secrets/db.env" -v "$vectors:/vectors:ro" \
  "$image" import-vectors /vectors > "$dir/status/import.json"
cat "$dir/status/import.json"; echo
# The catalog: each bound name with the description of the first vector defining it.
docker run --rm -v "$vectors:/v:ro" python:3.13-slim python -I -c '
import json, glob
cat = {}
for p in sorted(glob.glob("/v/*/*.json")):
    v = json.load(open(p))
    if not v.get("valid"):
        continue
    names = list(v.get("world", {}).get("functions", {}))
    if v["kind"] == "receipt" and isinstance(v.get("recompute", {}).get("descriptor"), dict):
        c = json.loads(v["input"])["cell"]
        names.append(c["ns"] + "/" + c["fn"])
    for n in names:
        cat.setdefault(n, "From the spec-0.4.1 vectors: " + v["description"][:200])
print(json.dumps(cat, indent=1, sort_keys=True))
' > "$dir/catalog.json.new"
mv "$dir/catalog.json.new" "$dir/catalog.json"
chmod 644 "$dir/catalog.json"
