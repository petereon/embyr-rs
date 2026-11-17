#!/usr/bin/env bash
# Logical backup of the embyr SYSTEM database (pg_dump custom format) -> rclone remote.
#   daily/   one dump per run, keep newest KEEP_DAILY
#   weekly/  the Sunday run is also copied here, keep newest KEEP_WEEKLY
# Exit non-zero (and ping "$PING_URL/fail") on ANY failure. Config: backup.env (see backup.env.example).
# Runbook: docs/operations/single-vps-deploy.md. Customer databases are NOT covered (customer's job,
# docs/operations/backup-disaster-recovery.md).
#
# RPO = interval between runs (daily timer => up to 24 h of lost sign-ups/projects/metrics). This is
# not PITR. The dump holds only ciphertext for secrets: EMBYR_ENCRYPTION_KEY is NOT in it, back that up
# separately and offline.
# UNTESTED: see the PR description.
set -Eeuo pipefail
umask 077

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$here/lib.sh"
load_backup_env

: "${BACKUP_REMOTE:?set BACKUP_REMOTE in backup.env, e.g. b2crypt:embyr-prod}"
KEEP_DAILY="${KEEP_DAILY:-7}"
KEEP_WEEKLY="${KEEP_WEEKLY:-4}"
BACKUP_DIR="${BACKUP_DIR:-$BUNDLE_DIR/backups}"
PING_URL="${PING_URL:-}"

ping_url() { # ping_url [suffix]
  [ -n "$PING_URL" ] || return 0
  curl -fsS -m 10 --retry 3 -o /dev/null "$PING_URL$1" || log "warning: could not reach PING_URL"
}

# EXIT (not ERR) so that `die` and set -e failures both report. $ok flips to 1 only on the last line.
ok=0
trap 'rc=$?; if [ "$ok" != 1 ]; then log "backup FAILED (exit $rc)"; ping_url /fail; fi' EXIT

command -v rclone >/dev/null || die "rclone not installed (apt install rclone)"
mkdir -p "$BACKUP_DIR"
chmod 700 "$BACKUP_DIR"

# One backup at a time (cron + manual run, or a slow upload overlapping the next timer).
exec 9>"$BACKUP_DIR/.lock"
flock -n 9 || die "another backup is already running"

ts="$(date -u +%Y%m%dT%H%M%SZ)"
name="embyr-$ts.dump"
tmp="$BACKUP_DIR/$name.partial"
final="$BACKUP_DIR/$name"

log "dumping system DB -> $name"
dc exec -T postgres pg_dump -U embyr -d embyr --format=custom --compress=6 >"$tmp"
[ -s "$tmp" ] || die "dump is empty"

# Cheap integrity check: pg_restore must be able to read the table of contents back.
dc exec -T postgres pg_restore --list <"$tmp" >/dev/null
mv "$tmp" "$final"
log "dump ok: $(du -h "$final" | cut -f1)"

log "uploading to $BACKUP_REMOTE/daily/"
rclone copyto "$final" "$BACKUP_REMOTE/daily/$name"
rclone lsf --files-only "$BACKUP_REMOTE/daily/$name" | grep -qx "$name" || die "upload not visible on remote"

if [ "$(date -u +%u)" = 7 ]; then
  log "Sunday: also keeping a weekly copy"
  rclone copyto "$final" "$BACKUP_REMOTE/weekly/$name"
fi

# Keep the newest $2 files of remote subdir $1. Count-based on purpose: if backups ever stop, nothing
# is pruned. Runs only after a successful upload (a failure above exits before this).
prune() {
  local sub="$1" keep="$2" f
  rclone lsf --files-only "$BACKUP_REMOTE/$sub" | sort | head -n "-$keep" | while read -r f; do
    case "$f" in
      embyr-*.dump)
        log "pruning $sub/$f"
        rclone deletefile "$BACKUP_REMOTE/$sub/$f" </dev/null
        ;;
    esac
  done
}
prune daily "$KEEP_DAILY"
if rclone lsf --files-only "$BACKUP_REMOTE/weekly" >/dev/null 2>&1; then
  prune weekly "$KEEP_WEEKLY"
fi

# Local staging copies: keep about three days.
find "$BACKUP_DIR" -maxdepth 1 -type f -name 'embyr-*.dump' -mtime +2 -delete

log "backup OK: $name"
ok=1
ping_url ""
