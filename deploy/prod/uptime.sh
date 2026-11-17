#!/bin/sh
# The uptime alert, from outside quran-box (vmi630775's /etc/cron.d/datumesh-prod, every 5 minutes): through
# Cloudflare, as a client sees it, GET https://datumesh.com/.well-known/datumesh.json (200, a key
# document for datumesh.com) and the first cell, Genesis 1:1 (an `ok` envelope). Two failures in
# a row are an ALERT. The Genesis check runs only once $dir/scripture-loaded exists (created after
# the scripture load; until then the store is empty and only the key document is checked). Writes /root/datumesh-prod/status/uptime.json and uptime.txt, appends the
# line to stdout (the cron log), exits 1 on an alert. Log and status file, as for mesh-test.
set -u
dir=/root/datumesh-prod
mkdir -p "$dir/status"
now="$(date -u +%FT%TZ)"
fail=""
kd="$(curl -fsS --max-time 15 https://datumesh.com/.well-known/datumesh.json 2>/dev/null)" || fail="$fail keydoc"
[ -z "$fail" ] && ! printf '%s' "$kd" | grep -q '"host":"datumesh.com"' && fail="$fail keydoc-host"
checked="keydoc"
if [ -f "$dir/scripture-loaded" ]; then
  checked="keydoc and Genesis 1:1"
  cell='https://datumesh.com/cell/datumesh.com/scripture/verse/text(corpus:%22tanakh%22,book:%22Gen%22,chapter:1,verse:1,edition:%22uxlc%22)'
  env="$(curl -sS --max-time 15 "$cell" 2>/dev/null)"
  printf '%s' "$env" | grep -q '"outcome":"ok"' || fail="$fail genesis-1-1"
fi
state="$dir/status/uptime.state"
prev=0; [ -f "$state" ] && prev="$(cat "$state")"
if [ -n "$fail" ]; then n=$((prev + 1)); else n=0; fi
echo "$n" > "$state"
alert=false; [ "$n" -ge 2 ] && alert=true
printf '{"checked": "%s", "ok": %s, "alert": %s, "failing": "%s", "consecutive_failures": %s}\n' \
  "$now" "$([ -z "$fail" ] && echo true || echo false)" "$alert" "$(echo $fail)" "$n" > "$dir/status/uptime.json"
line="$now $($alert && echo "ALERT" || { [ -z "$fail" ] && echo OK || echo WARN; }):${fail:- $checked ok} ($n in a row)"
echo "$line" > "$dir/status/uptime.txt"
echo "$line"
! $alert
