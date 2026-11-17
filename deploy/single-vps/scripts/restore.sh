#!/usr/bin/env bash
# Restore an embyr system-DB dump made by backup.sh.
#
#   restore.sh (--latest | <dump file | rclone path>)           restore drill into SCRATCH db embyr_restore_test,
#                                                               print sanity numbers, drop it again
#   restore.sh --prod (--latest | <dump file | rclone path>)    DESTRUCTIVE: replace the live `embyr` database
#
# Env: KEEP_SCRATCH=1 keeps the scratch DB after a drill; SKIP_SAFETY_DUMP=1 skips the pre-restore dump
# of the live DB in --prod mode (only if the live DB is too damaged to dump).
# Drill procedure: scripts/restore-test.md. Runbook: docs/operations/single-vps-deploy.md.
# UNTESTED: see the PR description.
set -euo pipefail
umask 077

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$here/lib.sh"
load_backup_env

usage() { sed -n '2,12p' "${BASH_SOURCE[0]}" >&2; exit 2; }

prod=0
src=""
while [ $# -gt 0 ]; do
  case "$1" in
    --prod) prod=1 ;;
    --latest) src="latest" ;;
    -h | --help) usage ;;
    -*) usage ;;
    *) src="$1" ;;
  esac
  shift
done
[ -n "$src" ] || usage

BACKUP_DIR="${BACKUP_DIR:-$BUNDLE_DIR/backups}"
work="$(mktemp -d)"
cleanup() { rm -f "$work/dump"; rmdir "$work" 2>/dev/null || true; }
trap cleanup EXIT

if [ "$src" = latest ]; then
  : "${BACKUP_REMOTE:?set BACKUP_REMOTE in backup.env for --latest}"
  newest="$(rclone lsf --files-only "$BACKUP_REMOTE/daily" | sort | tail -n 1)"
  [ -n "$newest" ] || die "no dumps found in $BACKUP_REMOTE/daily"
  src="$BACKUP_REMOTE/daily/$newest"
  log "latest dump: $src"
fi

if [ -f "$src" ]; then
  cp -- "$src" "$work/dump"
else
  log "fetching $src"
  rclone copyto "$src" "$work/dump"
fi
[ -s "$work/dump" ] || die "dump is empty or missing"

psql_pg() { dc exec -T postgres psql -U embyr -d postgres -v ON_ERROR_STOP=1 "$@"; }
restore_into() { dc exec -T postgres pg_restore -U embyr --no-owner --no-privileges --exit-on-error -d "$1" <"$work/dump"; }

if [ "$prod" = 0 ]; then
  log "drill: restoring into scratch database embyr_restore_test (live DB untouched)"
  psql_pg -c "DROP DATABASE IF EXISTS embyr_restore_test" -c "CREATE DATABASE embyr_restore_test"
  restore_into embyr_restore_test
  log "restore finished; sanity numbers:"
  dc exec -T postgres psql -U embyr -d embyr_restore_test -v ON_ERROR_STOP=1 -At -F ' = ' \
    -c "SELECT 'public tables', count(*) FROM information_schema.tables WHERE table_schema = 'public'" \
    -c "SELECT 'latest migration', max(version) FROM _sqlx_migrations" \
    -c "SELECT 'accounts', count(*) FROM accounts" \
    -c "SELECT 'users', count(*) FROM users" \
    -c "SELECT 'projects', count(*) FROM projects"
  if [ "${KEEP_SCRATCH:-0}" = 1 ]; then
    log "KEEP_SCRATCH=1: leaving embyr_restore_test in place; drop it when done"
  else
    psql_pg -c "DROP DATABASE embyr_restore_test"
    log "scratch database dropped"
  fi
  log "DRILL OK. Reminder: also confirm EMBYR_ENCRYPTION_KEY is retrievable from your offline copy."
  exit 0
fi

# ---- --prod ----
[ -t 0 ] || die "--prod needs an interactive terminal for the confirmation prompt"
log "THIS REPLACES THE LIVE DATABASE 'embyr' with the dump above. embyr-server will be stopped meanwhile."
log "Everything written since the dump was taken is lost. The encryption key in .env.production must be"
log "the one that was in use when the dump was taken, or encrypted columns will be unreadable."
read -r -p "Type 'overwrite embyr' to continue: " answer
[ "$answer" = "overwrite embyr" ] || die "aborted"

dc stop embyr-server

if [ "${SKIP_SAFETY_DUMP:-0}" != 1 ]; then
  mkdir -p "$BACKUP_DIR"
  safety="$BACKUP_DIR/embyr-pre-restore-$(date -u +%Y%m%dT%H%M%SZ).dump"
  log "safety dump of the current live DB -> $safety"
  dc exec -T postgres pg_dump -U embyr -d embyr --format=custom --compress=6 >"$safety" \
    || die "safety dump failed; fix it or re-run with SKIP_SAFETY_DUMP=1 (embyr-server is stopped)"
fi

psql_pg -c "DROP DATABASE IF EXISTS embyr WITH (FORCE)" -c "CREATE DATABASE embyr"
restore_into embyr
log "restore finished; starting embyr-server"
dc up -d --wait --wait-timeout 120 embyr-server
log "PROD RESTORE OK. Verify: sign in to the console, then run scripts/check.sh."
