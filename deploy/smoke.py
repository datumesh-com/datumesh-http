# SPDX-License-Identifier: Apache-2.0
"""The HTTPS smoke suite (brief test 12) against the test mesh: every endpoint once, envelopes
against the spec's schema, the key and mesh documents, a receipt fetched over HTTPS verified
with the Python library datumesh 0.4.2 (smoke.mjs does the same with @datumesh/verify 0.4.2),
cache headers, pending (a batch of cold computed cells), a rate-limit refusal, /metrics,
/health. Since 0.1.1 (spec-0.4.2): pinned requests (call and URI) and the echo, trust
`computed`, E_NAME_UNBOUND, a slice page and a batch checked with the library, the versions
endpoint, and a scripture verse with its blob (10.1 step 7.7).

    python smoke.py <base URL> <spec dir> <API key file> [CA file]   (JSON results to stdout; exit 1 on a failure)

With the real certificate no CA file is given: the system trust store verifies it, and the
key document is also fetched with datumesh.fetch_keys (TLS verified, no redirects).
"""
import json
import pathlib
import random
import ssl
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

import datumesh
import jsonschema
import referencing

base, spec, keyfile = sys.argv[1].rstrip("/"), pathlib.Path(sys.argv[2]), sys.argv[3]
cafile = sys.argv[4] if len(sys.argv) > 4 else None
host = base.split("//", 1)[1].split("/")[0]
ctx = ssl.create_default_context(cafile=cafile)
KEY = open(keyfile).read().strip()
results = []


def check(name, ok, detail=""):
    results.append({"check": name, "ok": bool(ok), "detail": detail})


def req(path, method="GET", body=None, headers=None, auth=True):
    h = dict(headers or {})
    if auth:
        h["Authorization"] = "Bearer " + KEY
    r = urllib.request.Request(base + path, data=body, method=method, headers=h)
    try:
        with urllib.request.urlopen(r, context=ctx, timeout=60) as resp:
            return resp.status, dict(resp.headers), resp.read()
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers), e.read()


resources = []
for p in (spec / "schemas").glob("*.json"):
    d = json.loads(p.read_text())
    resources.append((d["$id"], referencing.Resource.from_contents(d)))
registry = referencing.Registry().with_resources(resources)
schema = lambda n: json.loads((spec / "schemas" / n).read_text())
env_v = jsonschema.Draft202012Validator(schema("envelope.schema.json"), registry=registry)
mesh_v = jsonschema.Draft202012Validator(schema("mesh.schema.json"), registry=registry)


def envelope_ok(name, body):
    errs = [e.message for e in env_v.iter_errors(json.loads(body))]
    check(name + ": envelope schema", not errs, "; ".join(errs[:3]))


# Health, metrics, OpenAPI.
s, _, b = req("/health")
check("/health", s == 200 and json.loads(b) == {"status": "ok"}, b.decode()[:100])
s, _, b = req("/metrics")
check("/metrics", s == 200 and b"datumesh_pool_size 32" in b, f"status {s}")
s, _, b = req("/openapi.json")
check("/openapi.json", s == 200 and len(json.loads(b)["paths"]) >= 17, f"{len(json.loads(b)['paths'])} paths")

# Key and mesh documents.
s, h, kd = req("/.well-known/datumesh.json")
keys = datumesh.keys_from_response(host, s, kd)
check("key document (SPEC 9.2)", keys.ok, keys.detail if not keys.ok else ",".join(k.id for k in keys.keys))
if cafile is None:
    fetched = datumesh.fetch_keys(host)
    check("key document by datumesh.fetch_keys (TLS verified)", fetched.ok, getattr(fetched, "detail", ""))
s2, _, md = req("/.well-known/datumesh-mesh.json")
check("mesh document schema", not list(mesh_v.iter_errors(json.loads(md))))
mr = datumesh.resolve_mesh(host, (s, kd), (s2, md))
check("mesh document resolves with the key document (SPEC 9.6)", mr.ok, getattr(mr, "detail", ""))

