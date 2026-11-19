#!/bin/sh
# Run one feed job: fetch and check in the pinned python image, write one publish-stored file to
# /srv/datumesh/feeds/out/, publish it with datumesh-mesh, then keep the new state. Logs to /srv/datumesh/logs/.
#
# usage: run_feed.sh <module> [history]
#   module: ecb_fx estr policy_rates treasury boe chirps   (register runs by hand, see README.md)
#   history: ecb_fx only, backfills from eurofxref-hist.zip
#
# Reads /srv/datumesh/.env (PYTHON_DIGEST, DATUMESH_PUBLISH_LANES) and /srv/datumesh/compose.yml.
# The fetch step uses the default bridge network (it needs the internet, not the datumesh_backend network).
# The job works on a copy of its state: the state moves in place only after publish-stored succeeds.
set -eu

module=${1:?usage: run_feed.sh <module> [history]}
mode=${2:-daily}
case "$module" in
  ecb_fx|estr|policy_rates|treasury|boe|chirps) ;;
  *) echo "run_feed.sh: unknown module $module" >&2; exit 2 ;;
esac

here=$(cd "$(dirname "$0")" && pwd)
root=${DATUMESH_ROOT:-/srv/datumesh}
feeds="$root/feeds"
out="$feeds/out"
state="$feeds/state"
work="$feeds/work/$module"
log="$root/logs/feeds-$module.log"

mkdir -p "$out" "$state" "$root/logs"
exec 9>"$feeds/$module.lock"
if ! flock -n 9; then
  echo "$(date -u +%FT%TZ) $module: the previous run is still going" >>"$log"
  exit 0
fi

fail() {
  echo "$(date -u +%FT%TZ) $module: FAILED: $*" >>"$log"
  exit 1
}

set -a
. "$root/.env" || fail "cannot read $root/.env"
set +a
[ -n "${PYTHON_DIGEST:-}" ] || fail "PYTHON_DIGEST is not set in $root/.env"
image="python:3.13-slim@sha256:$PYTHON_DIGEST"

{
  echo "== $(date -u +%FT%TZ) $module $mode"

  rm -rf "$work"
  mkdir -p "$work/state"
  if [ -f "$state/$module.json" ]; then cp "$state/$module.json" "$work/state/"; fi

  modeargs=""
  if [ "$module" = ecb_fx ]; then modeargs="--mode $mode"; fi
  deps=""
  if [ "$module" = chirps ]; then deps="pip install -q --no-cache-dir numpy==2.1.3 tifffile==2024.8.30 && "; fi

  # fetch and check: prints the one file it wrote (nothing when there is nothing new)
  docker run --rm \
      -v "$here:/feeds:ro" -v "$out:/out" -v "$work/state:/state" \
      -w /feeds "$image" \
      sh -c "${deps}python -B $module.py --state-dir /state --out-dir /out $modeargs" \
    >"$work/stdout" || fail "fetch step failed"
  path=$(tail -n 1 "$work/stdout")

  if [ -z "$path" ]; then
    echo "nothing new"
    exit 0
  fi
  file="$out/$(basename "$path")"
  [ -s "$file" ] || fail "no file $file"
  echo "wrote $file ($(wc -l <"$file") lines)"

  docker compose -f "$root/compose.yml" run --rm \
      -e DATUMESH_PUBLISH_LANES="${DATUMESH_PUBLISH_LANES:-2}" \
      datumesh-mesh publish-stored "$file" || fail "publish-stored failed; the state is unchanged"

  cp "$work/state/$module.json" "$state/$module.json" || fail "could not keep the state"
  echo "published $file"
} >>"$log" 2>&1
