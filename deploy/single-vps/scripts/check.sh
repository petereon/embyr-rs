#!/usr/bin/env bash
# Local health check for the VPS, meant for cron/systemd every 5 minutes. Complements (does not replace)
# an EXTERNAL uptime monitor on https://api.<DOMAIN>/healthz: only an outside probe notices a dead VPS,
# and CHECK_PING_URL (a dead-man's-switch such as healthchecks.io) covers that from the inside.
#
# Checks: all three containers running and not unhealthy/restarting; public /healthz returns ok;
# root disk below 85%; newest backup younger than 36 h. Exit 1 listing every problem; on success pings
# CHECK_PING_URL if set (backup.env), on failure pings "$CHECK_PING_URL/fail".
# UNTESTED: see the PR description.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$here/lib.sh"
load_backup_env

domain="$(envval DOMAIN)"
BACKUP_DIR="${BACKUP_DIR:-$BUNDLE_DIR/backups}"
problems=()

for svc in postgres embyr-server caddy; do
  line="$(dc ps --format '{{.Service}} {{.State}} {{.Health}}' "$svc" 2>/dev/null || true)"
  line="${line% }" # services without a healthcheck (caddy) print an empty Health column
  case "$line" in
    "$svc running" | "$svc running healthy") ;;
    *) problems+=("container $svc: '${line:-not found}'") ;;
  esac
done

body="$(curl -fsS -m 10 "https://api.$domain/healthz" 2>/dev/null || true)"
case "$body" in
  *'"ok"'*) ;;
  *) problems+=("https://api.$domain/healthz did not answer ok: '${body:-no response}'") ;;
esac

used="$(df --output=pcent / | tail -n 1 | tr -dc '0-9')"
if [ "${used:-100}" -ge 85 ]; then problems+=("root disk ${used}% full"); fi

if ! find "$BACKUP_DIR" -maxdepth 1 -type f -name 'embyr-*.dump' -mmin -2160 2>/dev/null | grep -q .; then
  problems+=("no local backup newer than 36 h in $BACKUP_DIR")
fi

ping_url() {
  [ -n "${CHECK_PING_URL:-}" ] || return 0
  curl -fsS -m 10 --retry 3 -o /dev/null "$CHECK_PING_URL$1" || true
}

if [ "${#problems[@]}" -gt 0 ]; then
  for p in "${problems[@]}"; do log "PROBLEM: $p"; done
  ping_url /fail
  exit 1
fi
log "all checks passed"
ping_url ""
