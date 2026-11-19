#!/bin/sh
# Print the crontab lines for the feed jobs. It installs nothing: review the lines, then paste them into the
# datumesh user's crontab (crontab -e -u datumesh) once the relay names are answered (see README.md).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
runner="$here/run_feed.sh"

cat <<EOF
# datumesh feed jobs (Build 2 Part 5). Off-peak minutes, server time. Output goes to /srv/datumesh/logs/.
# Rehearsal: these publish under the names in names.toml (mesh-test) until the relay names are answered.
11 2 * * *  $runner ecb_fx
23 2 * * *  $runner estr
37 2 * * *  $runner policy_rates
41 3 * * *  $runner treasury
53 3 * * *  $runner boe
17 4 3 * *  $runner chirps
EOF
