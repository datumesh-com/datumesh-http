#!/bin/sh
# Download the scripture sources listed in SOURCES into <dir> (a new directory, outside the
# repository) and check each file's SHA-256 against the list; exit 1 on a mismatch. The files
# are untrusted data: nothing here runs them.
#   deploy/seed/download.sh <dir>
set -e
here="$(cd "$(dirname "$0")" && pwd)"
dir="$1"
[ -n "$dir" ] || { echo "usage: download.sh <dir>" >&2; exit 2; }
mkdir -p "$dir"
grep -v '^#' "$here/SOURCES" | while read -r sum file url; do
  curl -fsS --proto '=https' --max-time 300 -o "$dir/$file" "$url"
  got="$(sha256sum "$dir/$file" | cut -d' ' -f1)"
  if [ "$got" = "$sum" ]; then
    echo "ok $file $got $(wc -c < "$dir/$file") bytes"
  else
    echo "MISMATCH $file: got $got, want $sum" >&2
    exit 1
  fi
done
