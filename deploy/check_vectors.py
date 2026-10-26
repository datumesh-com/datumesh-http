# SPDX-License-Identifier: Apache-2.0
"""Which of the spec-0.4.1 vectors' cells the test mesh answers, and whether it answers them
as the vectors say. Uses the Python library datumesh 0.4.1 for cell URIs and hashes.

    python check_vectors.py <vectors dir> <base URL> [CA file] <API key file>   (JSON summary to stdout)

- receipt vectors (valid, with a descriptor): the cell as of the receipt's asof must be `ok`
  with that very receipt (same receipt hash) for a stored function; a computed one is
  evaluated by the mesh (its own receipt), counted when the value is the vector's;
- formula-eval and excel vectors, when the mesh binds every function of the vector's world as
  the vector does: the cell as of the vector's asof must be `ok` with the vector's value
  (valid vectors), `not_covered` with the fault (an evaluation fault), `not_covered`
  (E_FORMULA_NOT_COVERED) or `error` E_EVAL_UNAVAILABLE (every other code).
"""
import json
import pathlib
import ssl
import sys
import urllib.error
import urllib.parse
import urllib.request

import datumesh

vectors, base = pathlib.Path(sys.argv[1]), sys.argv[2].rstrip("/")
# Arguments: vectors, base URL, then optionally a CA file and an API key file.
rest = sys.argv[3:]
cafile = rest[0] if len(rest) == 2 else None
ctx = ssl.create_default_context(cafile=cafile)
FAULTS = set(datumesh.FAULT_CODES)


def get(path):
    req = urllib.request.Request(base + path, headers={"Authorization": "Bearer " + KEY} if KEY else {})
    try:
        with urllib.request.urlopen(req, context=ctx, timeout=30) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


KEY = open(rest[-1]).read().strip() if rest else None
bound_cache = {}


def bound(name, asof):
    k = (name, asof)
    if k not in bound_cache:
        q = urllib.parse.urlencode({"name": name, "asof": asof}, safe=":").replace("%2F", "%2F")
        _, b = get("/bindings?" + q)
        bound_cache[k] = b.get("bindings", {}).get(name, {}).get("fn_hash")
    return bound_cache[k]


res = {"receipt": {}, "formula": {}, "world receipts": {}}
seen_world = set()
details = []


def count(group, what):
    res[group][what] = res[group].get(what, 0) + 1


for p in sorted(vectors.glob("*/*.json")):
    v = json.loads(p.read_text())
    if v["kind"] == "receipt" and v["valid"]:
        r = json.loads(v["input"])
        d = v.get("recompute", {}).get("descriptor")
        if not isinstance(d, dict):
            count("receipt", "no descriptor in the vector: not answerable by name")
            continue
        c = r["cell"]
        name = c["ns"] + "/" + c["fn"]
        if bound(name, r["asof"]) != c["fn_hash"]:
            count("receipt", "name bound to another function")
            continue
        uri = datumesh.format_uri({"ns": c["ns"], "fn": c["fn"], "args": c["args"], "asof": r["asof"]})
        status, env = get("/cell/" + uri[len("cell://"):])
        if d["code"]["lang"] == "stored":
            ok = env.get("outcome") == "ok" and datumesh.receipt_hash(env["receipt"]) == v["sha256"]
            count("receipt", "stored: the vector's receipt" if ok else "stored: other answer")
        else:
            got = env.get("receipt", {})
            ok = env.get("outcome") == "ok" and got.get("value") == r["value"] and got.get("type") == r["type"]
            count("receipt", "computed: the vector's value" if ok else f"computed: {env.get('outcome')} {env.get('error', env.get('fault', ''))}".strip())
        if not ok:
            details.append({"id": v["id"], "outcome": env.get("outcome"), "error": env.get("error")})
    elif v["kind"] in ("formula-eval", "excel"):
        # The world's stored receipts: cells of stored functions, answerable by name.
        for r in v["world"].get("receipts", []):
            c = r["cell"]
            h = datumesh.receipt_hash(r)
            if h in seen_world:
                continue
            seen_world.add(h)
            if bound(c["ns"] + "/" + c["fn"], r["asof"]) != c["fn_hash"]:
                count("world receipts", "name bound to another function")
                continue
            uri = datumesh.format_uri({"ns": c["ns"], "fn": c["fn"], "args": c["args"], "asof": r["asof"]})
            _, env = get("/cell/" + uri[len("cell://"):])
            if env.get("outcome") == "ok" and datumesh.receipt_hash(env["receipt"]) == h:
                count("world receipts", "the receipt")
            elif env.get("outcome") == "ok" and datumesh.cell_hash(env["receipt"]["cell"]) == datumesh.cell_hash(c):
                count("world receipts", "another version of the cell (one answering then)")
            else:
                count("world receipts", "otherwise: " + str(env.get("outcome")))
        fs = dict(v["world"]["functions"])
        if v["kind"] == "excel":
            # SPEC 11: an excel vector's function is its printed body under its signature.
            fs[v["cell"]["fn"]] = {"descriptor": {"code": {"body": v["body"], "lang": "formula"}, "inputs": [], "signature": v["signature"]}}
        if any(bound(n, v["asof"]) != datumesh.function_hash(f["descriptor"]) for n, f in fs.items()):
            count("formula", "a function of its world is bound otherwise")
            continue
        ns, fn = v["cell"]["fn"].rsplit("/", 1)
        uri = datumesh.format_uri({"ns": ns, "fn": fn, "args": v["cell"]["args"], "asof": v["asof"]})
        status, env = get("/cell/" + uri[len("cell://"):])
        o = env.get("outcome")
        if v["valid"]:
            got = env.get("receipt", {})
            ok = o == "ok" and got.get("value") == v["value"]["value"] and got.get("type") == v["value"]["type"]
        elif v["error"] in FAULTS:
            ok = o == "not_covered" and env.get("fault") == v["error"]
        elif v["error"] == "E_FORMULA_NOT_COVERED":
            ok = o == "not_covered" and "fault" not in env
        else:
            ok = o == "error" and env.get("error") == "E_EVAL_UNAVAILABLE"
        count("formula", ("valid" if v["valid"] else "invalid") + (": as the vector says" if ok else ": otherwise"))
        if not ok:
            details.append({"id": v["id"], "outcome": o, "error": env.get("error"), "fault": env.get("fault"), "want": v.get("error", v.get("value"))})

print(json.dumps({"summary": res, "otherwise": details}, indent=1))
