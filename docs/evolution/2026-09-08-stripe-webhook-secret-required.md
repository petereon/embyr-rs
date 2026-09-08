# Evolution: stripe-webhook-secret-required

**Date:** 2026-09-08/09
**Feature:** Server now refuses to start with a forgeable Stripe webhook (empty/absent signing
secret) instead of silently accepting an unauthenticated attacker's HMAC.
**Job:** JOB-13 (`production-deployment`) — reused, persona P2 Sam Chen.
**ADRs:** none new — decisions embedded in this feature's own `feature-delta.md`.

## This closes finding #1 from `docs/product/production-readiness-audit-2026-09-08.md` — the sharpest of 8 blockers

## Business Context

`STRIPE_WEBHOOK_SIGNING_SECRET` defaulted to an empty string when unset. An empty HMAC key is a
valid HMAC key — so the webhook route stayed mounted and would accept a signature computed with
that same empty key, which any unauthenticated attacker can also compute. The handler then
executed `UPDATE subscriptions SET status = ... WHERE stripe_subscription_id = $1` based on that
forged, attacker-controlled input. This is not a theoretical risk — it's a directly exploitable,
unauthenticated write path into real billing state.

## Key Decisions

| Decision | Verdict |
|---|---|
| D1 | Narrow, not global fail-fast — `STRIPE_SECRET_KEY` present + `STRIPE_WEBHOOK_SIGNING_SECRET` absent/empty → refuse to start. Stripe fully unconfigured (a legitimate BYOC/self-hosted shape) → server starts normally |
| D2 | Webhook route is not *mounted* at all when Stripe is unconfigured (the true root-cause fix, at the composition root) — not mounted-but-rejects-in-handler |
| D3 | A GitHub-Actions-style empty-string secret is treated identically to "unset" — a configured-but-absent repo secret is `""`, not omitted, and must not silently escape the fail-fast check |

## Steps Completed

Run through the full nWave subagent pipeline (DISCUSS=`nw-product-owner`,
DESIGN=`nw-solution-architect`, DISTILL=`nw-acceptance-designer`, DELIVER=`nw-software-crafter`)
— a genuinely thorough and eventful delivery worth recounting honestly, not glossing over:

1. **DISCUSS**: locked the narrow-not-global fail-fast rule, reusing JOB-13.
2. **DESIGN**: chose not-mounting over mount-but-reject as the true root-cause fix, and found a
   real CI landmine along the way — the shared test-subprocess helper (`ServerProcess::start()`)
   didn't `env_clear`, so it silently inherited CI's own job-level `STRIPE_SECRET_KEY`, which
   would have broken 22 unrelated test call sites the moment fail-fast validation landed.
3. **DISTILL**: fixed the CI landmine directly, wrote 2 acceptance tests (AC-WHS-01/02),
   confirmed correct RED state.
4. **DELIVER**: implemented the fix exactly as designed. All tests passed, all named regression
   guards passed.
5. **Orchestrator's own blast-radius completion**: the first full-workspace regression run (run
   before QUALITY_GATE, per this feature's own decision to run full — not scoped — regression
   given the stakes) caught 7 unrelated test binaries failing to *compile*. DESIGN's own
   blast-radius grep for `build_admin_router` call sites had missed 6 shared test-context helper
   files (outside `card_payments_backend`) using the identical `String::new()` call shape.
   Fixed directly: `String::new()` → `None` in all 6.
6. **Orchestrator's own 2-real-gap mutation-testing catch-and-fix**: QUALITY_GATE found 2
   genuine acceptance-test gaps in the security-critical logic (see mutation report for full
   detail) and fixed both with new tests.
7. **A procedural detour**: mid-QUALITY_GATE, a manual `git checkout --` to fix a leftover
   corrupted file raced a still-live `cargo-mutants` process, invalidating an entire retest —
   discarded and rerun cleanly. See Lessons Learned.

## Lessons Learned

