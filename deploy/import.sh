#!/bin/sh
# Imports the spec vectors' cells into the test mesh's store (datumesh-mesh
# import-vectors: key documents, descriptors, receipts, manifests, bindings, versions; no
# other data) and writes the /find catalog of the names bound. Idempotent. The report goes to
# $DATUMESH_DIR/status/import.json.
set -e
here="$(cd "$(dirname "$0")" && pwd)"
dir="${DATUMESH_DIR:-/root/datumesh-mesh-test}"
vectors="$(cd "$here/../spec/vectors" && pwd)"
image="${DATUMESH_MESH_IMAGE:-$(docker inspect datumesh-mesh-test-mesh-1 --format '{{.Config.Image}}' 2>/dev/null || echo datumesh-mesh:0.2.0-local)}"
docker run --rm --network datumesh-mesh-test_backend --env-file "$dir/secrets/db.env" -v "$vectors:/vectors:ro" \
  "$image" import-vectors /vectors > "$dir/status/import.json"
cat "$dir/status/import.json"; echo
# The catalog: each bound name with the description of the first vector defining it, and the
# seed's functions (deploy/seed.sh) when it has run.
mkdir -p "$dir/seed/out"
docker run --rm -v "$vectors:/v:ro" -v "$dir/seed/out:/seed:ro" python:3.13-slim python -I -c '
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
        cat.setdefault(n, "From the spec vectors: " + v["description"][:200])
about = {
    "ecb/fx_rate": "Synthetic test data, ECB-shaped: 1 EUR in USD, GBP, JPY, CHF (and one NOK cell under a restricted test licence), 30 weekdays to 2026-10-30.",
    "ecb/holiday": "Synthetic test data: TARGET closing days of 2026.",
    "ecb/fx_cross": "Test formula: the cross rate of two fx_rate quotes against EUR.",
    "scripture/verse_text": "The text of a verse, as the source publishes it (a text/plain blob): Tanakh (UXLC 2.6), New Testament (SBLGNT v1.2), Quran (Tanzil 1.1, Uthmani and Simple).",
    "scripture/chapter_count": "The number of chapters of a book of a corpus.",
    "scripture/verse_count": "The number of verses of a chapter of a book of a corpus.",
    "scripture/strip_niqqud": "The UXLC verse without points and accents (the uxlc-consonantal edition), computed by formula.",
}
for f in glob.glob("/seed/*.jsonl"):
    for line in open(f, encoding="utf-8"):
        l = json.loads(line)
        n = l.get("function") or l.get("formula")
        if n:
            cat[n] = about.get(n.split("/", 1)[1], "Seed data of the test mesh.")
print(json.dumps(cat, indent=1, sort_keys=True))
' > "$dir/catalog.json.new"
mv "$dir/catalog.json.new" "$dir/catalog.json"
chmod 644 "$dir/catalog.json"
