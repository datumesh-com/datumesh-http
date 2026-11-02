# SPDX-License-Identifier: Apache-2.0
"""Check every case of a JSON lines file (written by tests/binding.rs) with the Python library
datumesh: the response the real endpoint served, against the request the case says was made
(kind "envelope" and "page": check_envelope; kind "batch": check_batch, the first failing
item's code). Prints a summary; exits 1 when a result is not the case's `expect` ("ok" or an
error code)."""
import json
import sys

import datumesh

total = bad = 0
mismatches = 0
for line in open(sys.argv[1]):
    c = json.loads(line)
    keys = [datumesh.keys_from_response(h, e["https"]["status"], e["https"].get("body")) for h, e in sorted(c["keys"].items())]
    if c["kind"] == "batch":
        b = datumesh.check_batch(c["status"], c["input"], keys, c["requests"])
        if not b.ok:
            got = b.code
        else:
            got = next((i.code for i in b.items if not i.ok), "ok")
        detail = b.detail
    else:
        r = datumesh.check_envelope(c["status"], c["input"], keys, c["request"])
        got = "ok" if r.ok else r.code
        detail = r.detail
    total += 1
    if got != "ok":
        mismatches += 1
    if got != c["expect"]:
        bad += 1
        print(f"{c['id']}: got {got}, want {c['expect']}: {detail}")
print(f"python datumesh {datumesh.__version__}: {total} cases, {mismatches} refused, {bad} unexpected")
sys.exit(1 if bad else 0)
