# Slice 07 — Free-Cap-Exceeded Suspension Enforced in Real Time

**Feature:** card-payments-backend
**Slice:** 07 of 07 | **Release:** 3 (Real-Time Cap Enforcement)
**Estimate:** 1 day
**Stories:** US-207
**Depends on:** Slice 06 (the computation this trigger reads), Slice 04 (reuses the same suspend call site pattern)

---

## Goal

Crossing ≥100% of any Free-plan dimension cap (per Slice 06's computation) suspends every project under the account, via the identical `set_project_status` path Slice 04's dunning trigger uses (D-12: one mechanism, two triggers).

## Learning Hypothesis

Disproves: "Wiring the cap-exceeded trigger to `suspend_project()` is a drop-in reuse of Slice 04's pattern" — this slice exists specifically to surface any race condition or ordering issue that fires from the synchronous request hot path (unlike Slice 04's async webhook handler context), which Slice 04 alone would not have caught.
Confirms if succeeds: The suspend mechanism is genuinely trigger-agnostic — safe to call from both an async webhook handler and a synchronous request-path check with no behavioral divergence.

## IN Scope

- Crossing ≥100% on any Free-plan dimension (per Slice 06) suspends all of the account's projects
- Below-100% usage never triggers suspension via this path
- Pro-plan accounts are never suspended by this mechanism (overage is billed instead, Slice 05)
- Calls the identical `set_project_status` function as Slice 04 — verified directly (shared test helper or code-path assertion), not assumed
- Upgrading to Pro (Slice 02) reactivates projects suspended by this mechanism — verified end-to-end from the enforcement side

## OUT Scope

- Any new suspension UI/messaging — the shipped `SuspensionBanner` (US-108, `card-payments` frontend) already handles display; this slice only makes the underlying state real

## Acceptance Criteria

- AC-207-01 through AC-207-05 (see feature-delta.md US-207)

## Dependencies

- Slice 06 complete (the cap_status computation)
- Slice 04 complete (the suspend call-site pattern being reused)

## Effort Estimate

1 day. Reference class: the suspend action itself (`set_project_status`) is fully reused from Slice 04/`lifecycle.rs`; the new work is exclusively the trigger-wiring and the request-hot-path race-condition verification named in the Learning Hypothesis.
