#!/usr/bin/env bash
# Black-box check of what the public internet can reach on console.<DOMAIN> (the Caddyfile allowlist).
# Run it after the first start, after editing the Caddyfile, and after every update (update.sh does).
# Needs DNS + TLS to be working. Exit 1 if anything operator-only answers or the SPA is unreachable.
# Keep this list in step with the Caddyfile header and crates/embyr-server/src/admin/router.rs.
# UNTESTED: see the PR description.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$here/lib.sh"

domain="$(envval DOMAIN)"
[ -n "$domain" ] || die "DOMAIN missing in $ENV_FILE"
console="https://console.$domain"
bad=0

status() { curl -s -o /dev/null -w '%{http_code}' -m 10 -X "$1" "$console$2" || true; }

expect() { # expect METHOD PATH WANT
  local got
  got="$(status "$1" "$2")"
  if [ "$got" = "$3" ]; then log "ok   $1 $2 -> $got"; else log "FAIL $1 $2 -> $got (want $3)"; bad=1; fi
}
expect_not_404() { # allowed API paths must reach the upstream (any status except the proxy's 404)
  local got
  got="$(status "$1" "$2")"
  if [ "$got" != 404 ] && [ "$got" != 000 ]; then log "ok   $1 $2 -> $got (reaches upstream)"; else log "FAIL $1 $2 -> $got (should reach upstream)"; bad=1; fi
}

# Must be invisible from outside.
expect GET /metrics 404
expect GET /healthz 404
expect GET /livez 404
expect POST /admin/v1/accounts 404
expect POST /admin/v1/billing/run-metering 404
expect POST /admin/v1/projects/example/suspend 404
expect POST /admin/v1/projects/example/activate 404
expect GET /admin/v1/oidc_providers 404
expect GET /admin/v1/not-a-route 404
expect GET /admin/v1 404

# Must work for the SPA.
expect GET / 302
expect GET /admin/ 200
expect_not_404 GET /admin/v1/auth/session
expect_not_404 GET /admin/v1/projects
expect_not_404 GET /admin/v1/members
expect_not_404 GET /admin/v1/billing

if [ "$bad" != 0 ]; then log "EXPOSURE CHECK FAILED"; exit 1; fi
log "exposure check passed"
