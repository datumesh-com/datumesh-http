# SPDX-License-Identifier: Apache-2.0
"""The restore drill's comparison: the same requests to the live mesh and to the mesh on the
restored database, compared byte for byte (status and body). The sample: every stored cell of
the vectors' worlds and stored receipt vectors, as of its receipt's asof (an answer that never
changes), plus the key and mesh documents and the bindings of every name; with a seed dir
(deploy/seed.sh's output), also every ECB cell and every tenth scripture cell as of its asof,
and their blobs' first hundred.

    python drill.py <vectors dir> <live base URL> <restored base URL> [seed dir]   (JSON to stdout; exit 1 on a difference)
"""
import json
import os
import pathlib
import sys
import time
import urllib.error
import urllib.request

import datumesh

vectors, live, restored = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]


# The tests' API key (the same on both meshes: they read the same secrets), when given in
# DATUMESH_DRILL_KEY_FILE: past the anonymous rate limit.
KEY = open(os.environ["DATUMESH_DRILL_KEY_FILE"]).read().strip() if os.environ.get("DATUMESH_DRILL_KEY_FILE") else None


def get(base, path):
    """The status and body; a rate-limit refusal is waited out (Retry-After) and asked again."""
    h = {"Authorization": "Bearer " + KEY} if KEY else {}
    for _ in range(30):
        try:
            with urllib.request.urlopen(urllib.request.Request(base + path, headers=h), timeout=30) as r:
                return r.status, r.read()
        except urllib.error.HTTPError as e:
            if e.code != 429:
                return e.code, e.read()
            time.sleep(int(e.headers.get("Retry-After", "1")) + 0.2)
    return 429, b""


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
if len(sys.argv) > 4:
    seed = pathlib.Path(sys.argv[4])
    blobs = []
    for name, every in (("ecb.jsonl", 1), ("scripture.jsonl", 10)):
        k = 0
        for line in open(seed / name, encoding="utf-8"):
            l = json.loads(line)
            if "cell" not in l:
                continue
            k += 1
            if k % every:
                continue
            ns, fn = l["cell"]["fn"].rsplit("/", 1)
            uri = datumesh.format_uri({"ns": ns, "fn": fn, "args": l["cell"]["args"], "asof": l["asof"]})
            paths.append("/cell/" + uri[len("cell://"):])
            names.add(l["cell"]["fn"])
            if "blob" in l["value"] and len(blobs) < 100:
                blobs.append("/blob/" + l["value"]["blob"].rsplit("/", 1)[1])
    paths.extend(blobs)
for n in sorted(names):
    paths.append("/bindings?name=" + n.replace("/", "%2F") + "&asof=2026-06-01T00:00:00.000000Z")
same, differ, ok, statuses = 0, [], 0, {}
for path in paths:
    a, b = get(live, path), get(restored, path)
    statuses[a[0]] = statuses.get(a[0], 0) + 1
    if a == b:
        same += 1
        ok += a[0] == 200
    else:
        differ.append(path)
print(json.dumps({"requests": len(paths), "identical": same, "identical_200": ok, "statuses (live)": statuses, "different": differ}, indent=1))
sys.exit(1 if differ else 0)
