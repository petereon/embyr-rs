# Slice 02: Embyr Verifies Customer Database Readiness and Complains Clearly When It Isn't

**Story**: US-02 | **Release**: 1 | **job_id**: JOB-15

## Goal
Give the operator (or a self-serve customer) a specific, actionable message when a submitted database isn't properly prepped, instead of a generic `backend_unavailable` 400 or a failure deferred to the first real document read/write — and let a DML-only connection string provision successfully when the database genuinely is ready.

## IN Scope
- `POST /admin/v1/projects` (`backend_mode=direct_pg`) verifies the submitted database's schema state before completing provisioning.
- Distinct, actionable responses for: fully prepped (proceeds normally), not prepped at all, and partially prepped / stale schema version.
- No regression to today's default path: existing full-privilege-DSN provisioning must continue to succeed unchanged.

## OUT Scope
- The exact verification mechanism (schema introspection query, `_sqlx_migrations` table check, stored marker, etc.) — DESIGN's call.
- WHEN the check runs beyond provisioning-time (connection-pool checkout, periodic re-check / drift detection) — explicitly deferred, candidate follow-up feature (see feature-delta § Out of Scope, Job Discovery Framing Resolution option (c)).
- `aws_secret` / `gcp_secret` / `agent` backend modes (see feature-delta § Out of Scope).

## Learning Hypothesis
**Disproves if it fails**: "A DML-only connection string can be distinguished, at provisioning time, from 'not yet prepped' purely by observable schema state — without embyr's provisioning flow needing (or attempting) any elevated privilege on the submitted connection string itself."
**Confirms if it succeeds**: `SystemDb::probe()`'s hard-gate schema-verification pattern (`crates/embyr-server/src/adapters/system_db.rs`) generalizes cleanly to a per-customer-DB check at provisioning time, without requiring DDL rights on the customer connection.

## Acceptance Criteria
- [ ] AC-02-01: Provisioning with a DML-only connection string against a database Slice 01's tool has already fully prepped succeeds, returning the same `{"project_id", "api_key"}` shape as any other successful `direct_pg` provisioning today.
- [ ] AC-02-02: Provisioning against a database with no embyr schema present at all fails with a 400 naming what is missing (e.g., which expected table was not found) — not a generic `backend_unavailable`.
- [ ] AC-02-03: Provisioning against a database prepped with an older schema version than currently expected fails with a 400 naming the expected version and the version found.
- [ ] AC-02-04: The failure response is distinguishable, in both status handling and message content, from a plain connectivity failure (the database is reachable but not ready, vs. unreachable).
- [ ] AC-02-05: Existing full-privilege-DSN `direct_pg` provisioning (today's default path, not DBA-gated) continues to succeed unchanged — no regression.

## Dependencies
Depends on Slice 01 (US-01) existing conceptually as the counterpart that produces "prepped" databases — but is independently testable against any database in a known schema state, real or hand-seeded, and does not require Slice 01 to ship first.

## Effort Estimate
1.5 days.

## Reference Class
`SystemDb::probe()` (`crates/embyr-server/src/adapters/system_db.rs`) — the existing hard-gate schema-verification-at-startup pattern for the *system* database (referenced by JOB-13); this slice is the analogous concept for *customer* databases at provisioning time.

## Pre-Slice SPIKE
Not required — `SystemDb::probe()` is a proven, working reference pattern to adapt; no new unproven mechanism is introduced at the DISCUSS-wave level of specification.
