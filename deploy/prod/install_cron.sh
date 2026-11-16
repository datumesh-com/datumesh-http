#!/bin/sh
# Installs the production jobs in the datumesh user's crontab on quran-box (it has no root and
# needs none: the jobs use docker, which the user may run). Copies of the scripts run from
# /srv/datumesh/bin, so a change to the repository changes nothing until installed again.
#   03:15 UTC  backup.sh       pg_dump, the 14 newest kept, the newest copied to vmi630775
#   */5        check.sh        the error alert (new 5xx or error envelopes) and resources
# The uptime alert runs from outside, on vmi630775 (uptime.sh, root's crontab there).
set -e
here="$(cd "$(dirname "$0")" && pwd)"
dir=/srv/datumesh
mkdir -p "$dir/bin" "$dir/logs"
install -m 0700 "$here/backup.sh" "$here/check.sh" "$here/restore_drill.sh" "$dir/bin/"
install -m 0644 "$here/drill_check.py" "$dir/bin/"
crontab -l 2>/dev/null | grep -v '# datumesh-prod$' > "$dir/bin/crontab.new" || true
cat >> "$dir/bin/crontab.new" <<CRON
15 3 * * * $dir/bin/backup.sh >> $dir/logs/backup.log 2>&1 # datumesh-prod
*/5 * * * * $dir/bin/check.sh >> $dir/logs/check.log 2>&1 # datumesh-prod
CRON
crontab "$dir/bin/crontab.new"
rm -f "$dir/bin/crontab.new"
crontab -l
