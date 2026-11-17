#!/usr/bin/env bash
# Deploy a new immutable embyr-server image, wait for it to be healthy, roll back automatically if not.
#
#   scripts/update.sh sha-1a2b3c4      (or vX.Y.Z)      never "master"
#   SKIP_BACKUP=1 scripts/update.sh ...                 skip the pre-update backup (not recommended)
#   HEALTH_TIMEOUT=180 scripts/update.sh ...            seconds to wait for a healthy container (default 120)
#
# Flow: validate tag -> pull -> backup.sh (aborts the update if the backup fails) -> switch tag in
# .env.production -> `up -d --wait` (healthcheck = /healthz) -> on failure restore the previous tag.
#
# Why rollback of the IMAGE is safe: migrations are forward-only and run at server start, but both
# migrators set sqlx `ignore_missing`, so the previous image boots against a schema that is already
# newer (docs/operations/migration-policy.md, "Older binary on a newer schema"). That holds only for
# expand/contract-compliant migrations (the CI lint enforces it). If a release's migration damaged data,
# image rollback is not enough: scripts/restore.sh --prod from the pre-update backup.
# Requires `docker login ghcr.io` (PAT with read:packages) while the package is private.
# UNTESTED: see the PR description.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$here/lib.sh"

unset EMBYR_IMAGE_TAG  # an exported variable would override .env.production in every compose call
new="${1:-}"
re='^(sha-[0-9a-f]{7}|v[0-9]+\.[0-9]+\.[0-9]+)$'
[[ "$new" =~ $re ]] || die "usage: update.sh <sha-xxxxxxx | vX.Y.Z>  (immutable tags only, never master)"
if grep -q 'REPLACE_ME' "$ENV_FILE"; then die "$ENV_FILE still contains REPLACE_ME placeholders"; fi

prev="$(envval EMBYR_IMAGE_TAG)"
[ -n "$prev" ] || die "EMBYR_IMAGE_TAG missing in $ENV_FILE"
timeout="${HEALTH_TIMEOUT:-120}"

set_tag() { # atomic rewrite of the EMBYR_IMAGE_TAG line; mktemp files are mode 600
  local tmp
  tmp="$(mktemp "$ENV_FILE.XXXXXX")"
  awk -v t="$1" '
    /^EMBYR_IMAGE_TAG=/ { print "EMBYR_IMAGE_TAG=" t; done = 1; next }
    { print }
    END { if (!done) print "EMBYR_IMAGE_TAG=" t }
  ' "$ENV_FILE" >"$tmp"
  mv "$tmp" "$ENV_FILE"
}

dc config -q
log "pulling embyr-server:$new (currently running: $prev)"
EMBYR_IMAGE_TAG="$new" dc pull embyr-server

if [ "${SKIP_BACKUP:-0}" != 1 ]; then
  log "backing up before the update"
  "$here/backup.sh"
fi

set_tag "$new"
log "starting $new"
if dc up -d --no-deps --wait --wait-timeout "$timeout" embyr-server; then
  log "UPDATE OK: $prev -> $new"
  printf '%s %s -> %s OK\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$prev" "$new" >>"$BUNDLE_DIR/deploy-history.log"
  "$here/verify-exposure.sh" || log "WARNING: exposure check failed, see above (Caddyfile or routes changed?)"
  exit 0
fi

log "NEW VERSION NOT HEALTHY. Last log lines:"
dc logs --tail 40 embyr-server >&2 || true
log "rolling back to $prev"
set_tag "$prev"
if dc up -d --no-deps --wait --wait-timeout "$timeout" embyr-server; then
  printf '%s %s -> %s FAILED, rolled back\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$prev" "$new" >>"$BUNDLE_DIR/deploy-history.log"
  die "update to $new failed; rolled back to $prev (running again)"
fi
die "update to $new failed AND rollback to $prev is not healthy: manual intervention needed (docker compose logs embyr-server)"
