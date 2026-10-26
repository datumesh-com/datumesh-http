#!/bin/sh
# Test 6's cache in front: nginx (proxy_cache, honouring the origin's Cache-Control) in a
# container named datumesh-http-nginx, in front of the test mesh, and the test
# `caching::cache_in_front`, which asserts by the origin's request counter that repeat gets
# are served by the cache. The containers are removed afterwards. Nothing listens beyond
# the Docker network (or 127.0.0.1 with --native).
#
#   tools/cache_test.sh [toolchain]   cargo in rust:<toolchain> (default 1.99) on the datumesh network
#   tools/cache_test.sh --native      local cargo; nginx on the host network, on 127.0.0.1
set -e
root="$(cd "$(dirname "$0")/.." && pwd)"
conf="$(mktemp -d)"
trap 'docker rm -f datumesh-http-nginx >/dev/null 2>&1 || true; rm -rf "$conf"' EXIT
if [ "$1" = "--native" ]; then
  origin="127.0.0.1:18080"; listen="127.0.0.1:18081"; resolver=""; net="--network host"
else
  origin="datumesh-http-cargo-cache:18080"; listen="8080"; resolver="resolver 127.0.0.11 valid=5s;"; net="--network datumesh"
fi
cat > "$conf/default.conf" <<CONF
proxy_cache_path /tmp/dmcache keys_zone=dm:10m max_size=100m inactive=1h use_temp_path=off;
server {
  listen $listen;
  $resolver
  location / {
    set \$origin "$origin";
    proxy_pass http://\$origin;
    proxy_http_version 1.1;
    proxy_cache dm;
    proxy_cache_key \$request_uri;
    proxy_cache_lock on;
    add_header X-Cache-Status \$upstream_cache_status always;
  }
}
CONF
docker rm -f datumesh-http-nginx >/dev/null 2>&1 || true
docker run -d --name datumesh-http-nginx $net -v "$conf/default.conf:/etc/nginx/conf.d/default.conf:ro" nginx:1.27-alpine >/dev/null
if [ "$1" = "--native" ]; then
  (cd "$root" && DATUMESH_HTTP_CACHE_URL="http://127.0.0.1:18081" DATUMESH_HTTP_ORIGIN_LISTEN="127.0.0.1:18080" cargo test --locked --test caching -- --nocapture cache_in_front)
else
  tag="${1:-1.99}"
  docker run --rm -i --name datumesh-http-cargo-cache --network datumesh \
    -v "$HOME/.cache/datumesh-cargo:/usr/local/cargo/registry" -v "$root:/w" -w /w \
    -e CARGO_TARGET_DIR="/w/target-$tag" -e CARGO_TERM_COLOR=never \
    -e DATABASE_URL="${DATABASE_URL:-postgres://postgres:postgres@datumesh-pg:5432/postgres}" \
    -e DATUMESH_HTTP_CACHE_URL="http://datumesh-http-nginx:8080" -e DATUMESH_HTTP_ORIGIN_LISTEN="0.0.0.0:18080" \
    "rust:$tag" cargo test --locked --test caching -- --nocapture cache_in_front
fi
