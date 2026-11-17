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
  pool) with a 3s timeout; 200 `{"status":"ok"}` or 503 `{"status":"unhealthy"}`. After
  SIGTERM it answers 503 `{"status":"draining"}` until the process exits (at most
  `EMBYR_SHUTDOWN_GRACE_SECS`, default 25) — expected during a deploy, not an incident. No auth,
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

## Maintenance: composite indexes created before typed ORDER BY

`ORDER BY` is now a typed multi-key expression (type rank + typed value). Composite
indexes created by earlier builds used the old text key `(fields->'f'->>'v')`; Postgres
cannot use them for `ORDER BY` any more (seq scan + sort on large collections). Newly
created indexes use the new keys (`order_key_exprs` in `embyr-pg-storage`, shared with
the query side). For each pre-existing composite index: delete it via the admin API
(`DELETE /admin/v1/projects/{id}/indexes/{index_id}`) and create it again with the same
fields. No automatic migration touches customer databases.

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

## Support: admin user lost their TOTP device

Email password reset (`POST /admin/v1/auth/password_reset/{request,confirm}`) still
needs a TOTP or recovery code, so it does **not** help here. If the user still has an
unused recovery code they can sign in or reset with it. Otherwise recovery is
operator-assisted. **There is no operator endpoint for TOTP re-enrolment (known gap)**;
the procedure re-runs the invitation flow, which enrols a fresh authenticator.

1. Verify the user's identity out of band (the admin console cannot do this for you).
2. In `psql -v email=user@example.com "$DATABASE_URL"` (system DB), one transaction:
   ```sql
   BEGIN;
   CREATE TEMP TABLE gone AS SELECT u.id, u.account_id, m.role
     FROM users u JOIN account_members m ON m.user_id = u.id WHERE lower(u.email) = lower(:'email');
   DELETE FROM sessions WHERE user_id IN (SELECT id FROM gone);
   DELETE FROM mfa_recovery_codes WHERE user_id IN (SELECT id FROM gone);
   UPDATE admin_api_keys SET revoked_at = now() WHERE member_id IN (SELECT id FROM gone) AND revoked_at IS NULL;
   UPDATE admin_api_keys SET member_id = NULL WHERE member_id IN (SELECT id FROM gone);
   DELETE FROM account_members WHERE user_id IN (SELECT id FROM gone);
   DELETE FROM users WHERE id IN (SELECT id FROM gone);   -- password_reset_tokens cascade
   SELECT * FROM gone;   -- note account_id and role; abort with ROLLBACK if empty or unexpected
   COMMIT;
   ```
3. Issue a fresh invitation for the same account and role. If another Owner/Admin
   remains, they invite the user from the console. If the user was the only Owner, mint
   one yourself (needs `b3sum` and `openssl`):
   ```sh
   TOKEN=$(openssl rand -base64 32 | tr '+/' '-_' | tr -d '=\n')      # 43 chars
   HASH=$(printf '%s' "$TOKEN" | b3sum --no-names)
   psql "$DATABASE_URL" -c "INSERT INTO invitations (account_id, email, role, expires_at, token_hash)
     VALUES ('<account_id>', '<email>', '<role>', now() + interval '7 days', decode('$HASH','hex'))"
   ```
   Give the user `<console>/admin/?invite=$TOKEN` over a trusted channel. They choose a
   new password, enrol a new authenticator and receive new recovery codes.
4. Their projects and data are tied to the account, not the user, and are unaffected.

## Incident: service down or not scraped

**Detect:** `EmbyrTargetDown` (`up{job="embyr-server"} == 0` for 1m) or `EmbyrMetricsAbsent`
(no `up` series for the job at all, or `embyr_pg_pool_idle{pool="system"}` missing for 5m).
While either fires, every other embyr alert is blind — treat as critical.