# Bindings, and the function hash of calc.example.net/e (the vectors' abs(a)).
s, _, b = req("/bindings?name=calc.example.net%2Fe&name=data.example.org%2Fdaily")
binds = json.loads(b)["bindings"]
check("/bindings", s == 200 and len(binds) == 2, json.dumps(binds)[:200])
e_hash = binds["calc.example.net/e"]["fn_hash"]

# A stored cell of the vectors, as of its receipt's asof: the vector's receipt, immutable.
daily = "data.example.org/daily?date=2026-03-04&asof=2026-03-04T18:00:00.000000Z"
s, h, b = req("/cell/" + daily)
envelope_ok("/cell stored", b)
env = json.loads(b)
check("/cell stored: ok, primary", s == 200 and env.get("outcome") == "ok" and env.get("trust") == "primary", b.decode()[:120])
check("cache: as of a past instant, immutable", h.get("Cache-Control") == "public, max-age=31536000, immutable", h.get("Cache-Control", ""))
stored_hash = datumesh.receipt_hash(env["receipt"]) if env.get("receipt") else None

# A computed cell, signed by this mesh: verified with the Python library against the key
# document fetched over HTTPS.
a = f"{random.randint(1, 10**9)}.{random.randint(1, 9)}"
cell = {"args": {"a": {"type": "number", "value": "-" + a}}, "fn": "e", "fn_hash": e_hash, "ns": "calc.example.net"}
cid = datumesh.cell_hash(cell)
path = f"/cell/calc.example.net/e?a=-{a}"
for _ in range(30):
    s, h, b = req(path)
    if s != 202:
        break
    time.sleep(1)
envelope_ok("/cell computed", b)
r = datumesh.check_envelope(s, b, [keys], {"cell_id": cid, "mesh": host})
check("/cell computed: envelope bound to the request, trust computed (Python 0.4.2)", r.ok and r.outcome == "ok" and r.trust == "computed", r.detail)
check("/cell computed: fn_hash echoed", json.loads(b).get("fn_hash") == e_hash)
rec = json.loads(b).get("receipt", {})
v = datumesh.verify(json.dumps(rec), [keys])
check("receipt verified at signed (Python 0.4.2)", v.ok and v.level == "signed", v.detail)
check("computed value: abs", rec.get("value") == a, rec.get("value", ""))
check("cache: about now, max-age=10 and Link canonical", h.get("Cache-Control") == "public, max-age=10" and 'rel="canonical"' in h.get("Link", ""), h.get("Link", ""))
canon = h.get("Link", "<>").split(">")[0][1:]
s3, h3, b3 = req(canon[len(base):]) if canon.startswith(base) else (0, {}, b"")
check("the canonical as-of form: the same receipt, immutable", s3 == 200 and json.loads(b3).get("receipt") == rec and "immutable" in h3.get("Cache-Control", ""), canon)
json.dump(rec, open("/tmp/receipt.json", "w"))

# Receipts, descriptors, manifests, blobs.
s, h, b = req("/receipt/" + datumesh.receipt_hash(rec), headers={"Accept-Encoding": "identity"})
check("/receipt", s == 200 and datumesh.receipt_hash(json.loads(b)) == datumesh.receipt_hash(rec) and "immutable" in h.get("Cache-Control", ""))
s, _, b = req("/descriptor/" + e_hash)
check("/descriptor", s == 200 and datumesh.function_hash(json.loads(b)) == e_hash)
m = json.loads((spec / "vectors" / "slice" / "manifest.json").read_text())
entry = m["entry"]
_, sid, mh = entry.split(":")
s, _, b = req(f"/manifest/{sid}/{mh}")
check("/manifest", s == 200 and datumesh.slice_entry(json.loads(b)) == entry, entry)
s, _, b = req("/blob/" + "0" * 64)
check("/blob (none held: 404)", s == 404)

