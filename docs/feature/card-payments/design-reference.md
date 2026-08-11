# Card Payments — Design Reference (from Claude Design prototype)

**Source**: Claude Design project "embyr" (projectId `a4c92548-e93b-47b3-8c59-39feb97471d1`), file
`embyr Console.html`. This is a React/JSX click-prototype — reference/mockup only, NOT literal
implementation code. Real implementation target is the Leptos/WASM `embyr-admin-ui` crate.
Sub-agents dispatched for this feature do NOT have access to the DesignSync MCP tool — this
document is the complete local substitute for reading the remote design project directly.

Companion doc: `docs/decisions/card-payments/grill-me-decisions.md` (13 locked architecture
decisions D-1..D-13 from a prior `/grill-me` session). Cross-references below use `D-N`.

## Why this design is trustworthy input

`uploads/spec.md` inside the same design project is the pre-existing, authoritative "User-Admin
Interface" spec (v2026-06-05). Its OQ-8 resolves: "Frontend tech stack: **Leptos** — Rust/WASM
frontend with `leptos_axum` integration on :9090 ... No TypeScript/JavaScript context switch."
That confirms the JSX is a prototyping artifact whose *behavior and layout* should be ported into
idiomatic Leptos TEA (Model/Msg/Update), not copied as code. The old spec's billing section was
read-only usage reporting; this design + the grill-me decisions supersede that with full
subscription/payment management.

## Existing target file to extend/replace

`crates/embyr-admin-ui/src/views/billing.rs` (127 lines) — current V1 placeholder. Renders a
time-range selector + per-database usage table with all-placeholder `"—"` values. Doc comment in
the file itself says: "V2 plan: replace with `#[server]` function that returns real billing data."
Zero Stripe/payment logic today. This is an expected, planned extension point.

## Leptos TEA conventions (confirmed by reading model.rs / msg.rs / update.rs / app.rs)

- `crates/embyr-admin-ui/src/model.rs`: newtype IDs (`pub struct XId(pub Uuid)`, all
  `Clone+Debug+Default+PartialEq+Eq+Hash`), enums with `#[default]` variant, domain structs
  `#[derive(Clone, Debug, Default, PartialEq)]`, `Section` enum drives top-level nav
  (`Dashboard`, `Databases`, `DbDetail(DbId)`, `Identities`, `ApiKeys`, `Billing`, `Settings`,
  `Login`) — a `Billing` variant already exists. `AppModel` is one flat struct with `pub` fields;
  `AppModel::from_mock()` builds initial state from `crate::data::mock::*`.
- `crates/embyr-admin-ui/src/msg.rs`: single exhaustive `Msg` enum, variants grouped by user
  story with `// ── US-NNN: Name ──` comment headers. `Msg` derives `Clone, Debug` only (no
  `PartialEq` — some variants would need it, others carry non-comparable payloads).
- `crates/embyr-admin-ui/src/update.rs`: single `pub fn update(model: &mut AppModel, msg: Msg)`,
  pure, no IO, exhaustive match with one arm block per `Msg` variant, mirroring the story-grouping
  comments from msg.rs. Small local helper fns (e.g. `count_owners`) live at the bottom of the
  file. Invariant guards (e.g. sole-Owner protection) are inlined as boolean checks before mutation.
- `crates/embyr-admin-ui/src/app.rs`: `App()` component creates `RwSignal<AppModel>` +
  `Callback<Msg>` wrapping `update()`, provides both via `provide_context`, and gates
  `AuthView` vs `ShellView` on `model.authed`. Entire file is `#[cfg(feature = "csr")]`.
- Existing subdirectory precedent for multi-file views: `views/db_detail/` = `mod.rs` +
  `overview.rs` + `connections.rs` + `keys.rs` + `logs.rs`. A `views/billing/` directory
  (`mod.rs` + `overview.rs` + `invoices.rs` + `modals.rs`) mirroring the JSX split is the natural
  DESIGN-wave choice, consistent with existing precedent.
- Existing primitives to reuse: `components/primitives/modal.rs`, `tabs.rs`, `toggle.rs`,
  `components/icons.rs`, `components/charts/mod.rs`. New primitives needed for this feature (not
  yet present): a Stripe-Elements-styled card-input group, a segmented control (JSX has
  `Segmented` in `ui.jsx` — check if a Rust equivalent already exists under `primitives/` before
  creating one), progress/cap bars.

## Design prototype component inventory (JSX → what it implements)

### Cross-cutting: `SuspensionBanner` (app.jsx) — **implements D-6 hard-stop**

