#!/usr/bin/env bash
# Shared helpers, sourced by the other scripts. Not meant to be executed directly.
# UNTESTED: see the PR description.

BUNDLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="$BUNDLE_DIR/.env.production"
COMPOSE_FILE_PATH="$BUNDLE_DIR/docker-compose.prod.yml"
BACKUP_ENV="${BACKUP_ENV:-$BUNDLE_DIR/backup.env}"

log() { printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >&2; }
die() { log "ERROR: $*"; exit 1; }

# docker compose bound to this bundle (project name is fixed to "embyr" in the compose file).
dc() { docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE_PATH" "$@"; }

# envval KEY: print KEY's value from .env.production without evaluating it as shell.
# Strips one pair of surrounding quotes. Prints nothing when the key is absent.
envval() {
  local v
  v=$(grep -m1 "^$1=" "$ENV_FILE" | cut -d= -f2-) || true
  v=${v%\"}; v=${v#\"}; v=${v%\'}; v=${v#\'}
  printf '%s' "$v"
}

# load_backup_env: source backup.env (shell syntax, trusted, chmod 600) if it exists.
load_backup_env() {
  if [ -r "$BACKUP_ENV" ]; then
    set -a
    # shellcheck source=/dev/null
    . "$BACKUP_ENV"
    set +a
  fi
}

[ -r "$ENV_FILE" ] || die "missing $ENV_FILE (cp .env.production.example .env.production, then fill it in)"
