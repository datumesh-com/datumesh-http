#!/bin/sh
# The 5-minute check on quran-box (the datumesh user's crontab): the error alert and resources.
# * errors: the mesh's /metrics, read on khatm_default (the mesh's own address there; nothing is
#   published), compared with the previous run: any new 5xx answer, or any new `error` envelope,
#   is an ALERT; so is a mesh that does not answer, or containers not running.
# * resources: disk of / over 70%, memory over 80%, Postgres data over 50 GB.
# * manifests: the bytes of the slice manifest tables (manifests + manifest_members, with their
#   indexes), and their growth since the day before (status/manifest-daily: the first reading of
#   each UTC day). Reported, not alerted (the user's item 6, 2026-11-25).
# Writes status/check.json and a one-line status/check.txt, appends the line to stdout (the cron
# log), and exits 1 on an alert. The same convention as mesh-test: log and status file, no e-mail.
set -u
dir=/srv/datumesh
now="$(date -u +%FT%TZ)"
alert=""
for c in datumesh-datumesh-mesh-1 datumesh-datumesh-postgres-1; do
  [ "$(docker inspect -f '{{.State.Running}}' "$c" 2>/dev/null)" = true ] || alert="$alert $c-not-running"
done
ip="$(docker inspect -f '{{(index .NetworkSettings.Networks "khatm_default").IPAddress}}' datumesh-datumesh-mesh-1 2>/dev/null)"
m="$(curl -fsS --max-time 10 "http://$ip:8080/metrics" 2>/dev/null)" || { alert="$alert mesh-unreachable"; m=""; }
e5="$(printf '%s\n' "$m" | awk -F'[ ]' '/^datumesh_http_requests_total\{.*status="5/ {s+=$NF} END {print s+0}')"
ee="$(printf '%s\n' "$m" | awk -F'[ ]' '/^datumesh_envelopes_total\{.*outcome="error"/ {s+=$NF} END {print s+0}')"
state="$dir/status/check.state"
if [ -f "$state" ]; then read -r p5 pe < "$state"; else p5=$e5; pe=$ee; fi
# A restart resets the counters: a lower count is a new baseline, not an error.
[ "$e5" -lt "$p5" ] && p5=0
[ "$ee" -lt "$pe" ] && pe=0
n5=$((e5 - p5)); ne=$((ee - pe))
[ -n "$m" ] && echo "$e5 $ee" > "$state"
[ "$n5" -gt 0 ] && alert="$alert ${n5}x5xx"
[ "$ne" -gt 0 ] && alert="$alert ${ne}xerror-envelope"
disk="$(df -P / | awk 'NR==2 {gsub("%", "", $5); print $5}')"
mem="$(awk '/^MemTotal:/ {t=$2} /^MemAvailable:/ {a=$2} END {printf "%d", (t-a)*100/t}' /proc/meminfo)"
pg="$(docker exec datumesh-datumesh-postgres-1 psql -U postgres -tAc 'select pg_database_size(current_database())' -d datumesh 2>/dev/null | tr -d ' ')"
pg="${pg:-0}"
mf="$(docker exec datumesh-datumesh-postgres-1 psql -U postgres -tAc "select pg_total_relation_size('manifests') + pg_total_relation_size('manifest_members')" -d datumesh 2>/dev/null | tr -d ' ')"
mf="${mf:-0}"
daily="$dir/status/manifest-daily"
today="$(date -u +%F)"
[ "$mf" -gt 0 ] && ! grep -q "^$today " "$daily" 2>/dev/null && echo "$today $mf" >> "$daily"
prev="$(awk -v d="$today" '$1 < d {v=$2} END {print v}' "$daily" 2>/dev/null)"
if [ -n "$prev" ]; then mfg=$((mf - prev)); else mfg=0; fi
[ "$disk" -gt 70 ] && alert="$alert disk"
[ "$mem" -gt 80 ] && alert="$alert memory"
[ "$pg" -gt 50000000000 ] && alert="$alert postgres"
ok=true; [ -n "$alert" ] && ok=false
printf '{"checked": "%s", "ok": %s, "alert": "%s", "new_5xx": %s, "new_error_envelopes": %s, "disk_percent": %s, "memory_percent": %s, "postgres_bytes": %s, "manifest_bytes": %s, "manifest_growth_since_yesterday": %s}\n' \
  "$now" "$ok" "$(echo $alert)" "$n5" "$ne" "$disk" "$mem" "$pg" "$mf" "$mfg" > "$dir/status/check.json"
line="$now $($ok && echo OK || echo "ALERT:$alert"): 5xx +$n5, error envelopes +$ne, disk $disk%, memory $mem%, postgres $pg bytes, manifests $mf bytes (+$mfg since yesterday)"
echo "$line" > "$dir/status/check.txt"
echo "$line"
$ok
