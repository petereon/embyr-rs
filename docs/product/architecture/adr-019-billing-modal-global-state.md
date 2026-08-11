# ADR-019: Cross-Cutting Modal Triggering — Global `AppModel` State for `CardModal`/`UpgradeModal`

## Status

Accepted

## Context

`card-payments` introduces two modals — `CardModal` (payment method capture) and `UpgradeModal`
(plan compare/confirm) — and a cross-cutting `SuspensionBanner` (US-108) that must be able to
open either modal directly from anywhere in the console, not just from within the Billing section.

Per AC-108-04, `SuspensionBanner` renders above every routed `Section` (Dashboard, Databases,
Billing, etc.), gated only on `AppModel::read_only()`. Per AC-108-05, clicking the banner's CTA
must open the correct modal *directly* — "Upgrade to Pro" opens `UpgradeModal`, "Update payment
method" opens `CardModal` — with no intermediate navigation step to the Billing section first.

The existing codebase has one precedent for a confirmation modal: `views/db_detail/mod.rs`'s
delete-confirmation dialog, which uses a plain **view-local `RwSignal<bool>`** (`show_del`) that
is never wired through `Msg`/`update()`. This works there because that modal is only ever
triggered from within the same view that renders it (`DbDetailView`'s own "Delete database"
button) — there is no cross-component coordination requirement.

`CardModal`/`UpgradeModal` do not have that property: they must be opened from at least two
component subtrees that are not ancestor/descendant of each other in the render tree —
`BillingView`'s own `PlanCard`/`PaymentMethodCard` (a descendant of the `Billing` `Section` match
arm), and `SuspensionBanner` (a sibling of the `Section` router itself, in `ShellView`).

## Decision

**`CardModal`/`UpgradeModal` open/step state lives in `AppModel`** (`card_modal_open: bool`,
`upgrade_modal_open: bool`, `upgrade_modal_step: UpgradeModalStep`), mutated only via `Msg`
(`OpenCardModal`/`CloseCardModal`/`OpenUpgradeModal`/`CloseUpgradeModal`/`SetUpgradeModalStep`),
and both modal components are **mounted at the `ShellView` level** (`views/mod.rs`), gated by
`<Show when=move || model.with(|m| m.card_modal_open)>` / the `upgrade_modal_open` equivalent,
rendered as siblings of `SuspensionBanner` and the routed `<main class="content">` block — not
nested inside `BillingView`.

This makes `SuspensionBanner`'s CTA a plain `dispatch(Msg::OpenCardModal)` /
`dispatch(Msg::OpenUpgradeModal)` call, identical in shape to `BillingView`'s own trigger buttons,
with no special-casing for "which section is currently active."

## Consequences

**Benefits:**

- `SuspensionBanner`'s CTA and `BillingView`'s own trigger buttons dispatch the exact same `Msg`
  variants — one code path, not two divergent "open the modal" mechanisms depending on caller.
- Modal visibility survives a `Section` change while open (not a required behavior per any AC,
  but a natural, low-risk consequence of global state that avoids a class of bugs where
  navigating away mid-modal silently discards in-progress form state).
- Directly exercises ADR-006's own stated rationale for `AppModel`/`Msg`/context ("no
  prop-drilling... a deeply nested component can dispatch without the parent knowing about it")
  in exactly the scenario it was designed for — this is not a new pattern, it is the textbook
  application of an existing one.
- Testable without a browser: `update(&mut model, Msg::OpenUpgradeModal)` followed by an assertion
  on `model.upgrade_modal_open` and `model.upgrade_modal_step` is a plain `cargo test`, same as
  every other `Msg` variant.

**Trade-offs and costs:**

- Diverges from `db_detail`'s existing local-`RwSignal` modal precedent — a future contributor
  skimming `db_detail/mod.rs` first might reasonably ask "why isn't this modal local too?" This
  ADR exists specifically to answer that question in one place rather than requiring the
  divergence to be re-derived from first principles at each site.
- Adds 5 fields to `AppModel` and 5 variants to `Msg` for what is, mechanically, pure UI-visibility
  state (not domain data). ADR-006 already flags `Msg` enum growth as a tracked, non-blocking
  maintainability concern at 50+ variants — this feature's 10 total new variants (5 of which are
  modal-visibility, 5 of which are domain mutations) are within that already-accepted trajectory.
- If a *third* independent trigger site for either modal is added later (unlikely given the
  console's shape, but not architecturally prevented), the "any component can dispatch
  `OpenCardModal`" property scales for free — this is the reverse of a cost, but worth stating as
  the reason this decision does not need revisiting if that happens.

## Alternatives Considered

### Alternative A: View-local `RwSignal<bool>`, lifted to `ShellView` and passed via props

Keep the `db_detail` pattern (`RwSignal<bool>` created where the modal is rendered), but create
it in `ShellView` and pass it down as a prop to both `BillingView`'s trigger buttons and
`SuspensionBanner`.

**Rejected because:**

- `BillingView` is three component-levels below `ShellView` in the render tree
  (`ShellView → <main> match → BillingView → overview.rs's PlanCard`). Passing the signal down as
  a prop through every intermediate layer reintroduces exactly the prop-drilling problem ADR-006
  rejected Alternative A ("Prop drilling") for — the same rationale applies here at smaller scale.
- `use_context::<RwSignal<bool>>()` for a *second*, billing-specific context value alongside the
  existing `AppModel`/`Callback<Msg>` context pair would work, but fragments state visibility: a
  future maintainer inspecting `AppModel`'s fields to understand "what can this console currently
  show" would not see modal-open state there, undermining ADR-006's own "single source of truth"
  consequence.

### Alternative B: Banner CTA navigates to Billing section, then opens the modal

`SuspensionBanner`'s CTA dispatches `Msg::NavigateTo(Section::Billing)` first; `BillingView`,
once mounted, reads a "pending intent" flag and self-opens the relevant modal on mount.

**Rejected because:**

- Directly contradicts AC-108-05's phrasing ("Priya clicks 'Upgrade to Pro' on the banner Then the
  Upgrade modal opens") — the UAT scenario describes a single, direct action-to-result transition,
  not a navigate-then-open two-step flow. A "pending intent" flag is also itself global `AppModel`
  state with an extra layer of indirection, so it does not avoid Alternative A/B's core trade-off —
  it just adds a race-prone "did the view mount and consume the intent flag yet" step for no
  benefit.

### Alternative C: Two independent global signals, one per modal, provided directly via context (bypassing `AppModel`)

`provide_context(card_modal_open: RwSignal<bool>)` at the `App` root, separate from `AppModel`.

**Rejected because:**

- This is ADR-006's Alternative C ("Multiple `RwSignal<T>` per domain area") re-litigated at
  smaller scope, and ADR-006 already rejected it for the same reason: cross-domain coordination
  (here, `SuspensionBanner`'s derived `effective_status()` deciding *which* modal to open) would
  need to read from `AppModel` for the derivation and from a second, separate signal for the
  open-action — two signals that must be kept conceptually synchronized by convention rather than
  by the type system. Keeping both in `AppModel` means `update()` remains the single place that
  can be inspected to answer "what does `Msg::OpenUpgradeModal` actually do."
