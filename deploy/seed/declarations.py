# SPDX-License-Identifier: Apache-2.0
"""The declarations of the test mesh's key document (SPEC 9.2, spec-0.4.3; http 0.2.1): which
restricted licences the mesh permits as inputs of its own functions (licence inheritance, SPEC
8.5). Exactly two, each open-ended from the bound instant of its function's seed:

* <host>/ecb/fx_cross over the restricted test licence https://<host>/licences/restricted-test
  (the EUR/NOK cell of seed/ecb.py); its twin fx_cross_undeclared is not declared;
* <host>/scripture/strip_niqqud over the UXLC licence https://tanach.us/License.html (the
  tanakh/uxlc verses of seed/scripture.py).

    python -I declarations.py <host> [--args]

Prints one JSON line per declaration: {"fn", "licence", "valid_from"}; with --args, the same
as `<fn> <licence> <valid_from>` lines, the arguments of `datumesh-mesh declare`
(deploy/licences.sh). Entries are never removed, only closed.
"""
import json
import sys

host = sys.argv[1]
ECB_BOUND = "2026-09-01T00:00:00.000000Z"  # seed/ecb.py BOUND
SCRIPTURE_TX = "2026-11-02T00:00:00.000000Z"  # seed/scripture.py TX
for fn, licence, valid_from in (
    (f"{host}/ecb/fx_cross", f"https://{host}/licences/restricted-test", ECB_BOUND),
    (f"{host}/scripture/strip_niqqud", "https://tanach.us/License.html", SCRIPTURE_TX),
):
    if "--args" in sys.argv[2:]:
        print(fn, licence, valid_from)
    else:
        print(json.dumps({"fn": fn, "licence": licence, "valid_from": valid_from}, sort_keys=True))
