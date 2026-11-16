#!/bin/sh
# Nightly backup of the production store (quran-box, run by the datumesh user's crontab):
# pg_dump (custom format) into /srv/datumesh/backups (mode 0700, dumps 0600), the 14 newest kept,
# then the newest dump copied off the server to vmi630775 with a key that can only write into one
# directory there (rrsync -wo; no shell). A dump holds the store, never the signing key, which
# lives in secrets/ and is not backed up. Logs to stdout; status to status/backup.json.
set -e
dir=/srv/datumesh
umask 077
ts="$(date -u +%Y%m%dT%H%M%SZ)"
out="$dir/backups/datumesh-$ts.dump"
docker exec datumesh-datumesh-postgres-1 pg_dump -U postgres -Fc datumesh > "$out.part"
mv "$out.part" "$out"
ls -1t "$dir"/backups/datumesh-*.dump | tail -n +15 | while read -r old; do rm -f "$old"; done
sum="$(sha256sum "$out" | cut -d' ' -f1)"
echo "$sum  $(basename "$out")" > "$out.sha256"
ls -1t "$dir"/backups/datumesh-*.dump.sha256 | tail -n +15 | while read -r old; do rm -f "$old"; done
offsite=failed
if rsync -t -e "ssh -i $dir/env/backup_ed25519 -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=$dir/env/known_hosts" \
    "$out" "$out.sha256" dmbackup@207.180.222.183:/; then
  offsite=ok
fi
printf '{"at": "%s", "dump": "%s", "bytes": %s, "sha256": "%s", "offsite": "%s"}\n' \
  "$(date -u +%FT%TZ)" "$(basename "$out")" "$(wc -c < "$out")" "$sum" "$offsite" | tee "$dir/status/backup.json"
[ "$offsite" = ok ]
