#!/bin/sh
# Checks the Cloudflare IP list in site-block.caddy against Cloudflare's published list
# (https://api.cloudflare.com/client/v4/ips, the same as https://www.cloudflare.com/ips/).
# Exit 0 when they are equal; otherwise prints the new `@notcf` line and exits 1. Run monthly
# (RUNBOOK.md). Changing the block means changing khatm's source on the operator's machine.
set -e
here="$(cd "$(dirname "$0")" && pwd)"
new="$(curl -fsS --max-time 30 https://api.cloudflare.com/client/v4/ips | python3 -I -c '
import json, sys
d = json.load(sys.stdin)["result"]
print(d["etag"], " ".join(d["ipv4_cidrs"] + d["ipv6_cidrs"]))')"
etag="${new%% *}"; list="${new#* }"
have="$(sed -n 's/^\t@notcf not remote_ip //p' "$here/site-block.caddy")"
if [ "$have" = "$list" ]; then
  echo "site-block.caddy is current (etag $etag)"
else
  echo "Cloudflare's list changed (etag $etag). New line for site-block.caddy and khatm's Caddyfile:"
  printf '\t@notcf not remote_ip %s\n' "$list"
  exit 1
fi
