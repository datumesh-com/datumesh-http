#!/bin/sh
# Installs the key-document check (source_keydocs.py, P3) on vmi630775, as root, outside quran-box: copies the
# script to /root/datumesh-prod/bin/ (0755) and writes /etc/cron.d/datumesh-prod-keydocs, one line, daily at 05:17 UTC:
#   17 5 * * * root /usr/bin/python3 -I /root/datumesh-prod/bin/source_keydocs.py >> /root/datumesh-prod/source-keydocs.log 2>&1
# Its status goes to /root/datumesh-prod/status (source-keydocs.json, .txt, .state). Nothing on the mesh changes: the
# script only reads. A change to the repository changes nothing until this is run again.
set -e
here="$(cd "$(dirname "$0")" && pwd)"
dir=/root/datumesh-prod
mkdir -p "$dir/bin" "$dir/status"
install -m 0755 "$here/source_keydocs.py" "$dir/bin/source_keydocs.py"
printf '%s\n' "17 5 * * * root /usr/bin/python3 -I $dir/bin/source_keydocs.py >> $dir/source-keydocs.log 2>&1" > /etc/cron.d/datumesh-prod-keydocs.new
chmod 0644 /etc/cron.d/datumesh-prod-keydocs.new
mv /etc/cron.d/datumesh-prod-keydocs.new /etc/cron.d/datumesh-prod-keydocs
cat /etc/cron.d/datumesh-prod-keydocs
