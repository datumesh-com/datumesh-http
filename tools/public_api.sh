#!/bin/sh
# The library's public API (`cargo public-api -ss`, which needs a nightly toolchain and
# cargo-public-api 0.52), compared with public-api.txt, or written with --write. Plain
# output whatever CARGO_TERM_COLOR says (CI sets it to always): --color=never.
#
#   tools/public_api.sh [--write]            in the datumesh-nightly-tools image (tools/cargo.sh)
#   tools/public_api.sh --native [--write]   with the local cargo (CI: nightly-2026-10-22)
set -e
root="$(cd "$(dirname "$0")/.." && pwd)"
native=false
write=false
for a in "$@"; do
  case "$a" in --native) native=true ;; --write) write=true ;; *) echo "unknown option $a" >&2; exit 2 ;; esac
done
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
if $native; then
  (cd "$root" && cargo public-api -ss --color=never) > "$work/api.txt"
else
  "$root/tools/cargo.sh" nightly-tools public-api -ss --color=never > "$work/api.txt"
fi
if $write; then
  cp "$work/api.txt" "$root/public-api.txt"
  echo "public-api.txt written"
else
  diff -u "$root/public-api.txt" "$work/api.txt"
  echo "public-api.txt is current"
fi
