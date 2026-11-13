#!/bin/sh
# Installs the nightly backup (03:15 UTC), the 15-minute resource check and the weekly domain
# re-check (Sundays 04:30 UTC, validate.sh: `datumesh-mesh validate --all`) as root cron jobs
# in /etc/cron.d/datumesh-mesh-test, running copies of the scripts in $DATUMESH_DIR/bin (so a
# checkout of the repository can change without changing what runs). Logs in $DATUMESH_DIR/logs.
set -e
here="$(cd "$(dirname "$0")" && pwd)"
dir="${DATUMESH_DIR:-/root/datumesh-mesh-test}"
mkdir -p "$dir/bin" "$dir/logs"
install -m 0700 "$here/backup.sh" "$here/resource_check.sh" "$here/validate.sh" "$dir/bin/"
cat > /etc/cron.d/datumesh-mesh-test <<CRON
# datumesh-mesh-test (installed by deploy/install_cron.sh)
SHELL=/bin/sh
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
DATUMESH_DIR=$dir
15 3 * * * root $dir/bin/backup.sh >> $dir/logs/backup.log 2>&1
*/15 * * * * root $dir/bin/resource_check.sh >> $dir/logs/resources.log 2>&1
30 4 * * 0 root $dir/bin/validate.sh >> $dir/logs/validate.log 2>&1
CRON
chmod 644 /etc/cron.d/datumesh-mesh-test
echo "installed /etc/cron.d/datumesh-mesh-test"
