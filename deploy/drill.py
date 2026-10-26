# SPDX-License-Identifier: Apache-2.0
"""The restore drill's comparison: the same requests to the live mesh and to the mesh on the
restored database, compared byte for byte (status and body). The sample: every stored cell of
the vectors' worlds and stored receipt vectors, as of its receipt's asof (an answer that never
changes), plus the key and mesh documents and the bindings of every name.

    python drill.py <vectors dir> <live base URL> <restored base URL>   (JSON to stdout; exit 1 on a difference)
"""
import json
import pathlib
import sys
import urllib.error
import urllib.request

import datumesh

vectors, live, restored = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]


def get(base, path):
    try:
        with urllib.request.urlopen(base + path, timeout=30) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


paths, names = ["/.well-known/datumesh.json", "/.well-known/datumesh-mesh.json"], set()
seen = set()
for p in sorted(vectors.glob("*/*.json")):
    v = json.loads(p.read_text())
    if not v.get("valid"):
        continue
    rs = list(v.get("world", {}).get("receipts", []))
    if v["kind"] == "receipt":
        rs.append(json.loads(v["input"]))
    for r in rs:
        c = r["cell"]
        uri = datumesh.format_uri({"ns": c["ns"], "fn": c["fn"], "args": c["args"], "asof": r["asof"]})
        if uri not in seen:
            seen.add(uri)
            paths.append("/cell/" + uri[len("cell://"):])
        names.add(c["ns"] + "/" + c["fn"])
for n in sorted(names):
    paths.append("/bindings?name=" + n.replace("/", "%2F") + "&asof=2026-06-01T00:00:00.000000Z")
same, differ, ok = 0, [], 0
for path in paths:
    a, b = get(live, path), get(restored, path)
    if a == b:
        same += 1
        ok += a[0] == 200
    else:
        differ.append(path)
print(json.dumps({"requests": len(paths), "identical": same, "identical_200": ok, "different": differ}, indent=1))
sys.exit(1 if differ else 0)
