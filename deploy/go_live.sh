#!/bin/sh
# Switches Caddy from its internal CA to Let's Encrypt once mesh-test.datumesh.com resolves to
# this VM: checks the A record by DNS-over-HTTPS (Cloudflare and Google), restarts Caddy with
# deploy/Caddyfile, waits for the certificate and prints its issuer and expiry. Refuses while
# the name does not resolve to the VM's address (no ACME attempt is made then).
set -e
here="$(cd "$(dirname "$0")" && pwd)"
want="${DATUMESH_PUBLIC_IP:-207.180.222.183}"
for doh in "https://cloudflare-dns.com/dns-query" "https://dns.google/resolve"; do
  got="$(curl -s -H 'accept: application/dns-json' "$doh?name=mesh-test.datumesh.com&type=A" | python3 -c 'import json,sys; print(" ".join(a["data"] for a in json.load(sys.stdin).get("Answer", []) if a.get("type") == 1))')"
  echo "$doh: ${got:-no A record}"
  [ "$got" = "$want" ] || { echo "mesh-test.datumesh.com does not resolve to $want yet" >&2; exit 2; }
done
docker compose -p datumesh-mesh-test -f "$here/compose.yml" up -d caddy
for i in $(seq 1 60); do
  cert="$(echo | openssl s_client -connect 127.0.0.1:443 -servername mesh-test.datumesh.com 2>/dev/null | openssl x509 -noout -issuer -enddate 2>/dev/null || true)"
  case "$cert" in *"Let's Encrypt"*|*"O = Let's Encrypt"*) echo "$cert"; exit 0 ;; esac
  sleep 5
done
echo "no Let's Encrypt certificate after 5 minutes; caddy's log:" >&2
docker logs --tail 30 datumesh-mesh-test-caddy-1 >&2
exit 1