Reads `{ subscription, readOnly, go } = useApp()`. Rendered directly inside the shell, above all
routed views, so it's visible regardless of which section is active. Two states:
- `subscription.status === "free_cap_exceeded"` → amber banner, "You've reached your Free plan
  limits for this cycle.", CTA "Upgrade to Pro".
- otherwise (`readOnly` true for another reason, i.e. payment failure) → red banner, "We couldn't
  process your last payment.", CTA "Update payment method".
Only rendered at all when `readOnly` is true. Port this as a component in `embyr-admin-ui` that
reads billing/subscription state from `AppModel` and renders above `<Show>`-routed content in
`ShellView`, gated on a new `readOnly`/`subscription_status`-derived condition.

### State derivation (store.jsx) — **the core D-6/D-7 business logic to port into `update.rs`/`model.rs`**

```js
const usageTotals = {
  reads: sum(db.reads) * 30, writes: sum(db.writes) * 30,
  deletes: sum(db.deletes) * 30, storageGB: sum(db.logStorageGB),
};
const capRatios = { reads: usageTotals.reads / FREE_CAPS.reads, ...same for writes/deletes/storageGB };
const maxCapKey = key with highest ratio;
const capExceeded = plan === "free" && capRatios[maxCapKey] >= 1;
const effectiveStatus = capExceeded ? "free_cap_exceeded" : paymentFailure ? "past_due" : "active";
const readOnly = effectiveStatus !== "active";
```

`FREE_CAPS = { reads: 2_000_000, writes: 500_000, deletes: 100_000, storageGB: 2 }` (data.js).
This is a derived/computed view over subscription + usage state, not stored state — the Rust
equivalent should likely be a pure function over `AppModel` fields (subscription plan, per-cap
usage, payment_failure flag) rather than duplicated stored fields, matching this codebase's
functional-where-practical paradigm (see project CLAUDE.md).

`setPlan`/`setCard`/`setPaymentFailure` are the JSX mutators — map to new `Msg` variants, e.g.
`Msg::SetPlan(Plan)`, `Msg::SetCard(Card)`, `Msg::SetPaymentFailure(bool)` (or better: real
variants driven by webhook-sourced state once backend exists — V1/mock-data phase can use direct
setters mirroring the JSX for parity, per this project's established mock-data-first UI pattern
seen in `AppModel::from_mock()`).

### `BillingView` shell (views_billing.jsx)

Page header "Plan & Billing" + `Tabs` (`overview` / `usage` / `invoices`):
- `overview` → `BillingOverviewTab` (views_billing_overview.jsx)
- `usage` → local `UsageTab`: per-database 3-color stacked `UsageBar` (reads=blue,
  writes=accent, deletes=red) + 5 KPI summary tiles + full breakdown table. This supersedes the
  existing all-placeholder table in `billing.rs`.
- `invoices` → `BillingInvoicesTab` (views_billing_invoices.jsx)

### `BillingOverviewTab` (views_billing_overview.jsx) — 5 sub-components, 2-column grid

- **`PlanCard`**: Free/Pro badge; status banners for `free_cap_exceeded`/`past_due`; Pro shows
  base price + renewal date, Free shows included volume; "Change plan"/"Upgrade to Pro" button
  opens `UpgradeModal`.
- **`PaymentMethodCard`**: card brand/last4/expiry, green "on file" badge, "Update"/"Add card"
  button opens `CardModal`, small mono `stripeCustomerId` display.
- **`CapUsageCard`** (Free plan) / **`NextInvoiceCard`** (Pro plan): `CapUsageCard` shows a
  `CapBar` per dimension (red ≥1.0, amber ≥0.8, accent otherwise) with expandable per-dimension
  breakdown, icons `{reads:"zap", writes:"activity", deletes:"trash", storageGB:"database"}`.
  `NextInvoiceCard` computes overage estimate from usage × `PRICING`, footer note: "Rates shown
  are illustrative placeholders — final pricing is set in DISCUSS/DESIGN" (i.e. the prototype
  itself defers exact pricing to this nWave pipeline — DISCUSS should set real numbers or
  explicitly re-defer to a pricing/business decision per grill-me's "Open" section).
- **`TestClockCard`**: dashed border, amber "TEST MODE" badge, `Segmented` control
  ("Payment succeeds"/"Payment fails") toggling `paymentFailure` — a dev-only Stripe-test-mode
  simulator, consistent with **D-13** (real Stripe test-mode, no mocked port). Port this as a
  dev/staging-only UI affordance that triggers real Stripe CLI `stripe trigger` events or a
  debug-only backend endpoint — do not fake payment state client-side in production builds.

