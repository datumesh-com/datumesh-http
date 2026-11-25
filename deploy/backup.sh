#!/bin/sh
# Nightly backup of the test mesh's database: pg_dump (custom format) of `datumesh` from the
# project's Postgres container into $DATUMESH_DIR/backups (mode 0700; dumps 0600), each with a
# `<dump>.sha256` file (`sha256sum -c` from that directory), the 14 newest kept. Run by cron
# (deploy/install_cron.sh); logs to stdout.
set -e
dir="${DATUMESH_DIR:-/root/datumesh-mesh-test}"
umask 077
mkdir -p "$dir/backups"
chmod 700 "$dir/backups"
ts="$(date -u +%Y%m%dT%H%M%SZ)"
out="$dir/backups/datumesh-$ts.dump"
docker exec datumesh-mesh-test-postgres-1 pg_dump -U postgres -Fc datumesh > "$out.part"
mv "$out.part" "$out"
(cd "$dir/backups" && sha256sum "$(basename "$out")" > "$(basename "$out").sha256")
ls -1t "$dir"/backups/datumesh-*.dump | tail -n +15 | while read -r old; do rm -f "$old" "$old.sha256"; done
echo "$(date -u +%FT%TZ) backup $out $(wc -c < "$out") bytes; $(ls -1 "$dir"/backups/datumesh-*.dump | wc -l) kept"
