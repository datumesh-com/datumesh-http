"""List the packages in Cargo.lock (from crates.io) whose locked version was published less
than DAYS days before CUTOFF (default: 14 days before today), with the newest older version.

    python3 tools/young_deps.py [Cargo.lock] [days]
"""
import datetime
import json
import sys
import time
import urllib.request

lock = sys.argv[1] if len(sys.argv) > 1 else "Cargo.lock"
days = int(sys.argv[2]) if len(sys.argv) > 2 else 14
cutoff = (datetime.datetime.utcnow() - datetime.timedelta(days=days)).strftime("%Y-%m-%dT%H:%M:%S")
pkgs, cur = [], {}
for line in open(lock):
    line = line.strip()
    if line == "[[package]]":
        if cur:
            pkgs.append(cur)
        cur = {}
    elif " = " in line:
        k, v = line.split(" = ", 1)
        cur[k] = v.strip('"')
if cur:
    pkgs.append(cur)
for p in pkgs:
    if "registry+" not in p.get("source", ""):
        continue
    req = urllib.request.Request(f"https://crates.io/api/v1/crates/{p['name']}/versions",
                                 headers={"User-Agent": "datumesh-http dependency age check (github.com/datumesh-com/datumesh-http)"})
    for _ in range(3):
        try:
            data = json.load(urllib.request.urlopen(req, timeout=30))
            break
        except Exception:
            time.sleep(5)
    else:
        print(f"{p['name']} {p['version']}: lookup failed")
        continue
    vs = {v["num"]: v for v in data["versions"]}
    v = vs.get(p["version"])
    if v and v["created_at"] > cutoff:
        older = [x["num"] for x in data["versions"] if x["created_at"] <= cutoff and not x["yanked"] and "-" not in x["num"]]
        print(f"{p['name']} {p['version']} published {v['created_at'][:10]}; older: {older[:3]}")
    time.sleep(1)
print("checked", sum(1 for p in pkgs if "registry+" in p.get("source", "")), "packages; cutoff", cutoff)
