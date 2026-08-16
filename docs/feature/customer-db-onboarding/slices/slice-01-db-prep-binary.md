# Slice 01: Customer-Run Database Preparation Step (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **job_id**: JOB-15

## Goal
Let a customer's DBA apply the exact schema embyr requires against their own Postgres, under their own elevated credentials, without ever handing embyr's SaaS a DDL-capable connection string.

## IN Scope
- A customer-operated entry point (exact packaging/CLI shape is DESIGN's call) that applies the same migration set `embyr-server` embeds at `migrations/customer/`.
- Idempotent re-run: partial/interrupted prior runs resume safely; already-current databases report "already up to date."
- Distinct, actionable reporting for: success (with schema version applied), insufficient privilege, and connection failure — three different messages, not one generic error.

## OUT Scope
- The server-side verification/complaint behavior (Slice 02).
- Distribution/release packaging mechanics of the tool itself.
- `aws_secret` / `gcp_secret` / `agent` backend modes (this slice targets the `direct_pg`-mode schema set only; see feature-delta § Out of Scope).
- Any UI — this is a command-line, log-reading interaction (Decision 3: lightweight UX depth).

## Learning Hypothesis
**Disproves if it fails**: "A standalone tool, run outside `embyr-server`'s own process, cannot reliably apply and idempotently re-apply the same migration set embyr-server embeds, without drifting from what embyr-server itself would have produced."
**Confirms if it succeeds**: The exact migration set in `migrations/customer/` can be packaged and run standalone, safely re-run, and produces byte-identical schema state to what `provision.rs`'s in-process `sqlx::migrate!` call produces today.

## Acceptance Criteria
- [ ] AC-01-01: Running the tool against a fresh, empty, reachable Postgres database (with sufficient privilege) applies the full migration set and reports success with the schema version applied.
- [ ] AC-01-02: Re-running the tool against a database it already fully prepared reports "already up to date" and makes no further schema changes.
- [ ] AC-01-03: Re-running the tool after an interrupted partial run resumes from the last successfully-applied migration and completes without duplicate-object errors.
- [ ] AC-01-04: Running the tool with a login that lacks required schema-modification privilege on the target database reports a specific message naming the missing privilege and the target database — not a raw driver error.
- [ ] AC-01-05: Running the tool against an unreachable Postgres host reports a connection failure, distinguishable from a privilege failure.

## Dependencies
None — `migrations/customer/` already exists (`0001_documents.sql`, `0002_transactions.sql`) and is the shared source of truth this slice packages.

## Effort Estimate
1.5 days.

## Reference Class
`embyr-agent`'s own startup migration behavior (AD-A08, `docs/product/architecture/brief.md`) — closest existing precedent for "a binary applies `migrations/customer/` outside `embyr-server`'s own process," though for a different actor (the agent itself, not a customer DBA) and a different trigger (agent startup, not a one-time pre-flight step).

## Pre-Slice SPIKE
Not required — reuse of an already-proven migration mechanism (`sqlx::migrate!` against `migrations/customer/`), low uncertainty.
