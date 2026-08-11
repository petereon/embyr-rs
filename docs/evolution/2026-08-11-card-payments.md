# Evolution: card-payments

**Date:** 2026-08-11
**Feature:** Self-service billing-management UI for `embyr-admin-ui` (plan visibility, usage-cap warnings, card capture, plan change, invoice history, suspension recovery)
**Job:** JOB-14 (`manage-subscription`, new — extends the umbrella JOB-10 `account-admin` without modifying it)
**ADR:** ADR-019 (`docs/product/architecture/adr-019-billing-modal-global-state.md`)

## Business Context

`crates/embyr-admin-ui/src/views/billing.rs` showed nothing but placeholder "—" dashes — no plan
visibility, no card-add path anywhere in the console, and the Free-plan hard-stop (no auto-upgrade,
per D-6) gave zero warning before suspending writes. Persona: Chris Okafor (P5 — Account Admin /
Platform Engineer), with Dana Whitfield (Pro plan) and Priya Raman (suspended) as secondary
examples. JOB-14's opportunity score (17) was the highest of any job scored to date, driven by
D-5's card-required-even-for-Free policy making this a hard blocker for every account, not an
optional nicety.

### Scope Assessment gate (DISCUSS)

The feature as originally framed spanned 4 bounded contexts (billing-domain, webhook-ingestion,
rate-limiting, admin-UI) and fired 5 of 5 oversized signals (>10 stories, >3 modules, >5 WS
integration points, >2 weeks effort, multiple independent shippable outcomes). It was split,
mirroring this project's own `user-admin-ui` → `admin-api-v2` precedent:

- **`card-payments` (this feature)**: frontend billing UI only, mock data (ADR-007), 9 stories, 1
  bounded context.
- **`card-payments-backend` (recommended follow-up, not executed)**: Stripe SDK integration,
  `subscriptions`/`processed_webhook_events` schema, 5th webhook sub-router, rate-limiter extension
  for real-time Free-cap enforcement, daily batch job, dunning-triggered suspension wiring.

This feature's `AppModel.subscription`/`Card`/`Invoice` mock types are the forward cross-crate type
contract `card-payments-backend`'s real handlers must match, per the same ADR-007 migration
guarantee already proven for `admin-api-v2`.

## What Shipped (9 user stories, 8 Elephant Carpaccio slices, all traced to JOB-14)

| Story | Slice | Description |
|-------|-------|-------------|
| US-101 (Walking Skeleton) | 01 | Plan card + Payment Method card + Overview/Usage/Invoices tab shell |
| US-102 | 02 | Cap Usage card — 4 per-dimension bars, accent/amber(≥80%)/red(≥100%), expandable per-database breakdown |
| US-103 | 03 | Next Invoice card (Pro-only) — itemized overage projection, mid-cycle "Estimated" framing |
| US-104 | 03 | Real per-database Usage table (reads/writes/deletes/storage), 3-color stacked bar, 5 KPI tiles — supersedes the old placeholder table |
| US-105 | 04 | CardModal — Rust-native card capture form, brand auto-detection, replace-not-append semantics, PCI SAQ-A reassurance copy |
| US-106 | 05 | UpgradeModal — compare→confirm two-step flow, mandatory downgrade warning |
| US-107 | 06 | Invoice history table, PDF-download affordance, Free-plan empty state |
| US-108 | 07 | `SuspensionBanner` — cross-cutting (renders above all `Section` content in `ShellView`), amber/red state, CTA routes to the correct modal |
| US-109 | 08 | `TestClockCard` dev-only payment-failure toggle for verifying the recovery flow without a real Stripe webhook |

Card/Stripe backend integration (webhooks, real payment processing, real-time rate-limiter
enforcement) was explicitly **deferred** to `card-payments-backend` — not built in this feature.

## Key Decisions (ADR-019 + DESIGN wave-decisions)

- **ADR-019 — CardModal/UpgradeModal open-state is global `AppModel` state, not view-local.**
  Diverges from `db_detail`'s local-`RwSignal` confirm-modal precedent because `SuspensionBanner`
  is cross-cutting and must open either modal directly regardless of the currently active
  `Section` (AC-108-05) — a case ADR-006's context-provided `AppModel`/`Msg` pattern exists to
  solve, and non-obvious/precedent-diverging enough to warrant its own ADR.
- `capExceeded`/`effectiveStatus`/`readOnly` implemented as pure `impl AppModel` methods
  (`usage_totals()`, `cap_ratios()`, `cap_exceeded()`, `effective_status()`, `read_only()`) —
  computed on every read, never stored as duplicated booleans. This is D-6/D-7's hard-stop logic
  and the single highest-consequence defect the design structurally prevents.
- `Database` struct extended with `usage: UsageStats` rather than a new parallel aggregate — one
  source of truth consumed identically by `CapUsageCard`, `NextInvoiceCard`, and the Usage tab.
- `Segmented` is a new primitive (value-selection semantics, distinct from `Tabs`'
  content-navigation semantics); progress/cap bars were deliberately **not** extracted as shared
  primitives — single consumer each in V1 (YAGNI).
