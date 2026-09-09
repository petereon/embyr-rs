# Evolution: stripe-webhook-body-limit

**Date:** 2026-09-09
**Feature:** Stripe webhook route bounds pre-signature request-body buffering to 5 MiB, returning
413 on overflow instead of allocating an unbounded amount of memory before the HMAC is even
checked.
**Job:** JOB-13 (`production-deployment`) — reused, persona P2 Sam Chen.
**ADRs:** ADR-070 (new) — Stripe webhook body-size ceiling.

## This closes finding #3 from `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

`axum::body::to_bytes(body, usize::MAX)` in `stripe_signature_middleware` buffered the ENTIRE
request body into memory before the `Stripe-Signature` HMAC was checked, on an (at the time)
unauthenticated code path. An attacker could POST an arbitrarily large body to
`POST /admin/v1/webhooks/stripe` and force unbounded server-side memory allocation with zero
authentication. Same route as already-closed finding #1 (forgeable webhook secret) — this fix
layers a size ceiling onto that route without touching its conditional-mount logic.

## Key Decisions

| Decision | Verdict |
|---|---|
| D1 | 5 MiB byte ceiling, hardcoded (not operator-configurable) — a security invariant, not a knob. Stripe's real payloads run KB to low hundreds of KB; 5 MiB is 10-50x generous headroom |
| D2 | Discriminate `LengthLimitError` → 413 Payload Too Large from every other `to_bytes`-style error → 401 Unauthorized (unchanged) |
| D3 | Narrow fix confined to this one route — confirmed via grep that no other unauthenticated route in `admin/` does manual pre-auth body buffering; every other admin route sits behind auth middleware already |
| D4 (DELIVER, root-cause-driven deviation from ADR-070's illustrative snippet) | Manual `Limited`-driven drain-and-discard loop instead of a bare `axum::body::to_bytes(body, limit)` call — the latter stops polling the connection the instant the limit is exceeded, which for a grossly-oversized attacker payload leaves the client mid-write when the server closes the connection, producing a TCP RST instead of a clean 413 response. Reproduced in an isolated minimal axum app to confirm before deviating. Same dependency, same 413/401 contract ADR-070 specifies — a correction of an illustrative snippet's own gap in ADR-070, not a re-decision of the locked design |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with peer
review, DISTILL=`nw-acceptance-designer`, DELIVER=`nw-software-crafter`):

1. **DISCUSS**: reused JOB-13, locked narrow-scope decision, wrote 5 ACs (AC-WBL-01 through 05),
   with AC-WBL-02 ("a legitimate Stripe webhook must always still succeed") called out as the
   single non-negotiable constraint, repeated across the story. DoR 9/9.
2. **DESIGN**: peer-reviewed, 0 critical/high findings. Confirmed via grep that `tower-http` (and
   its `RequestBodyLimitLayer`) is not in this workspace at all — rejected adding it in favor of
   reusing `http_body_util::Limited`/`LengthLimitError`, both already direct dependencies. Picked
   5 MiB and the 413/401 discrimination scheme. Wrote ADR-070.
3. **DISTILL**: wrote `tests/production_readiness/acceptance/pr07_stripe_webhook_body_limit.rs`
   (4 tests). AC-WBL-01's oversized-body test specifically measures RSS delta via
   `/proc/<pid>/status` `VmRSS`, not just the status code — proving memory actually stays bounded,
   not merely that the response is 413. Confirmed correct RED (401 today, not 413) against the
   unfixed code before handoff.
4. **DELIVER**: implemented the ADR-070 mechanism, hit a real transport-level failure
   (`ConnectionReset`) with the naive one-liner under the acceptance test's own attacker-scale
   (50 MiB) case, root-caused it against `http-body-util`'s own source rather than patching around
   the symptom, and shipped the drain-and-discard variant instead. All 4 `pr07` tests green,
   `#[ignore]` removed (single-story feature, no reason to gate behind opt-in). Regression guards
   `pr06_stripe_webhook_secret_required` (4/4) and `card_payments_backend_cpb03_webhook_ingestion`
   (8/8) confirmed unmodified and green.
5. **Orchestrator's full-workspace regression** (run before QUALITY_GATE, given this touches a
   security-sensitive shared middleware): 0 failures across every test binary.
6. **QUALITY_GATE**: scoped `--in-diff` mutation run (`ps -p`-verified clean before touching the
   file). 7 mutants, **7/7 caught, 0 missed, 0 unviable** — the cleanest mutation result of this
   session's 3 blocker fixes so far. The single most security-critical mutant (`delete ! in
   read_body_bounded`, the guard that stops appending bytes past the limit) was caught by the
   RSS-delta assertion DISCUSS insisted on, not a status-code check alone.

## Lessons Learned

1. **A design doc's illustrative code snippet is not gospel — verify it against the actual
   library behavior before shipping it verbatim, especially for attacker-scale inputs the design
   phase didn't concretely simulate.** ADR-070's `axum::body::to_bytes(body, limit)` one-liner is
   textbook-correct for a *moderately* oversized body, but breaks down at the acceptance test's
   own attacker-scale (50 MiB vs. 5 MiB limit) case because of how `http_body_util::Limited`
   actually stops polling. DELIVER caught this by actually running the acceptance test against
   the naive implementation first, rather than trusting the snippet compiled and moving on.
2. **When a design snippet doesn't survive contact with a real test, reproduce the failure in
   isolation before deviating from the locked design** — DELIVER built a 20-line minimal
   axum/hyper repro outside the workspace to confirm the `ConnectionReset` was a genuine library
   behavior, not an embyr-server-specific bug, before committing to the drain-and-discard
   alternative. This is the same root-cause-before-patching discipline this session's `CLAUDE.md`
   and `ponytail` skill both already mandate, applied to a design-level (not just code-level)
   surprise.
3. **A test that measures the actual security property (RSS delta) rather than just the
   surface-level symptom (status code) catches mutants a status-code-only test would miss** — the
   `!too_large` guard deletion mutant is exactly the kind of gap a naive "assert 413" test
   wouldn't have caught (a buggy implementation could still return 413 while having already
   buffered the whole oversized body). Reinforces DISCUSS's own insistence on the RSS-measurement
   AC wording rather than accepting a simpler test.

## Key Files

- `crates/embyr-server/src/admin/middleware/stripe_signature.rs` — the only production file
  changed; `MAX_STRIPE_WEBHOOK_BODY_BYTES` constant, `read_body_bounded` helper.
- `docs/product/architecture/adr-070-stripe-webhook-body-size-ceiling.md` (new).
- `tests/production_readiness/acceptance/pr07_stripe_webhook_body_limit.rs` (new) — 4 tests:
  `oversized_body_rejected_with_413_and_bounded_memory_growth`,
  `legitimate_normally_sized_webhook_succeeds_end_to_end`,
  `correctly_signed_webhook_comfortably_within_ceiling_still_succeeds`,
  `oversized_unsigned_request_never_reaches_signature_verification_or_db`.
- `tests/production_readiness/common/mod.rs` — `read_rss_kb`, `sign_stripe_payload` helpers.
- `docs/feature/stripe-webhook-body-limit/deliver/mutation/mutation-report.md` — full account.

## Follow-Up Work

Findings #4-#8 from the same audit remain (LISTEN/NOTIFY listener leak + no reconnect; fake
composite-index creation; missing 168h soft-delete purge sweeper; hardcoded-None aws/gcp secret
fetchers; no build/release path for embyr-agent). This is the last finding on the Stripe webhook
route specifically — findings #1, #2 (metric cardinality, different file), and #3 are all now
closed; no more warm-context work remains on this particular route.
