#!/bin/sh
# Run one feed job: fetch and check in the pinned python image, write one publish-stored file to
# /srv/datumesh/feeds/out/, publish it with datumesh-mesh, then keep the new state. Logs to /srv/datumesh/logs/.
#
# usage: FEEDS_HOST=<host> run_feed.sh <module> [history]
#   FEEDS_HOST: mesh-test.datumesh.com (rehearsal) or datumesh.com (production). There is no default: the names
#   a job publishes are spelled for that host, and state and output are kept per host.
#   module: ecb_fx estr policy_rates treasury boe chirps   (register runs by hand, see README.md)
#   history: ecb_fx treasury: ecb_fx backfills from eurofxref-hist.zip; treasury backfills from 1990, a year a file,
#     and this script goes on year by year until there is nothing new (a failure loses at most the year it was in)
#
# Reads /srv/datumesh/.env (PYTHON_DIGEST, DATUMESH_PUBLISH_LANES) and /srv/datumesh/compose.yml.
# The fetch step uses the default bridge network (it needs the internet, not the datumesh_backend network).
# The job works on a copy of its state: the state moves in place only after publish-stored succeeds.
set -eu

module=${1:?usage: FEEDS_HOST=<host> run_feed.sh <module> [history]}
mode=${2:-daily}
host=${FEEDS_HOST:-}
case "$host" in
  mesh-test.datumesh.com|datumesh.com) ;;
  *) echo "run_feed.sh: set FEEDS_HOST to mesh-test.datumesh.com or datumesh.com (got '$host')" >&2; exit 2 ;;
esac
case "$module" in
  ecb_fx|estr|policy_rates|treasury|boe|chirps) ;;
  *) echo "run_feed.sh: unknown module $module" >&2; exit 2 ;;
esac

here=$(cd "$(dirname "$0")" && pwd)
root=${DATUMESH_ROOT:-/srv/datumesh}
feeds="$root/feeds"
out="$feeds/out/$host"
state="$feeds/state/$host"
work="$feeds/work/$host/$module"
log="$root/logs/feeds-$module.log"

mkdir -p "$out" "$state" "$root/logs"
exec 9>"$feeds/$host.$module.lock"
if ! flock -n 9; then
  echo "$(date -u +%FT%TZ) $host $module: the previous run is still going" >>"$log"
  exit 0
fi

fail() {
  echo "$(date -u +%FT%TZ) $host $module: FAILED: $*" >>"$log"
  exit 1
}

set -a
. "$root/.env" || fail "cannot read $root/.env"
set +a
[ -n "${PYTHON_DIGEST:-}" ] || fail "PYTHON_DIGEST is not set in $root/.env"
image="python:3.13-slim@sha256:$PYTHON_DIGEST"

{
  echo "== $(date -u +%FT%TZ) $host $module $mode"

  rm -rf "$work"
  mkdir -p "$work/state"
  if [ -f "$state/$module.json" ]; then cp "$state/$module.json" "$work/state/"; fi

  modeargs=""
  if [ "$module" = ecb_fx ] || [ "$module" = treasury ]; then modeargs="--mode $mode"; fi
  deps=""
  if [ "$module" = chirps ]; then deps="pip install -q --no-cache-dir numpy==2.1.3 tifffile==2024.8.30 imagecodecs==2024.12.30 && "; fi

  while :; do
    # fetch and check: prints the one file it wrote (nothing when there is nothing new)
    docker run --rm \
        -v "$here:/feeds:ro" -v "$out:/out" -v "$work/state:/state" \
        -w /feeds "$image" \
        sh -c "${deps}python -B $module.py --host \"$host\" --state-dir /state --out-dir /out $modeargs" \
      >"$work/stdout" || fail "fetch step failed"
    path=$(tail -n 1 "$work/stdout")

    if [ -z "$path" ]; then
      echo "nothing new"
      break
    fi
    file="$out/$(basename "$path")"
    [ -s "$file" ] || fail "no file $file"
    echo "wrote $file ($(wc -l <"$file") lines)"

    docker compose -f "$root/compose.yml" run --rm \
        -e DATUMESH_PUBLISH_LANES="${DATUMESH_PUBLISH_LANES:-2}" \
        datumesh-mesh publish-stored "$file" || fail "publish-stored failed; the state is unchanged"

    cp "$work/state/$module.json" "$state/$module.json" || fail "could not keep the state"
    echo "published $file"
    # a history run writes one year a file: go on until there is nothing new; any other run is one file
    if [ "$mode" != history ]; then break; fi
  done
} >>"$log" 2>&1