1. **A configured-but-absent GitHub Actions repo secret is `""`, not "unset."** Any future
   "is this optional secret actually configured" check in this codebase must filter empty
   strings, not just check `Option::is_some()` — reused directly from the TLS pair's own
   established pattern, and now itself a second, load-bearing precedent for a third feature.
2. **A shared test-subprocess helper that doesn't `env_clear` silently inherits CI's own
   job-level environment variables.** This class of bug is easy to miss because it only
   manifests in CI, not locally — worth a proactive grep-check ("does any other shared test
   helper have the same gap for some OTHER secret?") as genuine follow-up work, not just this
   one instance.
3. **When changing a widely-shared function's own signature, grep for the SPECIFIC pattern
   being changed, not just the function name.** `build_admin_router` has 13+ call sites across
   the test suite; DESIGN's own grep for the function name should have found all of them, but
   the actual miss was subtler — 6 test-context helpers matched the exact textual call shape
   (`stripe_gateway, String::new(),`) that DESIGN's own blast-radius check apparently didn't
   verify against exhaustively. A full-workspace regression run (not just a scoped check) before
   QUALITY_GATE is what caught this — reinforces running the full suite for any composition-root
   signature change, not a scoped check.
4. **`cargo-mutants`' default invocation skips `#[ignore]`d tests — now confirmed twice this
   session as a fully-established interaction**, not a one-off. The fix (a name-filtered
   `--ignored <feature-substring>` retest, never a bare `--ignored`) is now a settled technique
   for this codebase.
5. **A fail-fast/conditional-mount feature's acceptance tests must independently prove BOTH the
   negative cases (absent, empty-string) AND the positive case (correctly configured, actually
   works) — and the positive case specifically THROUGH THE REAL PRODUCTION ENTRY POINT
   (`main.rs`), not just via an unrelated regression-guard test that happens to configure the
   same values through a more direct code path.** This is the single most broadly valuable
   finding of this feature: it's a general QUALITY_GATE discipline for any future fail-fast or
   conditional-mount feature in this codebase, not specific to Stripe.
6. **Never touch a file a live `cargo-mutants --in-place` process is still scoped to.** A
   distinct, previously-undocumented hazard from the already-known cargo-sweep external
   corruption: manually restoring a file mid-run (even for a genuinely correct reason) races
   `cargo-mutants`' own mutate/test/restore cycle unpredictably. Always confirm the process has
   genuinely exited (`ps -p <PID>`) — not just that its own log output "looks done" — before any
   manual git operation on its scoped files.

## Key Files

- `crates/embyr-server/src/config.rs` — fail-fast validation, reuses existing `MissingVars`.
- `crates/embyr-server/src/main.rs` — removed `.unwrap_or_default()`, threads `Option<String>`.
- `crates/embyr-server/src/admin/router.rs` — conditional webhook-router construction/merge.
- `tests/production_readiness/common/mod.rs` — CI-env-leak fix (`env_remove`).
- `tests/card_payments_backend/common/mod.rs`, `tests/admin_api_v2/common/mod.rs`,
  `tests/client_auth_hosted_identity/common/mod.rs`, `tests/anonymous_sessions/common/mod.rs`,
  `tests/client_auth/common/mod.rs`, `tests/oauth_providers/common/mod.rs`,
  `tests/security_rules/common/mod.rs` — blast-radius completion.
- `tests/production_readiness/acceptance/pr06_stripe_webhook_secret_required.rs` (new) — 4 tests:
  `webhook_route_not_reachable_when_stripe_fully_unconfigured`,
  `exits_nonzero_when_stripe_secret_key_set_without_webhook_signing_secret`,
  `exits_nonzero_when_stripe_webhook_signing_secret_is_empty_string`,
  `webhook_route_is_reachable_when_stripe_correctly_configured`.
- `docs/feature/stripe-webhook-secret-required/deliver/mutation/mutation-report.md` — full
  account of the QUALITY_GATE journey.

## Follow-Up Work

Findings #2 and #3 from the same production-readiness audit are on the exact same webhook
route this feature just touched (unbounded Prometheus label cardinality, unbounded request-body
buffering) — good next targets with warm context.
