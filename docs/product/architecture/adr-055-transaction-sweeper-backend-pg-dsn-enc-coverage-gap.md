# ADR-055: `backend_pg_dsn_enc IS NULL` Coverage Gap — Accept as Documented Scope

## Status

Accepted

## Context

`TransactionSweeper` resolves a `direct_pg` project's DSN via
`projects.backend_pg_dsn_enc` (AES-256-GCM under `EMBYR_ENCRYPTION_KEY`,
decrypted via `decrypt_with_rotation`) — never `ecies_encrypted_dsn`, since
that path structurally requires a live `api_key` the sweeper never holds
(ADR-054 § D3). `backend_pg_dsn_enc` is populated **only** when a project
owner explicitly submits `backend_pg_dsn` via `PATCH /admin/v1/projects/:id`
(`crates/embyr-server/src/admin/handlers/projects.rs::patch_project`) — never
at provisioning (`crates/embyr-server/src/admin/handlers/provision.rs:397`
writes `ecies_encrypted_dsn` only). Any `direct_pg` project that has never had
its DSN re-submitted this way has `backend_pg_dsn_enc IS NULL` and is
structurally unreachable by this sweeper.

DISCUSS escalated this as needing an explicit DESIGN sign-off, naming (without
independently verifying) that `admin-api-v2`'s own SDK-key-rotation feature
already accepted an identical gap for a different consumer.

### Precedent re-verified directly

`docs/product/architecture/adr-014-sdk-key-ecies-integration.md` (full,
re-read directly, not trusted from DISCUSS's citation alone) confirms the
claim precisely. ADR-014 § Consequences, "Known limitation — pre-existing
projects":

> Projects provisioned before admin-api-v2 (i.e., before this migration runs)
> have `backend_pg_dsn_enc = NULL`. They are permanently excluded from SDK key
> rotation DSN re-encryption. The handler detects this condition and skips
> step 7b silently (no error, no DSN update)... The only remediation is to
> re-provision the project (delete + recreate).

Same root cause exactly: the plaintext DSN only ever existed encrypted under
an api_key-derived ECIES key (`ecies_encrypted_dsn`); recovering it to
populate `backend_pg_dsn_enc` requires either the original plaintext api_key
(never stored, by design — only its Argon2id hash is) or an owner's active,
independent re-submission. `docs/evolution/2026-08-07-admin-api-v2.md`
(§ Retrospective item 5) further confirms this was a DISTILL-wave-caught
edge case with a dedicated acceptance test
(`pre_existing_project_with_null_dsn_enc_skips_re_encryption_silently`), not
an oversight.

## Decision

**Accept the gap as documented scope — no backfill, no forced DSN
re-submission prompt built in this feature.** A `direct_pg` project with
`backend_pg_dsn_enc IS NULL` is silently, permanently excluded from both
reclaim (Slice 01) and purge (Slice 02), indefinitely, until (and unless) its
owner independently PATCHes a DSN for an unrelated reason (e.g. a future SDK
key rotation, ADR-014's own consumer).

**Named, not hidden**: `/metrics`' two counters measure reclaim/purge
ACTIVITY, not COVERAGE. A project this sweeper can never reach produces zero
counter activity, indistinguishable from a project with genuinely zero
orphaned rows. Sam Chen (the operator persona this feature serves) has no
signal from `/metrics` alone distinguishing the two cases.

## Rationale

1. **Direct precedent, not analogy.** ADR-014 already shipped the identical
   accept-the-gap resolution for a different consumer (SDK key rotation) of
   the SAME structurally-unreachable-without-out-of-band-resubmission root
   cause. A second consumer inventing a DIFFERENT resolution (e.g., a
   backfill flow) would fragment this codebase's own established answer to
   one problem into two inconsistent policies for the identical root cause.
2. **Severity inheritance.** DISCUSS's own framing (feature-delta.md line 5)
   already established this whole feature as low-severity, pure
   storage-growth mitigation, not a correctness or security fix. The
   coverage gap inherits that severity: an unreclaimed orphaned row in an
   already-small bookkeeping table, on a project nobody has touched since
   before `admin-api-v2` shipped, is a strictly smaller problem than the one
   this feature exists to fix.
3. **Cost asymmetry and right-sizing.** A real remediation (an admin action
   prompting affected owners to re-submit their DSN, or an operator-facing
   report of permanently-unreachable projects) is realistically its own
   right-sized feature — its own DISCUSS, its own UAT scenarios, its own
   operator UX — not a rider on a 2.5-day, 2-slice sweeper. DISCUSS's own
   Elephant Carpaccio gate already scored this feature at exactly its right
   size (0 of 5 signals fired) partly because this residual scope was
   excluded; reopening it here would retroactively invalidate that sizing.

## Alternatives Considered

1. **Build a one-time backfill admin action now.** Rejected — cost asymmetry
   above, and structurally impossible to fully backfill regardless: the
   plaintext DSN is not recoverable server-side without either the original
   api_key or an owner's active re-submission (the same "chicken and egg"
   ADR-014 itself already named).
2. **Add a new Prometheus gauge counting `direct_pg` projects with
   `backend_pg_dsn_enc IS NULL`** — a lightweight partial mitigation giving
   Sam Chen COVERAGE visibility, not just activity. Rejected for v1 as
   unrequested scope beyond DISCUSS's own explicitly-sized two counters
   (DISCUSS Resolution 2 already drew this exact line: "cheap, reuses an
   already-installed mechanism... not escalated"). Named below as a
   candidate follow-up.

## Named Follow-Up (not built now)

A one-time admin-facing report/action surfacing "which `direct_pg` projects
have `backend_pg_dsn_enc IS NULL`" — a superset of both this feature's own
gap and ADR-014's own SDK-key-rotation gap, since they share the identical
root cause and could be remediated by the same "prompt owner to re-submit DSN
via the existing `PATCH` endpoint" mechanism. Flagged as a candidate single
future feature covering both consumers at once, not two separate patches.

## Consequences

**Positive**: zero new code, zero new schema, zero contradiction with an
already-shipped, already-accepted precedent for the identical root cause.

**Negative**: Sam Chen has no `/metrics`-only signal distinguishing "zero
orphaned rows" from "permanently unreachable by this sweeper" — named
explicitly here as a known, accepted UX gap, not silently absorbed into the
feature's own Outcome KPIs (which measure activity, per DISCUSS, unchanged).