- `views/billing/{mod,overview,usage,invoices,modals}.rs` mirrors the existing `views/db_detail/`
  multi-file subdirectory precedent exactly — no new structural pattern.
- `views/billing.rs` (127-line V1 placeholder) deleted, content ported into `usage.rs`/`mod.rs` —
  the file's own doc comment ("V2 plan: replace with `#[server]` function") anticipated this
  replacement.
- CardModal/UpgradeModal remain Rust-native form/validation only in V1 — no Stripe.js/Elements JS
  interop shim (D-3 scope boundary, forward-flagged for `card-payments-backend`).
- Reuse verdict: 10 EXTEND, 1 REPLACE, 1 PATTERN REUSE, 2 CREATE NEW (`Segmented` primitive; the
  `views/billing/` file set itself), 1 explicit NOT APPLICABLE (`Toggle`), 1 explicit NOT EXTRACTED
  (bar primitives). Zero unjustified CREATE NEW decisions.

## Steps Completed

All 15 DELIVER roadmap steps (`01-01` through `09-02`) show complete
`PREPARE → RED_ACCEPTANCE → GREEN → COMMIT` DES traces in `execution-log.json`
(`des-verify-integrity docs/feature/card-payments/deliver/` → exit 0, "All 15 steps have complete
DES traces"). 13 steps were originally planned; 2 were orchestrator-added closing steps:

| Step | Name | Status |
|------|------|--------|
| 01-01 | Walking skeleton — plan/payment cards, tab shell, modal wiring | PASS |
| 02-01 | `bar_color` threshold function + trash icon | PASS |
| 02-02 | `usage_totals` + `cap_ratios` derivation | PASS |
| 02-03 | `cap_exceeded` + per-database usage rows | PASS |
| 03-01 | `detect_card_brand` + `card_number_is_complete` | PASS |
| 03-02 | `Msg::SetCard` replace-not-append | PASS |
| 04-01 | `UpgradeModal` compare/downgrade-warning projection | PASS |
| 04-02 | `Msg::SetPlan` confirm mutation + suspension-clearing | PASS |
| 05-01 | `usage_table_rows` real per-database Usage tab | PASS |
| 05-02 | Next invoice estimate overage projection | PASS |
| 06-01 | `Msg::SetInvoices` wholesale replacement | PASS |
| 07-01 | Suspension banner + `read_only` derivation | PASS |
| 08-01 | Test mode simulator toggle | PASS |
| 09-01 | Closed orphaned-scaffold gap in mock billing/invoice data (found via post-implementation scaffold-marker grep) | PASS |
| 09-02 | Closed real Usage/Invoices tab rendering gap (found by Phase 4 adversarial review) | PASS |

## Scenarios: 65/65 green, 0 ignored

`cargo test -p embyr-admin-ui --test card_payments_walking_skeleton` → 6/6;
`cargo test -p embyr-admin-ui --test card_payments_tea_state` → 59/59. Zero `#[ignore]` markers
remain in `tests/card_payments/`. Sibling `user-admin-ui` suites unregressed
(`user_admin_ui_walking_skeleton` 2/2, `user_admin_ui_tea_state` 64 passed/25 pre-existing ignores).

## Quality Gates

- **Roadmap review (Phase 1):** APPROVED, 0 blockers reported by `nw-acceptance-designer-reviewer`
  — but 2 real bugs were found by the orchestrator *after* that approval, by reading the actual
  test files: a test assigned to a step before its dependency was implemented, and a backward
  method-call dependency between two originally-separate steps (`suspension_banner_view()`
  scheduled before `effective_status()`, which it internally calls). Both fixed directly in
  `roadmap.json` before dispatch. Neither was caught by the automated review.
- **Adversarial review (Phase 4):** returned **REJECTED** with 9 findings (3 blocker, 3 high, 3
  medium/low). Orchestrator fact-checked every finding against the actual codebase:
  - 2 findings were reviewer errors (a "file not found" claim on `adr-019-*.md`, which exists at
    121 lines; a proptest-degeneracy claim from misreading normal `cargo test` output).
  - 3-4 were overreach — applying a "no Leptos-rendering-tests" standard the whole codebase
    (including the already-shipped `user-admin-ui`) doesn't meet.
  - 1 was a genuine, valuable catch: `usage.rs`/`invoices.rs` were rendering placeholder stub text
    despite correct underlying model data. Fixed as step `09-02` (real `UsageTab`/`InvoicesTab`
    rendering `usage_table_rows()`/`model.invoices`), full regression suite re-verified green.
  - Remaining findings were accepted as known, already-flagged limitations (WASM bundle size not
    re-measured this session; CardModal/UpgradeModal placeholder copy is non-functional by design
    per D-3; `Segmented`/`TestClockCard` polish never built, orchestrator-sanctioned).
- **Mutation testing (Phase 5, `per-feature`):** 110 diff-scoped mutants across `model.rs`/
  `update.rs`/`data.rs` (`suspension_banner.rs` excluded — view layer, no rendering harness). Run 1:
  79/110 caught (72.5%, below gate). Added 7 killing tests (uncovered `cap_ratios()` dimensions,
  `from_mock()`'s Pro-plan seeding, `next_invoice_estimate()`'s formula + boundary, Amex/Discover
  brand detection). Run 2: 105/110 caught, 4 pre-existing-`user-admin-ui` misses confirmed caught by
  the full suite + 1 unviable → **100% in-scope kill rate**, well above the 80% `per-feature` gate.
- `cargo build --workspace --features embyr-admin-ui/csr` clean throughout.

## Known Deferred / Incomplete Items

- CardModal/UpgradeModal render PCI-reassurance copy and structural shells but no real form-field
  Stripe.js interop — explicitly `card-payments-backend` scope per D-3.
- The `Segmented` primitive and `TestClockCard` dev toggle polish (Slice 08) were never fully
  built out — orchestrator-sanctioned, no test required it.
- WASM bundle size was not re-measured against the existing ≤4.5 MB CI gate in this DELIVER
  session — flagged, not blocking; existing gate still passes on prior baseline.
- WCAG accessibility audit criteria for the new bar/badge/modal/segmented components remain
  unscheduled — a pre-existing gap in this codebase (also true of `user-admin-ui`), not introduced
  by this feature.
- Outcome KPI instrumentation (funnel events for the 3 Outcome KPIs defined in DISCUSS) is deferred
  to `card-payments-backend`'s DEVOPS wave — this feature ships UI-only, no real event pipeline
  exists yet.

## Lessons Learned

1. **Roadmap review is not a substitute for reading the actual test files.** The
   `nw-acceptance-designer-reviewer` approved a 14-step roadmap with zero blockers, but it contained
   two real sequencing bugs (a test scheduled before its dependency; a backward method-call
   dependency between `suspension_banner_view()` and `effective_status()`). Both were caught only
   because the orchestrator read the actual test files before dispatch, not by the automated
   reviewer. Automated roadmap review should be treated as a floor, not a ceiling, for dependency
   correctness.
2. **Adversarial review earns its cost even when its harshest framing doesn't hold up.** The Phase
   4 review's wholesale "testing theater" / REJECTED framing did not survive fact-checking (2 of 9
   findings were outright reviewer errors, 3-4 were overreach against a codebase-wide standard the
   project doesn't meet anywhere). But it caught one genuine, high-value completeness gap
   (`usage.rs`/`invoices.rs` rendering placeholder stubs despite correct model data) that the
   orchestrator's own review of crafter self-reports had missed. Net: worth running, but its
   verdict needs independent verification before being taken at face value.

## Key Files

- `crates/embyr-admin-ui/src/model.rs` — `Subscription`/`Card`/`Invoice`/`UsageStats`/`Plan`/
  `EffectiveStatus` + `impl AppModel` derivation methods (`usage_totals`, `cap_ratios`,
  `cap_exceeded`, `effective_status`, `read_only`)
- `crates/embyr-admin-ui/src/msg.rs`, `update.rs` — 10 new billing `Msg` variants + match arms
- `crates/embyr-admin-ui/src/data.rs` — `FREE_CAPS`, `PRICING`, `mock::subscription()`,
  `mock::invoices()`
- `crates/embyr-admin-ui/src/views/billing/{mod,overview,usage,invoices,modals}.rs` (NEW,
  replaces deleted `views/billing.rs`)
- `crates/embyr-admin-ui/src/components/suspension_banner.rs` (NEW)
- `crates/embyr-admin-ui/src/components/primitives/segmented.rs` (NEW)
- `docs/product/architecture/adr-019-billing-modal-global-state.md`
- `docs/product/architecture/c4-diagrams-admin-ui.md` § C4 L3 — Component Diagram (Billing
  Subsystem — card-payments)
- `docs/product/architecture/brief.md` § Application Architecture — card-payments
- `docs/product/journeys/billing-management.yaml` (new SSOT journey, supersedes
  `user-admin.yaml` step 9 via back-propagation with `superseded_by` cross-reference)
- `tests/card_payments/` — 65 acceptance/TEA-state scenarios (6 walking-skeleton + 59 slice tests)
- `docs/feature/card-payments/feature-delta.md` — full DISCUSS+DESIGN+DISTILL+DELIVER narrative
  (lean v3.14 SSOT for this feature; retained in place, not migrated)

## Follow-Up Work (Recommended: `card-payments-backend`)

- Stripe SDK integration, `subscriptions`/`accounts.stripe_customer_id`/`processed_webhook_events`
  schema (D-10).
- 5th webhook sub-router on :9090, sibling of `public_router` (D-11).
- Real-time Free-cap enforcement extending the existing per-project rate limiter (D-9).
- Daily batch job pushing usage to Stripe (D-8).
- Stripe Smart Retries → suspend, reusing the existing JOB-06 operator-suspend path (D-12).
- Real Stripe Elements JS interop shim for CardModal (D-3) — this feature's Rust-native form is the
  interaction-design validation target, not the final PCI SAQ-A implementation.
- Outcome KPI event instrumentation (funnel events for the 3 KPIs defined in this feature's
  DISCUSS wave) and WCAG accessibility audit for the new billing components.
