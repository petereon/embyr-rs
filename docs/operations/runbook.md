# embyr-server Incident Runbook

**Status:** checked-in artifact, not auto-deployed. Nothing in this repo automatically
imports `grafana-dashboard.json` into a Grafana instance, applies `alert-rules.yml` to a
Prometheus, or wires alert delivery (Slack/PagerDuty/etc). An operator does all three
manually against their own monitoring stack. This mirrors the "not automated" framing
`docs/operations/backup-disaster-recovery.md` (finding #18) already uses for backup/DR.

This document's job is **notice and triage** — recognizing an incident is happening and
narrowing down which subsystem is at fault. It is not a full recovery procedure; where a
scenario needs one (notably system-DB loss), it links out rather than duplicating.

## What's actually exported

- `GET /metrics` on **:9090** (admin port, `Authorization: Bearer $EMBYR_ADMIN_KEY`) —
  Prometheus text format. Metric names and their source: see
  `docs/evolution/2026-08-08-observability.md` and `crates/embyr-server/src/observability.rs`.
- `GET /healthz` on **:8081** — readiness. Calls `SystemDb::probe()` (shared system Postgres
  pool) with a 3s timeout; 200 `{"status":"ok"}` or 503 `{"status":"unhealthy"}`. No auth,
  no request body echoed back (ADR-078, AC-HDC-11 — never leaks raw driver/schema text).
- `GET /livez` on **:8081** — liveness. Zero I/O, always 200 while the process is up. Never
  fails on a Postgres outage by design (ADR-078) — do not alert on `/livez` for DB problems,
  that's what `/healthz` and the metrics below are for.

Neither `/healthz` nor `/livez` is itself a Prometheus metric — they're plain HTTP
endpoints (for a Kubernetes-style orchestrator's own probes). To graph or alert on them
directly from Prometheus, deploy `blackbox_exporter` separately and point it at them; this
repo does not ship or configure one. The dashboard and alerts here lean on the metrics
embyr-server exports natively wherever possible instead.

## Incident: Postgres down after startup (finding #15 / #29)

This is the scenario the finding calls out by name: `/healthz` used to be a hardcoded
`200 OK`, so a system-DB outage that started *after* a healthy startup was invisible to an
orchestrator. ADR-078 fixed the check itself; this runbook is how an operator notices it
via monitoring rather than waiting for a client-facing outage report.

**Detect:**
1. `EmbyrSystemDbPoolExhausted` fires (`alert-rules.yml`) — `embyr_pg_pool_idle{pool="system"}`
   at 0 for 2+ minutes. This is the built-in-metric proxy signal.
2. If blackbox_exporter is deployed against `/healthz`: `probe_success` drops to 0, or the
   probe's own HTTP status code stops being 200.
3. Corroborating signals on the dashboard: `EmbyrRateLimiterPgErrorsClimbing` /
   `EmbyrRateLimiterPgTimeoutsHigh` firing around the same time (the rate limiter shares the
   same system DB pool and degrades on the same outage — see finding #20 below).

**Triage:**
1. Confirm it's the system DB and not a customer/tenant DB — `/healthz` and the pool
   gauges here are scoped to the *system* Postgres pool only (auth, sessions, admin API,
   project provisioning), not per-tenant `direct_pg`/BYOC backends. A single customer's
   backend being unreachable does not trip this alert or `/healthz`.
2. `curl -s -o /dev/null -w '%{http_code}\n' http://<host>:8081/healthz` against an
   affected pod — 503 confirms readiness has flipped; the orchestrator should already be
   pulling that pod out of rotation (that's the point of the readiness/liveness split in
   ADR-078: liveness stays green, so pods are NOT restarted, only de-routed).
3. Check the system Postgres instance directly (connectivity, `max_connections`, disk,
   replication lag if applicable) — `/healthz`'s `SystemDb::probe()` is `SELECT 1` + a
   schema check, so a 503 means the pool genuinely can't reach or validate the DB, not a
   embyr-server bug.
4. **Recovery** (restoring the actual database) is out of scope for this document — follow
   `docs/operations/backup-disaster-recovery.md` (finding #18), including its step 4
   precondition on `EMBYR_ENCRYPTION_KEY`/`EMBYR_ENCRYPTION_KEY_PREVIOUS` recoverability if
   this is a full-loss restore rather than a transient outage.
5. Once Postgres is reachable again, `/healthz` recovers on its own next probe (no cache,
   fresh check every request per ADR-078) — no embyr-server restart needed.

## Incident: rate limiter degraded (finding #20)

**Detect:** `EmbyrRateLimiterPgErrorsClimbing` and/or `EmbyrRateLimiterPgTimeoutsHigh`.

**Triage:**
1. This is a **bounded** degradation, not an outage: on a Postgres error or a >20ms
   timeout, `RateLimiter::check()` falls back to the same in-process, per-instance token
   bucket already used elsewhere — it does not disable rate limiting (that was the pre-fix
   bug this same finding closed; see `docs/evolution/2026-09-15-rate-limiter-fail-open.md`).
2. Effect during the degradation: rate limits are enforced per-replica instead of
   cluster-wide, so a project could burst up to (replica count × per-instance capacity)
   instead of the intended shared capacity. Not a security hole, but worth knowing if a
   customer reports "my rate limit felt loose" during this window.
3. Usually co-occurs with the Postgres-down scenario above (same system DB pool) — check
   `EmbyrSystemDbPoolExhausted` and the pool-idle panel first.
4. The sign-in rate limiter (`embyr_signin_rate_limit_pg_timeout_total`,
   `embyr_signin_rate_limit_requests_total`) shares the same system pool and same failure
   shape — check it too if this fires during a suspected DB incident.

## Incident: elevated gRPC error rate or latency

**Detect:** `EmbyrGrpcErrorRateHigh` (>5% non-OK over 5m) or `EmbyrGrpcLatencySLOBreach`
(p99 > 2s over 10m, the same threshold the histogram's own bucket boundaries are aligned
to — see `GRPC_DURATION_BUCKETS` in `observability.rs`).

**Triage:**
1. Break down `embyr_grpc_requests_total{status!="ok"}` by `method` and `status` on the
   dashboard to find which RPC and which tonic status code (`internal`, `unavailable`,
   `deadline_exceeded`, etc. — see `grpc_status_label()` in
   `crates/embyr-server/src/middleware/obs_helpers.rs` for the full mapping) is driving it.
2. `internal`/`unavailable` clustering on multiple methods at once → check the system DB
   pool panel and the Postgres-down runbook section above; a downstream DB problem surfaces
   here as gRPC errors before it surfaces as a `/healthz` failure in some cases.
3. `deadline_exceeded` or a latency-only breach with error rate flat → check the pool
   `size` vs `idle` gap (connections queuing, not failing) and whether a single tenant/DB
   is disproportionately represented (finding #17's collection-group scan cost is one
   known source of slow queries; `docs/evolution/2026-09-14-collection-group-query-index.md`).

## Incident: customer API-key compromise (finding #33)

**Why this is here:** every backend DSN / TOTP secret / OIDC client secret is
ECIES-encrypted with a recipient key *deterministically re-derived from the
project's API key* (`crates/embyr-core/src/auth/ecies.rs:derive_static_secret`).
There is no separate, independently-rotatable recipient keypair. This is a
deliberate design trade-off (no key-management infrastructure to operate), but
it means **API-key compromise = retroactive decryption of every ciphertext
ever encrypted for that project**, not just future ones.

**On confirmed or suspected compromise of a project's API key:**
1. Rotate the project's API key immediately (admin API key-rotation endpoint).
   This changes `derive_static_secret`'s input, so the recipient keypair
   changes too — but existing ciphertexts were encrypted under the *old*
   derived key and do not automatically re-encrypt.
2. Re-encrypt every ECIES ciphertext for that project under the new key:
   `backend_pg_dsn_enc`, `totp_secret_enc` (per user), `client_secret_enc`
   (per OIDC provider), any `hosted_identity_signing_keys.private_key_enc`
   rows. There is no automated re-encryption tool as of this writing — this
   is a manual/scripted admin-API operation (decrypt with old key, encrypt
   with new key, per row) until one is built.
3. Treat any secret whose plaintext could have been read (customer Postgres
   DSN, TOTP seed, OIDC client secret) as compromised in its own right —
   rotate it at its source (customer DB password, OIDC provider), not just
   the ECIES wrapper.
4. The 2026-09-19 KDF domain-separation fix (`docs/evolution/2026-09-19-ecies-kdf-domain-separation.md`)
   does not mitigate this scenario — it hardens against key-substitution
   attacks in the DH exchange, not against the API key itself leaking.

## Where to look next

- `docs/operations/grafana-dashboard.json` — the dashboard these alerts and panels above
  describe; import it into Grafana against a Prometheus scraping `:9090/metrics`.
- `docs/operations/alert-rules.yml` — the alert definitions themselves, apply to Prometheus.
- `docs/operations/backup-disaster-recovery.md` — full DB recovery procedure (finding #18).
- `docs/product/architecture/adr-078-liveness-readiness-split.md` — why `/healthz` and
  `/livez` mean what they mean here (deliberately inverted from the common Kubernetes-docs
  convention).
- `docs/evolution/2026-08-08-observability.md` — how every metric referenced above was
  built, and the full list with label cardinality notes (`project_id` is intentionally
  bounded on the rate-limiter counter — ADR-069/ADR-016).