# Pins (spec-0.4.2): the call form name@<fn_hash>(args) and the URI's fn_hash=, the pin echoed.
s, _, b = req("/cell/" + urllib.parse.quote(f"calc.example.net/e@{e_hash}(-{a})", safe="/@"))
r = datumesh.check_envelope(s, b, [keys], {"cell_id": cid, "fn_hash": e_hash, "mesh": host})
check("pinned call: ok, the pin echoed (Python 0.4.2)", r.ok and r.outcome == "ok" and json.loads(b).get("fn_hash") == e_hash, r.detail)
s, _, b = req(f"{path}&fn_hash={e_hash}")
r = datumesh.check_envelope(s, b, [keys], {"cell_id": cid, "fn_hash": e_hash, "mesh": host})
check("pinned URI: ok, the pin echoed", r.ok and r.outcome == "ok", r.detail)
s, _, b = req(f"{path}&fn_hash={'0' * 64}")
check("pinned to an unknown function: not_covered", s == 404 and json.loads(b).get("outcome") == "not_covered", b.decode()[:160])
s, _, b = req(f"/cell/{host}/no_such_function?x=1")
envelope_ok("E_NAME_UNBOUND", b)
check("an unbound name: not_covered with E_NAME_UNBOUND", json.loads(b).get("outcome") == "not_covered" and json.loads(b).get("error") == "E_NAME_UNBOUND", b.decode()[:160])
s, _, b = req("/cell/data.example.org/daily?date=*")
check("a slice at /cell: E_SLICE_AT_CELL", s == 400 and json.loads(b).get("error") == "E_SLICE_AT_CELL", b.decode()[:160])

# Batch (5.7), slice page (5.6), Arrow, versions (9.8), explain, find, verify.
# The batch and the page use cells this mesh signed (the seed's), whose key document is this
# host's; the vectors' cells are signed by their hosts' test keys, not served here.
fx_bind = json.loads(req("/bindings?name=" + urllib.parse.quote(f"{host}/ecb/fx_rate", safe=""))[2])["bindings"][f"{host}/ecb/fx_rate"]["fn_hash"]
fx_args = {"base": {"type": "string", "value": "EUR"}, "date": {"type": "date", "value": "2026-10-30"}, "quote": {"type": "string", "value": "USD"}}
fx_id = datumesh.cell_hash({"args": fx_args, "fn": "fx_rate", "fn_hash": fx_bind, "ns": f"{host}/ecb"})
fx_uri = datumesh.format_uri({"ns": f"{host}/ecb", "fn": "fx_rate", "args": fx_args})[len("cell://"):]
s, _, b = req("/cells", "POST", json.dumps([fx_uri, path[len("/cell/"):], "calc.example.net/e(-" + a + ")"]).encode(), {"Content-Type": "application/json"})
br = datumesh.check_batch(s, b, [keys], [{"cell_id": fx_id, "mesh": host}, {"cell_id": cid, "mesh": host}, {"cell_id": cid, "mesh": host}])
items = json.loads(b) if s == 200 else []
check("/cells: [{envelope, status}], each bound to its request (Python 0.4.2)", br.ok and all(i.ok for i in br.items) and [i["envelope"].get("outcome") for i in items] == ["ok", "ok", "ok"], br.detail + " " + "; ".join(i.detail for i in br.items or [] if not i.ok)[:300])
for i in items:
    envelope_ok("/cells item", json.dumps(i["envelope"]).encode())
