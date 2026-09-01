# Slice 02: A Concrete Document Path Is Deterministically Routed to Its Matching Pattern on Reads

**Story**: US-02 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 2.5 days

## Goal
Build the genuinely new routing mechanism: given a concrete document path and a project's stored multi-segment patterns, deterministically resolve the ONE structurally-matching pattern (if any), bind every captured wildcard segment by name, and wire this into `GetDocument`'s existing rule-lookup composition.

## IN Scope
- The routing lookup itself: concrete path → at most one matching pattern + name-keyed variable bindings.
- Extend `evaluate()`'s `path_variable_value: Option<&str>` (4a, single value) to a name-keyed binding structure.
- Wire routing into `handle_get_document`'s existing rule-lookup composition.
- Fall-through to whatever pre-existing behavior (4a single-wildcard rule, original zero-wildcard rule, or "no rule ⇒ unrestricted") already governs a collection when no multi-segment pattern matches.
- Non-leakage proof: two concrete paths matching the same pattern shape at different wildcard values resolve to fully independent bindings.

## OUT Scope
- Write-path/Listen wiring (Slice 03).
- Structural-overlap detection at import time (Slice 04 — this slice assumes at most one pattern CAN match, per Resolution 1; it does not itself build the overlap-rejection check).
- Recursive wildcards.
- `RunQuery`/Listen subscribe-time compliance re-verification (flagged for DESIGN, § System Constraints — not this slice's own deliverable unless the re-verification surfaces a real gap).

## Learning Hypothesis
Disproves: a concrete document path cannot be deterministically routed to the one structurally-matching stored pattern, with every captured variable correctly bound by name, on a real `GetDocument` call, without either an unbounded per-request scan cost or a second, drift-prone matching implementation.
Confirms (if it succeeds): the routing mechanism can be built as one shared, pure, zero-IO matching function, reusable by both request-time routing and (Slice 04's own) import-time overlap detection.

## Acceptance Criteria
AC-17-207 through AC-17-212 (see `feature-delta.md` § User Stories, US-02).

## Dependencies
Slice 01 (produces the pattern representation this slice routes against).

## Effort Estimate
2.5 days. **Single riskiest slice in this feature** (§ Prioritization) — sequenced first among the WS trio for this reason.

## Reference Class
No direct precedent in this codebase — confirmed by direct read that `system_db.rs` has no "list all rules for a project" method for any of the 3 rule tables today (§ Reading Confirmation). `security-rules-collection-group-rules`' own `group_access_rules` mechanism is a related but non-reusable precedent (flat bare-id exact match, zero pattern/precedence concept).

## Pre-Slice SPIKE
Recommended, not mandatory: a short technical spike to confirm the chosen storage/routing direction (§ System Constraints' two named candidates) before full implementation, given this is the single highest-uncertainty slice in the feature.
