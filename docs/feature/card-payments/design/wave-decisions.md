# DESIGN Decisions — card-payments

## Key Decisions

- [D1] Extend the existing `embyr-admin-ui` TEA skeleton (ADR-005/006/007) with zero new architectural pattern: no new crate, no new external dependency, no backend changes. This feature is frontend-only per the DISCUSS Scope Assessment split (see: `docs/feature/card-payments/feature-delta.md` § Wave: DISCUSS / [REF] Scope Assessment).
- [D2] `views/billing/{mod,overview,usage,invoices,modals}.rs` mirrors the existing `views/db_detail/` multi-file subdirectory precedent exactly (see: `crates/embyr-admin-ui/src/views/db_detail/mod.rs`).
- [D3] `capExceeded`/`effectiveStatus`/`readOnly` implemented as pure `impl AppModel` methods (`usage_totals()`, `cap_ratios()`, `cap_exceeded()`, `effective_status()`, `read_only()`) in `model.rs` — computed on every read, never stored as duplicated booleans. Satisfies the DISCUSS constraint verbatim (see: `docs/feature/card-payments/discuss/wave-decisions.md` § Constraints Established).
- [D4] `CardModal`/`UpgradeModal` open/step state is global `AppModel` state (dispatched via `Msg`), not view-local `RwSignal` — diverges from `db_detail`'s local-modal precedent because `SuspensionBanner` (cross-cutting) must open either modal directly regardless of active `Section` (AC-108-05). See: `docs/product/architecture/adr-019-billing-modal-global-state.md`.
- [D5] `Segmented` is a new primitive (`components/primitives/segmented.rs`); progress/cap bars are NOT extracted as shared primitives (kept page-local, single consumer each in V1 — YAGNI). See: `docs/feature/card-payments/feature-delta.md` § Wave: DESIGN / [REF] Reuse Analysis.
- [D6] `Database` struct extended with `usage: UsageStats` field rather than a new parallel aggregate — one source of truth per database for usage numbers consumed by `CapUsageCard`, `NextInvoiceCard`, and the Usage tab.
- [D7] `FREE_CAPS`/`PRICING` constants placed in `data.rs`, matching the DISCUSS Shared Artifacts Registry's explicit source-of-truth assignment and `data.rs`'s existing role blending mock-seed data with pure display-computation helpers.
- [D8] `CardModal` remains Rust-native form/validation only in V1 — no Stripe.js/Elements JS interop shim (inherited from D-3/D-5 locked decisions; explicit DISCUSS scope boundary, forward-flagged for `card-payments-backend`).

## Architecture Summary

- Pattern: Modular monolith extension (existing) — Leptos 0.8 CSR WASM SPA, TEA (`RwSignal<AppModel>` + `Callback<Msg>` via Leptos context), mock-first data layer (ADR-007). No pattern change.
- Paradigm: Functional-where-practical Rust (project CLAUDE.md, unchanged — no re-litigation per dispatch instructions).
- Key components: `views/billing/{mod,overview,usage,invoices,modals}.rs`, `components/suspension_banner.rs`, `components/primitives/segmented.rs`, extensions to `model.rs`/`msg.rs`/`update.rs`/`data.rs`/`icons.rs`/`views/mod.rs`.

## Reuse Analysis