s, _, b = req(f"/slice/{host}/ecb/fx_rate?base=EUR&date=*&quote=USD")
page = json.loads(b) if s == 200 else {}
envelope_ok("/slice page", b)
pr = datumesh.check_envelope(s, b, [keys], {"slice_id": datumesh.slice_id(page["slice"]) if "slice" in page else "0" * 64, "mesh": host})
check("/slice JSON: a page envelope checked (Python 0.4.2)", pr.ok and len(page.get("members", [])) == 30, f"{len(page.get('members', []))} members; {pr.detail}")
s, _, b = req("/versions/data.example.org/daily?date=2026-03-04")
vs = json.loads(b) if s == 200 else {}
check("/versions (9.8)", s == 200 and len(vs.get("versions", [])) >= 1, b.decode()[:200])
s, h, b = req("/slice/data.example.org/daily?date=*&asof=2026-03-10T00:00:00.000000Z", headers={"Accept": "application/vnd.apache.arrow.stream"})
check("/slice Arrow", s == 200 and h.get("Content-Type") == "application/vnd.apache.arrow.stream" and b[:4] == b"\xff\xff\xff\xff", f"{len(b)} bytes")
s, _, b = req("/explain/" + path[len("/cell/"):])
check("/explain", s == 200 and "envelope" in json.loads(b))
s, _, b = req("/find?q=daily")
check("/find", s == 200 and len(json.loads(b)["results"]) > 0)
s, _, b = req("/verify", "POST", json.dumps(rec).encode(), {"Content-Type": "application/json"})
vr = json.loads(b)
check("/verify", s == 200 and vr.get("ok") and vr.get("level") == "recomputed", b.decode()[:200])

# Pending: a batch of cold computed cells takes longer than the 1 s budget; the cells not
# answered by then are jobs, polled to their envelope (bound to the original request).
seed = random.randint(1, 10**9)
calls = [f"calc.example.net/e?a={seed}.{k}" for k in range(1, 600)]
s, _, b = req("/cells", "POST", json.dumps(calls).encode(), {"Content-Type": "application/json"})
items = [i["envelope"] for i in json.loads(b)] if s == 200 else []
pend = [i for i in items if i.get("outcome") == "pending"]
check("pending: a batch past the budget gives jobs", len(pend) > 0, f"{len(pend)} of {len(items)} pending")
if pend:
    p = pend[-1]
    envelope_ok("pending", json.dumps(p).encode())
    job = p["job"]
    check("pending: poll on this host", p["poll"] == f"https://{host}/jobs/{job}")
    for _ in range(120):
        s, _, b = req(f"/jobs/{job}")
        if s != 202:
            break
        time.sleep(0.5)
    r = datumesh.check_envelope(s, b, [keys], {"cell_id": p["request"]["cell_id"], "job": job})
    check("pending: the job's envelope, bound to the original request", r.ok and r.outcome == "ok", r.detail)

# A scripture verse with its blob: the envelope, the receipt verified with its content (7.7).
verse = f"/cell/{host}/scripture/verse_text?book=Gen&chapter=1&corpus=tanakh&edition=uxlc&verse=1"
s, _, b = req(verse)
envelope_ok("scripture verse", b)
ve = json.loads(b)
vrec = ve.get("receipt", {})
blob_ref = vrec.get("value", "")
s2, h2, blob = req("/blob/" + blob_ref[:64])
vv = datumesh.verify(json.dumps(vrec), [keys], None, blob)
check("scripture verse (Genesis 1:1, UXLC 2.6): primary, a blob; verified with its content (Python 0.4.2)",
      ve.get("trust") == "primary" and vrec.get("type") == "blob" and s2 == 200 and vv.ok and vv.level == "signed" and vv.blob_checked,
      f"{blob_ref}; {vv.code} {vv.detail}; {vrec.get('licence')}")
json.dump({"path": verse, "receipt": vrec}, open("/tmp/verse.json", "w"))

# A rate-limit refusal (anonymous: 60 a minute, burst 120), last.
refused = None
for k in range(200):
    s, h, b = req("/descriptor/" + e_hash, auth=False)
    if s == 429:
        refused = (k, h, b)
        break
if refused:
    k, h, b = refused
    envelope_ok("rate_limited", b)
    check("rate limit refuses (429, Retry-After)", json.loads(b).get("outcome") == "rate_limited" and "Retry-After" in h, f"after {k} requests, retry after {h.get('Retry-After')} s")
else:
    check("rate limit refuses", False, "no 429 in 200 requests")

failed = [r for r in results if not r["ok"]]
print(json.dumps({"base": base, "checks": len(results), "failed": len(failed), "results": results, "stored_receipt": stored_hash}, indent=1))
sys.exit(1 if failed else 0)
