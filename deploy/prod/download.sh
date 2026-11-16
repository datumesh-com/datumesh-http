#!/bin/sh
# The fresh download for the production load: the four scripture sources, checked against
# seed/SOURCES by seed/download.sh (a mismatch stops), and tanach.us/License.html, which has no
# checksum (it is the notice UXLC's zip does not carry; its text is quoted in the edition cell).
#   deploy/prod/download.sh <new empty dir>
set -e
here="$(cd "$(dirname "$0")" && pwd)"
dir="$1"
[ -n "$dir" ] || { echo "usage: download.sh <dir>" >&2; exit 2; }
"$here/../seed/download.sh" "$dir"
curl -fsS --proto '=https' --max-time 60 -o "$dir/uxlc-license.html" https://tanach.us/License.html
echo "ok uxlc-license.html $(sha256sum "$dir/uxlc-license.html" | cut -d' ' -f1) $(wc -c < "$dir/uxlc-license.html") bytes (not pinned)"