| Existing Component | File | Overlap | Decision | Justification |
|-------------------|------|---------|----------|---------------|
| `Tabs` | `components/primitives/tabs.rs` | Overview/Usage/Invoices sub-tab bar | EXTEND (reuse unmodified) | Generic `&'static str` API already covers billing's tab set |
| `Modal` | `components/primitives/modal.rs` | CardModal/UpgradeModal overlay shell | EXTEND (reuse unmodified) | `title`/`on_close`/`children` props cover both use cases |
| `Toggle` | `components/primitives/toggle.rs` | Checked for TestClockCard | NOT APPLICABLE | No boolean-switch use case in any of the 8 slices; `Segmented` is semantically distinct |
| `Database` | `model.rs` | Per-database usage counters | EXTEND | Add `usage: UsageStats` field — one aggregate, not a parallel type |
| `AppModel` | `model.rs` | Subscription/invoice/modal-open state | EXTEND | 5 new fields — single source of truth per ADR-006 |
| `Msg` | `msg.rs` | New billing transitions | EXTEND | New story-grouped block, 10 variants |
| `update()` | `update.rs` | New billing match arms | EXTEND | Arms in existing exhaustive match |
| `data.rs` | `data.rs` | Mock constructors + constants | EXTEND | Follows `mock::databases()`/`mock::admin_keys()` convention |
| `views/billing.rs` (127-line placeholder) | `views/billing.rs` | Fully superseded | REPLACE (content migrated, file deleted) | File's own doc comment anticipated this replacement |
| `views/mod.rs` | `views/mod.rs` | Module resolution + shell routing | EXTEND | Standard file→directory promotion; `ShellView` extended to mount cross-cutting components |
| `views/db_detail/` layout | `views/db_detail/*.rs` | Multi-file view subdirectory shape | PATTERN REUSE (no shared code) | `views/billing/` mirrors this shape exactly |
| `Icon` | `components/icons.rs` | 3 of 4 CapUsageCard dimension icons already exist | EXTEND | Add `"trash"` match arm |
| — | `components/primitives/segmented.rs` | No existing N-way value-select control | CREATE NEW | Confirmed absent from `mod.rs`/`modal.rs`/`tabs.rs`/`toggle.rs`; semantically distinct from `Tabs` |
| — | (bar-shaped rendering) | No existing progress-bar primitive | NOT EXTRACTED | Single consumer each in V1 (CapUsageCard, UsageTab) — YAGNI |
| `components/mod.rs` | `components/mod.rs` | Global chrome re-exports | EXTEND | `SuspensionBanner` added alongside `Sidebar`/`Topbar` |

**Verdict**: 10 EXTEND / 1 REPLACE / 1 PATTERN REUSE / 2 CREATE NEW / 1 NOT APPLICABLE / 1 NOT EXTRACTED. Zero unjustified CREATE NEW.

## Technology Stack

- Leptos 0.8 CSR WASM (unchanged, ADR-005) — no new dependency.
- No Stripe.js/Stripe Elements JS interop in V1 (D-3/D-5 scope boundary) — zero new JS/npm surface.
- No new Rust crate dependency of any kind. Bundle size impact is incremental application code only; existing ≤4.5 MB CI gate applies unchanged.

## Constraints Established

- `capExceeded`/`effectiveStatus`/`readOnly` must remain pure `impl AppModel` methods — any future PR that introduces a stored `is_suspended`-style field duplicating this logic is a regression against this design and against the DISCUSS constraint it satisfies.
- `CardModal`/`UpgradeModal` open-state must remain global `AppModel` fields (ADR-019) — reverting to view-local `RwSignal` would silently break `SuspensionBanner`'s cross-Section CTA (AC-108-05) without a compile error.
- `SuspensionBanner` is advisory visibility only in this feature (inherited DISCUSS constraint) — this design does not add any write-blocking/disabling behavior anywhere in the console; real enforcement is `card-payments-backend` scope (D-9's rate-limiter extension).
- Existing `≤4.5 MB` WASM bundle CI gate applies unchanged to all new components.

## Upstream Changes

None. No DESIGN-wave decision here contradicts or requires changing any DISCUSS-wave story, AC, or locked decision (D-1..D-13). No `docs/feature/card-payments/design/upstream-changes.md` was created — there is nothing to propagate upstream.

## Handoff

**To DISTILL (acceptance-designer)**: `docs/feature/card-payments/feature-delta.md` § Wave: DESIGN (component decomposition, model/msg/update contracts, driving/driven ports, C4 Component diagram), this file, `docs/product/architecture/brief.md` § Application Architecture — card-payments, `docs/product/architecture/adr-019-billing-modal-global-state.md`.

**To DEVOPS (platform-architect)**: forward flag only — no external integrations in this feature; `card-payments-backend`'s eventual DESIGN wave will need Pact-style contract test recommendations for Stripe (REST API + webhooks), noted in `feature-delta.md` § Wave: DESIGN / [REF] Driven Ports and Adapters.

**Per-wave peer review**: skipped per dispatch instructions (default skip per SKILL — no contested ADR, no novel pattern, no unverified performance budget, no security-boundary change; this is a well-precedented extension of an existing, working pattern). Mandatory consolidated review fires at end of DISTILL covering all 4 waves in parallel.
