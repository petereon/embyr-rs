#!/usr/bin/env bash
# Host firewall (ufw): SSH from SSH_SOURCE, 80/tcp and 443/tcp from anywhere, everything else denied.
#
#   sudo SSH_SOURCE=203.0.113.7/32 ./scripts/firewall.sh     # SSH only from your own IP
#   sudo SSH_SOURCE=any            ./scripts/firewall.sh     # SSH from anywhere (rate limited)
#   SSH_PORT=2222 changes the SSH port (default 22).
#
# SSH_SOURCE has no default on purpose: a wrong guess locks you out of a remote box. Keep your current
# SSH session open and test a second one before closing it.
#
# IMPORTANT: ufw does NOT filter ports that Docker publishes (Docker writes iptables rules ahead of
# ufw's). It protects the host's own services (sshd, anything you install later). What keeps Postgres
# and embyr's internal ports private is that docker-compose.prod.yml publishes only 80/443 and
# 127.0.0.1:9090. Re-check with `ss -tlnp` and `docker compose ps` after every compose change.
# No AAAA DNS records unless you have configured Docker IPv6 properly (see the runbook): otherwise all
# IPv6 clients appear to embyr as one address.
# UNTESTED: see the PR description.
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "run as root (sudo)" >&2; exit 1; }
command -v ufw >/dev/null || { echo "ufw not installed (apt install ufw)" >&2; exit 1; }
: "${SSH_SOURCE:?set SSH_SOURCE to a CIDR (203.0.113.7/32) or 'any'}"
SSH_PORT="${SSH_PORT:-22}"

ufw default deny incoming
ufw default allow outgoing

if [ "$SSH_SOURCE" = any ]; then
  # `limit` = allow, but throttle repeated connection attempts from one address.
  ufw limit "$SSH_PORT"/tcp comment 'ssh (rate limited)'
else
  ufw allow from "$SSH_SOURCE" to any port "$SSH_PORT" proto tcp comment 'ssh (restricted source)'
fi
ufw allow 80/tcp comment 'http (ACME + redirect)'
ufw allow 443/tcp comment 'https'

# Rules are in place; only now switch it on (idempotent).
ufw --force enable
ufw status verbose
