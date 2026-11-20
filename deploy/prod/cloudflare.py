# SPDX-License-Identifier: Apache-2.0
"""Cloudflare for the production mesh (step 1.7a, fact A as accepted): Worker routes with no script
for the mesh's public paths, so those requests go to the origin (quran-box, through khatm's Caddy)
and everything else stays on the site's Worker; and the apex A record.

    python3 -I cloudflare.py plan              what would change (default; changes nothing)
    python3 -I cloudflare.py routes            add the no-script routes
    python3 -I cloudflare.py dns <ipv4>        point the proxied apex A record at <ipv4>
    python3 -I cloudflare.py undo-routes       remove exactly the routes this script adds

The token is read from CF_API_TOKEN or /root/cf-key and never printed. Every route whose path can
carry a query string has a trailing `*` (the user's rule, /cells included). /health and /metrics
get no route: the public sees the site's 404 for them.
"""
import json
import os
import pathlib
import sys
import urllib.request

ZONE = "datumesh.com"
API = "https://api.cloudflare.com/client/v4"
PATHS = [
    "/cell/*", "/cells*", "/slice/*", "/receipt/*", "/descriptor/*", "/manifest/*", "/versions/*", "/blob/*",
    "/explain/*", "/find*", "/verify*", "/jobs/*", "/bindings*", "/describe/*", "/openapi.json*",
    "/.well-known/datumesh.json*", "/.well-known/datumesh-mesh.json*", "/.well-known/acme-challenge/*",
    # Build 2 (http 0.4.x): function pages, takeover pages, scoped evaluation, signed bundles.
    "/fn/*", "/takeover/*", "/whatif*", "/bundles*",
]
ROUTES = [ZONE + p for p in PATHS]


def token():
    return (os.environ.get("CF_API_TOKEN") or pathlib.Path("/root/cf-key").read_text()).strip()


def call(method, path, body=None):
    req = urllib.request.Request(API + path, method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"Authorization": "Bearer " + token(), "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        d = json.load(r)
    if not d.get("success"):
        sys.exit(f"{method} {path}: {d.get('errors')}")
    return d["result"]


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "plan"
    zone = call("GET", f"/zones?name={ZONE}")[0]
    zid = zone["id"]
    routes = {r["pattern"]: r for r in call("GET", f"/zones/{zid}/workers/routes")}
    apex = [r for r in call("GET", f"/zones/{zid}/dns_records?type=A&name={ZONE}")]
    if cmd == "plan":
        print(f"zone {ZONE}: plan {zone['plan']['name']}, ssl {call('GET', f'/zones/{zid}/settings/ssl')['value']}")
        for r in apex:
            print(f"A {r['name']} {r['content']} proxied={r['proxied']}")
        for p in ROUTES:
            have = routes.get(p)
            print(f"route {p}: " + ("exists, script " + str(have.get('script')) if have else "to add (no script)"))
        for p, r in routes.items():
            if p not in ROUTES:
                print(f"route {p}: kept, script {r.get('script')}")
    elif cmd == "routes":
        for p in ROUTES:
            if p in routes:
                if routes[p].get("script"):
                    sys.exit(f"{p} exists with script {routes[p]['script']}: not changed")
                continue
            call("POST", f"/zones/{zid}/workers/routes", {"pattern": p})
            print(f"added {p} (no script)")
    elif cmd == "undo-routes":
        for p in ROUTES:
            if p in routes and not routes[p].get("script"):
                call("DELETE", f"/zones/{zid}/workers/routes/{routes[p]['id']}")
                print(f"removed {p}")
    elif cmd == "dns":
        ip = sys.argv[2]
        if len(apex) != 1:
            sys.exit(f"expected one apex A record, found {len(apex)}")
        r = apex[0]
        call("PATCH", f"/zones/{zid}/dns_records/{r['id']}", {"content": ip, "proxied": True})
        print(f"A {ZONE}: {r['content']} -> {ip}, proxied")
    else:
        sys.exit(__doc__)


main()