**Triage:**
1. Is the process up? `systemctl status embyr-server` / `docker ps`. If not, read the last
   log lines: a startup refusal prints `Error: ...` and exits 1 (missing/invalid env var,
   `EMBYR_REQUIRE_SMTP=true` without SMTP, unreachable `DATABASE_URL`).
2. Process up but target down: `curl -s -H "Authorization: Bearer $EMBYR_ADMIN_KEY"
   http://127.0.0.1:9090/metrics | head`. 401 → the scrape config's bearer token is stale
   (admin-key rotation). Connection refused/timeout → firewall or `:9090` binding.
3. `EmbyrMetricsAbsent` only: the target left the scrape config / service discovery, or the
   `job` name no longer matches the `job="embyr-server"` matcher in `alert-rules.yml`.

**Mitigate:** restart the service; if the newest release will not start, roll back to the
previous immutable image (`release-process.md` → Rollback Procedure).

## Incident: restart loop

**Detect:** `EmbyrRestartLoop` (`up` flipped more than 3 times in 15m). Cross-check the
supervisor's own count — `systemctl show embyr-server -p NRestarts` or
`docker inspect -f '{{.RestartCount}}' embyr-server` — since a loop faster than the scrape
interval can hide from Prometheus.

**Triage:** `journalctl -u embyr-server -n 200` (or `docker logs --tail 200`). Exit code 1
with an `Error:` line right after start = configuration/dependency refusal (same list as
above). A panic or OOM-kill (`dmesg | grep -i oom`, exit 137) after serving for a while =
runtime fault; compare memory against `resource-sizing.md`.

**Mitigate:** config refusal → fix the env file and restart. Regression in a new release →
roll back to the previous `:sha-<short sha>` image. Do not just raise the restart limit.

## Incident: readiness failing

**Detect:** there is no native metric for `/healthz`. Either the orchestrator/load balancer
reports the instance unready, or (if you deployed `blackbox_exporter` and enabled the
commented `EmbyrReadinessFailing` rule) `probe_success == 0` for 2m.

**Triage:** `curl -s http://127.0.0.1:8081/healthz`.
- `{"status":"draining"}` — the process received SIGTERM and is shutting down; it exits
  within `EMBYR_SHUTDOWN_GRACE_SECS`. Only a problem if nothing is starting in its place.
- `{"status":"unhealthy"}` — system Postgres is unreachable or slow (>3s): follow
  "Postgres down after startup" above.
- No response — the process is down: follow "service down or not scraped".

## Known limitations: real-time Listen in `backend_mode=agent`

Listen works in agent mode via the agent's `Subscribe` stream (see
`docs/feature/agent-mode-listen/feature-delta.md`). Three caveats:

- A delete reaches clients only where the collection's rule does not read the document
  (the event carries no pre-delete fields; rule-gated deletes are withheld, fail closed).
- Clients see a RESET after any agent-stream reconnect.
- `AgentNotifyBridge` has no reconnect of its own: after a stream drop the server
  reconnects and the agent opens a fresh Postgres connection.

## Security notes for operators

- **API keys in `?key=` query strings.** Firestore-compatible clients may send the project API
  key as `?key=<api-key>` (as the Google SDKs do). Any reverse proxy or load balancer in front
  of `:8080`/`:8081` MUST NOT log request query strings (e.g. strip `$args`/`$query_string` from
  nginx `log_format`, disable query logging on AWS ALB/GCP LB access logs, or redact `key=`),
  otherwise project API keys end up in your access logs.
- **Hosted-identity tokens are not revoked on password reset.** A hosted-identity ID token
  (1 hour lifetime) is stateless: resetting the user's password does not invalidate tokens
  already issued, they simply expire. Revoking them would need a per-user "valid after"
  timestamp checked on every request; not implemented. Admin (dashboard) password resets DO
  revoke all of that user's sessions.
- **`EMBYR_ADMIN_KEY` minimum length.** The server refuses to start if `EMBYR_ADMIN_KEY` is
  shorter than 32 characters.

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