### Modals (views_billing_modals.jsx)

- **`UpgradeModal`**: two-step (`compare` → `confirmDowngrade`). Side-by-side plan comparison via
  `PlanColumn` + `PLAN_FEATURES.free`/`.pro` bullet lists. Downgrade confirmation explicitly warns:
  "Hard usage caps apply immediately — exceeding them suspends the console until you upgrade
  again." — directly implements **D-6**.
- **`CardModal`**: Stripe-Elements-styled capture form (`.stripe-el` CSS class family) with
  brand-detection (`detectBrand()`) and number formatting helpers. Copy: "Card details are
  tokenized by Stripe Elements — embyr never sees the raw number (PCI SAQ-A)." Footer: "Secured by
  Stripe · test mode, no real charge." Directly implements **D-3**. In the real Leptos
  implementation this requires a small JS interop shim to load/mount Stripe.js/Stripe Elements
  inside the WASM app (per grill-me D-3's explicit note) — Elements itself cannot be a pure Rust
  component since it's an iframe-based widget Stripe controls for PCI scope reasons.

### `BillingInvoicesTab` (views_billing_invoices.jsx)

Table: Date / Period / Base / Overage / Total / Status / PDF-download. Free-plan empty state: "The
Free plan has no recurring charges — invoices appear once you're on Pro."

## Mock data shapes (data.js) — for V1/mock-data-first Rust model design

```js
FREE_CAPS = { reads: 2_000_000, writes: 500_000, deletes: 100_000, storageGB: 2 }
PRICING = { proBase: 49, overage: { reads: 0.05, writes: 0.08, deletes: 0.15 }, overageUnit: 100_000, storagePerGB: 0.20 }
subscription = { plan: "free"|"pro", stripeCustomerId, currentPeriodEnd, card: { brand, last4, expMonth, expYear }, paymentFailure: bool }
invoice = { id, date, period, base, overage, amount, status: "upcoming"|"paid" }
```

## Shared UI primitives already in the JSX design system (ui.jsx / icons.jsx / charts.jsx / styles.css)

`Button`, `Badge`/`StatusBadge`, `Card`, `Toggle`, `Segmented`, `Tabs`, `Field`/`Input`/`Select`,
`Modal`, `ToastProvider`. Icons via Lucide-style SVG path data (`icons.jsx`). Design tokens: CSS
custom properties for colors (`--accent`, `--green`/`--amber`/`--red`/`--blue` + `-soft`
variants), 3 "look" themes, density modes. `.stripe-el` class family is specifically for the card
capture form styling. Check `crates/embyr-admin-ui` existing CSS/primitives for equivalents before
introducing new ones — several (`Tabs`, `Modal`, `Toggle`) already exist as Rust components.

## Explicitly NOT part of the product (ignore when porting)

- `tweaks-panel.jsx` / `ConsoleTweaks` in app.jsx — the design tool's own theming/debug panel,
  marked `// @ds-adherence-ignore -- omelette starter scaffold` in source. Not product UI.
- Views for Dashboard/Databases/Identities/API Keys/Auth/Settings/DB Detail — already implemented
  in `embyr-admin-ui`; useful only for design-language consistency reference, not in scope for
  this feature.

## Backend scope reminder (from grill-me-decisions.md, not covered by the JSX design at all)

The JSX is frontend-only. Backend work per D-1..D-13 is a separate, larger scope: Stripe SDK
integration, new `subscriptions` + `processed_webhook_events` tables +
`accounts.stripe_customer_id` column, a 5th admin sub-router for Stripe webhooks (structural
sibling of `public_router` in `crates/embyr-server/src/admin/router.rs`), extension of the
existing Postgres token-bucket rate limiter (`crates/embyr-server/src/middleware/rate_limit.rs`)
for real-time Free-cap enforcement, a daily batch job pushing usage to Stripe from
`daily_project_metrics`, and dunning handling reusing the same suspension path as Free-cap-exceeded.
DISCUSS should slice this feature across both frontend (Leptos billing views) and backend
(Stripe/webhooks/rate-limiter/schema) as separate Elephant-Carpaccio slices, sequenced so backend
subscription/read APIs exist before frontend views that display live (not mock) data need them —
though per this project's established pattern (see `user-admin-ui`), an initial mock-data-only
frontend slice is an acceptable/expected early slice.
