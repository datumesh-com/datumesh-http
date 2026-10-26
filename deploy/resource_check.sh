#!/bin/sh
# The 15-minute resource check (cron, deploy/install_cron.sh): disk use of / (threshold 70%),
# memory in use (80%), the Postgres data of the test mesh (50 GB), and egress this month
# (200 GB). Writes $DATUMESH_DIR/status/resources.json and a one-line resources.txt, appends
# the line to stdout (the cron log), and exits 1 when any threshold is exceeded.
#
# Egress is the VM's whole outbound traffic (the default route's interface, tx_bytes), so it
# over-counts the mesh's share. The kernel's counter starts again at boot: the month's total is
# kept in status/egress.state (month, last counter, bytes this month), adding the counter's
# growth each run, or the whole counter after a reboot (when it went down).
set -u
dir="${DATUMESH_DIR:-/root/datumesh-mesh-test}"
status="$dir/status"
mkdir -p "$status"
now="$(date -u +%FT%TZ)"
month="$(date -u +%Y-%m)"
disk_pct="$(df -P / | awk 'NR==2 {gsub("%", "", $5); print $5}')"
mem_pct="$(awk '/^MemTotal:/ {t=$2} /^MemAvailable:/ {a=$2} END {printf "%d", (t-a)*100/t}' /proc/meminfo)"
pg_bytes="$(docker exec datumesh-mesh-test-postgres-1 psql -U postgres -tAc 'select sum(pg_database_size(datname)) from pg_database' 2>/dev/null | tr -d ' ')"
pg_bytes="${pg_bytes:-0}"
iface="$(ip route show default | awk '{for (i=1;i<=NF;i++) if ($i=="dev") {print $(i+1); exit}}')"
tx="$(cat "/sys/class/net/$iface/statistics/tx_bytes" 2>/dev/null || echo 0)"
state="$status/egress.state"
# The first run counts everything since boot (an over-count when the VM booted last month).
if [ -f "$state" ]; then read -r smonth slast stotal < "$state"; else smonth="$month"; slast=0; stotal=0; fi
if [ "$smonth" != "$month" ]; then smonth="$month"; stotal=0; fi
if [ "$tx" -ge "$slast" ]; then stotal=$((stotal + tx - slast)); else stotal=$((stotal + tx)); fi
echo "$smonth $tx $stotal" > "$state.new" && mv "$state.new" "$state"
over=""
[ "$disk_pct" -gt 70 ] && over="$over disk"
[ "$mem_pct" -gt 80 ] && over="$over memory"
[ "$pg_bytes" -gt 50000000000 ] && over="$over postgres"
[ "$stotal" -gt 200000000000 ] && over="$over egress"
ok=true; [ -n "$over" ] && ok=false
gb() { awk -v b="$1" 'BEGIN {printf "%.2f", b / 1e9}'; }
cat > "$status/resources.json.new" <<JSON
{"checked": "$now", "ok": $ok, "over": "$(echo $over)",
 "disk": {"used_percent": $disk_pct, "threshold_percent": 70},
 "memory": {"used_percent": $mem_pct, "threshold_percent": 80},
 "postgres": {"bytes": $pg_bytes, "threshold_bytes": 50000000000},
 "egress": {"month": "$smonth", "interface": "$iface", "bytes": $stotal, "threshold_bytes": 200000000000}}
JSON
mv "$status/resources.json.new" "$status/resources.json"
line="$now $( $ok && echo OK || echo "OVER:$over"): disk ${disk_pct}%/70%, memory ${mem_pct}%/80%, postgres $(gb "$pg_bytes") GB/50 GB, egress $smonth $(gb "$stotal") GB/200 GB"
echo "$line" > "$status/resources.txt"
echo "$line"
$ok
