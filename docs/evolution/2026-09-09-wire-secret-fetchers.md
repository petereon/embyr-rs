# Evolution: wire-secret-fetchers

**Date:** 2026-09-09
**Feature:** Real AWS/GCP secret fetchers are now constructed at the composition root when
credentials are available, so `aws_secret`/`gcp_secret`-mode projects actually work end-to-end
instead of always failing with `Status::internal`.
**Job:** JOB-05 (`cloud-secret`) — reused, persona P3 Morgan (Tenant Admin-DevOps Lead).
**ADRs:** none new — reused/confirmed ADR-054 §D7's already-specified pattern.

## This closes finding #7 from `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

`main.rs` hardcoded `aws_secret_fetcher: None, gcp_secret_fetcher: None` at every construction
site — not because the mechanism was missing (`AwsSecretFetcher`/`GcpSecretFetcher` were fully
implemented and already proven end-to-end via test-only helpers), but because the real composition
root simply never constructed and wired them in. Every request for a project provisioned in
`aws_secret`/`gcp_secret` mode failed with `Status::internal`, even on a deployment with genuinely
valid AWS/GCP credentials available.

## Key Decisions

| Decision | Verdict |
|---|---|
| Construction asymmetry | AWS: unconditional construction (`aws_config::load_defaults` is infallible/lazy — credential resolution happens on first real API call, not construction). GCP: gated on `EMBYR_GCP_ACCESS_TOKEN` being present/non-empty (the fetcher's own constructor needs the token synchronously) |
| TTL | New `CLOUD_SECRET_FETCHER_TTL_SECS = 300`, distinct from the pre-existing `SECRET_FETCHER_TTL_SECS_UNUSED` (a different one-shot bootstrap use) — justified by AWS/GCP credential rotation cadence (hours-to-days) making 5-minute staleness acceptable while bounding API call volume |
| Scope | No global startup fail-fast — a deployment with neither cloud configured still starts normally; `direct_pg`-mode projects (the common case) unaffected. A provisioned `aws_secret`/`gcp_secret` project on a deployment lacking that cloud's credentials keeps failing closed exactly as before — locked as a non-regression, not new behavior |
| GCP base-URL override | Explicitly NOT added — `config.rs`'s own pre-existing comment cites a deliberate, already-decided prior exclusion (OQ-SM-4/ADR-018 Alternatives A6). Re-opening that would be scope creep into a different feature's own ADR |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with peer
review, DISTILL=`nw-acceptance-designer`, DELIVER=`nw-software-crafter`):

1. **DISCUSS**: reused JOB-05. Found a THIRD hardcoded-`None` site the audit's own citation
   missed (`transaction_sweeper::spawn`, not just `FirestoreService`/`build_admin_router`).
   Confirmed provisioning (`provision.rs`) already correctly gates on fetcher presence — no zombie
   projects possible today, fixing composition-root construction fixes both provisioning and live
   traffic at once. Found the exact real construction shape already proven in `lib.rs`'s test-only
   helpers. Found the AWS/GCP construction asymmetry by reading both constructors directly.
2. **DESIGN**: peer-reviewed (0 critical, 1 high/2 medium, all resolved inline — TTL justification,
   ADR-durability condition, GCP silent-`None` comment requirement). Confirmed the full blast
   radius: exactly 3 production edit points, zero test files need changing (verified every match
   individually, not just the first found, per this session's own established
   `stripe-webhook-secret-required` lesson about incomplete blast-radius greps).
3. **DISTILL**: wrote 6 acceptance scenarios spawning the REAL `main.rs` binary (not the test-only
   `lib.rs` helpers those existing `us_10`/`us_11` tests use — proving the actual composition-root
   fix, not just re-proving the already-proven mechanism). Confirmed correct RED. Flagged a genuine
   testability asymmetry: AWS respects the standard `AWS_ENDPOINT_URL` env var (zero
   production-code changes needed to redirect to LocalStack), but GCP's base URL has no override —
   a deliberate, already-documented prior decision, not something to fix here. Orchestrator
   accepted the wiring-only proof for GCP rather than reopening that prior ADR.
4. **DELIVER**: implemented exactly as designed. All 6 scenarios green, `us_10_aws_secrets.rs`/
   `us_11_gcp_secrets.rs` unmodified and green, clippy clean.
5. **Orchestrator's full-workspace regression**: hit another `cargo-sweep` mid-run interference
   (a sweep-down, not a full clean this time) — confirmed via `cargo-sweep.log` as environmental,
   resolved via isolated rerun of the one affected test.
6. **QUALITY_GATE**: scoped mutation run deliberately excluded the one network-dependent test
   (real egress to Google's own Secret Manager API) from the per-mutant loop to avoid hammering a
   live third-party API dozens of times. The resulting single miss (a GCP token-presence filter
   inversion) was investigated empirically — the mutation was manually reapplied and the excluded
   test run ONCE against it, confirming it reliably catches the bug — then reverted. A scoping
   artifact of the deliberate test-command exclusion, not a genuine gap.

## Lessons Learned

1. **A test tagged as requiring a real external dependency doesn't have to be excluded from
   mutation-testing coverage analysis entirely — it can be verified once, manually, against the
   specific mutant(s) its own exclusion would otherwise leave uncaught**, rather than either
   including it in a repeated automated loop (bad — hammers a live third-party API) or silently
   accepting reduced coverage without checking (bad — could hide a real gap). This is a reusable
   pattern for any future feature with a similarly-tagged `@requires_external` test.
2. **A blast-radius investigation should check not just "which existing tests break" but "which
   composition-root call sites the audit's own finding citation might have missed"** — DISCUSS
   found a third hardcoded-`None` site (`transaction_sweeper::spawn`) the audit's own two-location
   citation didn't name, by reading the actual code rather than trusting the finding's own location
   list as exhaustive.
3. **Not every design gap needs fixing in the feature that surfaces it** — the GCP base-URL
   override gap was real and DISTILL surfaced it clearly, but it was already a deliberate, prior,
   documented decision (a different ADR's own scope) — reopening it here would have been scope
   creep. Recognizing "this is a pre-existing, already-reasoned-through limitation" vs. "this is a
   new gap this feature should close" is a recurring, load-bearing judgment call this session has
   now made correctly multiple times (findings #2, #5, #7 all had this shape).

## Key Files

- `crates/embyr-server/src/main.rs` — the only production wiring file; new Step 8b construction
  block, threaded into all 3 sites.
- `crates/embyr-server/src/config.rs` — `CLOUD_SECRET_FETCHER_TTL_SECS` (new),
  `GCP_SECRET_MANAGER_BASE_URL` (bumped to `pub`).
- `tests/production_readiness/acceptance/pr09_wire_secret_fetchers.rs` (new) — 6 scenarios.
- `docs/feature/wire-secret-fetchers/deliver/mutation/mutation-report.md` — full account.

## Follow-Up Work

Finding #8 (no build/release path for embyr-agent) is the LAST remaining item from the original 8
Blockers. This closes the AWS/GCP secret-fetcher gap flagged as a follow-up candidate in both
`composite-index-real-creation`'s and `soft-delete-purge-sweeper`'s own evolution docs.
