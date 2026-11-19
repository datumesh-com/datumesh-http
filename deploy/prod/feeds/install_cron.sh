#!/bin/sh
# Print the crontab lines for the feed jobs. It installs nothing: review the lines, then paste them into the
# datumesh user's crontab (crontab -e -u datumesh). The host is required, with no default.
#
# usage: install_cron.sh <host>     host: mesh-test.datumesh.com or datumesh.com
set -eu

host=${1:-}
case "$host" in
  mesh-test.datumesh.com|datumesh.com) ;;
  *) echo "install_cron.sh: usage: install_cron.sh mesh-test.datumesh.com|datumesh.com" >&2; exit 2 ;;
esac

here=$(cd "$(dirname "$0")" && pwd)
runner="$here/run_feed.sh"

cat <<EOF
# datumesh feed jobs (Build 2 Part 5). Off-peak minutes, server time. Output goes to /srv/datumesh/logs/.
# These publish under the names of $host (names.toml); state and output are kept per host.
11 2 * * *  FEEDS_HOST=$host $runner ecb_fx
23 2 * * *  FEEDS_HOST=$host $runner estr
37 2 * * *  FEEDS_HOST=$host $runner policy_rates
41 3 * * *  FEEDS_HOST=$host $runner treasury
53 3 * * *  FEEDS_HOST=$host $runner boe
17 4 3 * *  FEEDS_HOST=$host $runner chirps
EOF
