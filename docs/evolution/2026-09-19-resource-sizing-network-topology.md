# 2026-09-19: Resource Sizing + Network Topology Operator Docs

Closes Medium findings #52 and #53 from
[`docs/product/production-readiness-audit-2026-09-08.md`](../product/production-readiness-audit-2026-09-08.md).
Pure documentation — no code changes.

## #52 — Resource sizing guidance

New doc: [`docs/operations/resource-sizing.md`](../operations/resource-sizing.md). Gives
CPU/memory `request`/`limit` recommendations for `embyr-server` (minimum-viable/eval tier and
moderate-production tier) and `embyr-agent` (lighter thin-proxy tier). Explicitly labeled as
reasonable starting estimates, not load-tested numbers. Grounds the estimate in three concrete
architecture facts: Argon2id's `m=65536 KiB` (64 MiB) per-concurrent-hash cost
(`crates/embyr-core/src/auth/argon2.rs`), Tokio runtime baseline, and how ADR-079's 5 pool-sizing
env vars (`EMBYR_SYSTEM_DB_MAX_CONNECTIONS` etc.) correlate with memory headroom rather than a
fixed per-connection number.

## #53 — Network topology guidance

New doc: [`docs/operations/network-topology.md`](../operations/network-topology.md). Translates
ADR-001's stated requirements into concrete operator steps: example DNS record structure
(`grpc.yourdomain.com`/`api.yourdomain.com` public, `admin.internal.yourdomain.com` private-only),
a firewall/security-group rule example keeping `:9090` off `0.0.0.0/0`, and BrowserChannel sticky
routing via nginx's `hash $arg_SID consistent` upstream directive — with an honest note that a
stock cloud LB's cookie-based stickiness does not match embyr's actual `SID`-query-param affinity
key, so an L7 proxy (or, as a weaker fallback, client-IP-hash on an L4 NLB) is required once
scaling past one instance.

## What was not done

No load testing was run to validate the resource-sizing numbers — flagged explicitly in the doc
itself. No new automation (Terraform/CloudFormation) was written for the firewall rule or DNS
records; both docs give example commands/config, matching the existing "no automation exists"
honesty convention from `runbook.md`/`backup-disaster-recovery.md`.

## Cross-references

Both new docs cross-reference each other, `single-host-deployment.md`, and ADR-001/ADR-079 rather
than duplicating their content.
