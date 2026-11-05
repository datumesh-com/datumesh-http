#!/bin/sh
# Licence inheritance on the test mesh (http 0.2.1, spec-0.4.3), run once after the mesh runs
# the 0.2.1 image; idempotent (run again, nothing changes):
#   1. seed/ecb.py again (deterministic) and `datumesh-mesh publish-stored` of ecb.jsonl: binds
#      the new ecb/fx_cross_undeclared (everything else in the file is already loaded);
#   2. the two declarations of seed/declarations.py into the key document of
#      mesh-test.datumesh.com in the store, with `datumesh-mesh declare` (an entry already there
#      is left as it is): ecb/fx_cross over the restricted test licence, scripture/strip_niqqud
#      over the UXLC licence;
#   3. the /find catalog again (import.sh), and a restart of the mesh.
# Then deploy/smoke.sh checks the result. Containers datumesh-mesh-test-licences-*; the report
# goes to $DATUMESH_DIR/status/licences.json. Nothing secret is printed.
#
# The key document then carries `declarations`: clients on datumesh-core < 0.3.7, datumesh
# < 0.4.3 or @datumesh/verify < 0.4.3 reject it (and so every receipt of the host).
set -e
here="$(cd "$(dirname "$0")" && pwd)"
dir="${DATUMESH_DIR:-/root/datumesh-mesh-test}"
image="${DATUMESH_MESH_IMAGE:-$(docker inspect datumesh-mesh-test-mesh-1 --format '{{.Config.Image}}')}"
host=mesh-test.datumesh.com
mkdir -p "$dir/seed/out" "$dir/status"
docker run --rm --name datumesh-mesh-test-licences-gen -v "$here/seed:/s:ro" -v "$dir/seed/out:/out" python:3.13-slim \
  sh -c "python -I /s/ecb.py /out $host > /out/ecb-report.json"
chmod -R a+rX "$dir/seed/out"
mesh() {
  name="$1"; shift
  docker run --rm --name "datumesh-mesh-test-licences-$name" --network datumesh-mesh-test_backend --env-file "$dir/secrets/db.env" \
    -e DATUMESH_HOST=$host -e DATUMESH_KEY_FILE=/run/secrets/mesh.key -v "$dir/secrets:/run/secrets:ro" \
    -v "$dir/seed/out:/seed:ro" "$image" "$@"
}
published="$(mesh publish publish-stored /seed/ecb.jsonl)"
declared=""
docker run --rm --name datumesh-mesh-test-licences-decl -v "$here/seed:/s:ro" python:3.13-slim \
  python -I /s/declarations.py $host --args > "$dir/status/declarations.txt"
while read -r fn licence from; do
  declared="$declared$(mesh declare declare --fn "$fn" --licence "$licence" --valid-from "$from")
"
done < "$dir/status/declarations.txt"
rm -f "$dir/status/declarations.txt"
"$here/import.sh" > /dev/null
docker restart datumesh-mesh-test-mesh-1 > /dev/null
until [ "$(docker inspect datumesh-mesh-test-mesh-1 --format '{{.State.Health.Status}}')" = healthy ]; do sleep 2; done
python3 -c "
import json, sys
r = {'publish_stored': json.loads('''$published'''), 'declarations': [json.loads(l) for l in '''$declared'''.splitlines() if l]}
json.dump(r, open('$dir/status/licences.json', 'w'), indent=1)
print(json.dumps(r))"
