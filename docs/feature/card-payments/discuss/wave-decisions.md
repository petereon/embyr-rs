# DISCUSS Decisions — card-payments

## Key Decisions

- [D1] **Feature type = Cross-cutting, but DISCUSS scope narrowed to frontend UI**: card-payments as originally framed spans 4 bounded contexts (billing-domain, webhook-ingestion, rate-limiting, admin-UI). Scope Assessment (Phase 1.5) found this oversized (3 of 5 signals fired). Split into `card-payments` (this feature — frontend billing UI, mock-data-first, extends `embyr-admin-ui`) and a recommended follow-up feature `card-payments-backend` (Stripe SDK, webhooks, schema, rate-limiter extension, batch job, dunning). Mirrors this project's own precedent: `user-admin-ui` (frontend, mock data) shipped as a separate nWave feature before `admin-api-v2` (backend wiring). See feature-delta.md § Scope Assessment.
- [D2] **Walking Skeleton = mock-data facade (ADR-007 pattern), not a new pattern**: Slice 01 renders real Plan/Payment-Method cards + a navigable tab shell against deterministic mock data; the eventual backend swap is a same-shape `Resource`/`Action` body substitution per ADR-007's established migration contract. No new WS pattern was needed — this project already solved "how do we sequence UI ahead of backend" once, for user-admin-ui.
- [D3] **New job JOB-14 (`manage-subscription`), not an extension of JOB-10**: JOB-10's "view billing breakdown" functional item is informational; JOB-14 carries distinct financial/trust stakes (card capture, hard-stop suspension, upgrade/downgrade decisions) with its own anxiety force (surprise billing, card-data safety) not present in JOB-10. JOB-10 left unchanged; cross-reference note added. See `docs/product/jobs.yaml` JOB-14, and JOB-10's inline note.
- [D4] **JOB-06 (tenant-control, operator-side suspend) is reused unchanged, not modified**: D-12's dunning suspension reuses the *same suspend code path* as Free-cap-exceeded (operator's JOB-06 mechanism), now triggered automatically by billing state rather than manually by Sam. No new job needed for this; flagged as an integration dependency for `card-payments-backend`, not this feature.
- [D5] **New SSOT journey `docs/product/journeys/billing-management.yaml`**, superseding `user-admin.yaml` happy_path step 9 via the Document Update / Back-Propagation procedure — step 9 preserved with a `superseded_by` cross-reference, not deleted.
- [D6] **9 user stories (US-101..US-109), 8 Elephant Carpaccio slices**, single bounded context (admin-UI), ≤1 day each, right-sized per the LeanUX sizing gate.
- [D7] **Production-data taste-test exception documented, not silently skipped**: all 8 slices use synthetic/mock data (ADR-007 pattern), which nominally fails the Elephant Carpaccio "no synthetic-only slices" taste test. Exception accepted because this project has an established, ADR-backed precedent (ADR-007) for sequencing UI validation ahead of backend integration; production-data proof is explicitly deferred to `card-payments-backend`'s slices (mirroring `admin-api-v2` slice-B01..B06).

## Requirements Summary

- Primary jobs/user needs: Chris (P5, Account Admin) needs to see plan/payment/usage-cap status at a glance, add or update a card, upgrade or downgrade plans with clear consequences, review invoice history, and understand *why* the console is read-only when suspended — all self-service, without contacting the embyr operator (JOB-14, extends JOB-10's umbrella self-sufficiency goal).
- Walking skeleton scope: Slice 01 — Billing Overview shell (real Plan + Payment Method cards, navigable tab shell, stub modals). See `docs/feature/card-payments/slices/slice-01-billing-overview-shell.md`.
- Feature type: User-facing (frontend UI), scoped down from the original cross-cutting framing per the split above.

## Constraints Established

- Every story traces to `job_id: JOB-14` (`docs/product/jobs.yaml`).
- Status derivation (`capExceeded`/`effectiveStatus`/`readOnly`) MUST be a pure function over `AppModel` fields, not duplicated stored booleans — per this project's CLAUDE.md functional-where-practical paradigm.
- `SuspensionBanner` is advisory visibility only in this feature; it does not disable/gray out any other console action. Real enforcement (blocking writes) is `card-payments-backend` scope (D-9's rate-limiter extension). Flagged explicitly so DESIGN does not assume otherwise.
- WASM bundle size guardrail (existing CI gate, ≤4.5 MB, established in user-admin-ui slice-01) applies to all new components added by this feature.
- CardModal is Rust-native form/validation only in V1 — no real Stripe.js/Elements JS interop shim yet (that shim is `card-payments-backend` scope per D-3's explicit note in grill-me-decisions.md).

## Upstream Changes

- No DISCOVER or DIVERGE artifacts exist for `card-payments` (`docs/feature/card-payments/discover/` and `diverge/` were not found). This DISCUSS wave proceeded directly from a prior `/grill-me` architecture-decision session (`docs/decisions/card-payments/grill-me-decisions.md`, D-1..D-13, locked) plus a design-reference document (`docs/feature/card-payments/design-reference.md`) substituting for direct DesignSync access. Flagged as a risk: no independent DISCOVER-stage evidence (customer interviews, opportunity validation) exists for this feature beyond the grill-me session's own reasoning. Recommend product-discoverer validation of the JOB-14 opportunity score (17) if resourcing allows, though the score is well-supported by JOB-10's existing precedent (opportunity_score 16, same persona) and D-6's explicit trust-preservation rationale.
- `docs/product/journeys/user-admin.yaml` step 9 updated (back-propagation) — see D5 above.

## Density / Expansion Notes

DISCUSS default density = lean + ask-intelligent. Triggers evaluated against this feature's artifacts:
- **Cross-context complexity**: fired pre-split (4 bounded contexts). Addressed via the Scope Assessment split itself (D1) — an `alternatives-considered`-style rationale is included in feature-delta.md § Scope Assessment rather than as a separate Tier-2 render.
- **Compliance/regulatory** (PCI, tokenization, encryption terms in AC): fired. Addressed inline via a `[HOW] Journey Deep-Dive` section in feature-delta.md covering the PCI SAQ-A messaging requirements, rather than a data-migration playbook (not applicable — no user migration involved).
- **Multi-stakeholder** (Chris, Dana, Priya as named examples; Sam via JOB-06 cross-reference): fired. Addressed via the extended persona note in `docs/product/personas/chris-account-admin.yaml` plus domain examples embedded per story.
- No interactive user was available in this autonomous dispatch to respond to an expansion menu; triggered content was rendered directly into feature-delta.md's Tier-1 sections rather than gated behind an `--expand` prompt, since Decision 3 (Comprehensive UX research depth) already calls for full experience mapping regardless of density mode.
