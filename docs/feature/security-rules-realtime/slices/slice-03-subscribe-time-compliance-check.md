# Slice 03: A Listen Subscription Against a Rule-Protected Collection Is Admitted Only if the Initial Snapshot's Filter Is Compliant

**Story**: US-03 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
Extend `check_query_compliance()` (ADR-031, unchanged) to `Listen`'s own subscribe-time admission decision — the same mechanism `RunQuery` already uses, now composed into `handle_add_target`'s async streaming call shape instead of a request/response one. A compliant subscription (filter satisfies the rule) is admitted and the stream opens; a non-compliant one is rejected outright, before the initial snapshot ever runs.

## IN Scope
- A rule lookup (`get_access_rule`, unchanged) inside `handle_add_target`, gated behind Slice 02's own real filter.
- `None` (no rule) → proceed exactly as today, structurally unmodified (proven fully in Slice 06).
- `Some` → call `check_query_compliance()` exactly as `handle_run_query` already does; `Admitted` → subscription proceeds; anything else → subscription rejected as a terminal stream error, before any row is read.
- Identity attach (`attach_client_identity_if_present`, unchanged function, new call site) so `request.auth` is available to the compliance check.

## OUT Scope
- Any modification to `check_query_compliance()`, `QueryComplianceOutcome`, or `UnsatisfiedConjunct` (ADR-031) — reused completely unmodified.
- Per-event re-checking (Slices 04–05) — this slice covers ONLY the one-time subscribe-time gate.
- The 5-shape decidable set is unchanged; no new rule shape is introduced.

## Learning Hypothesis
**Disproves if it fails**: A Listen subscription cannot be proven compliant with a rule-protected collection's own rule, before the initial snapshot runs, by reusing `check_query_compliance()` completely unmodified.
**Confirms if it succeeds**: The identical function, identical types, and identical admit/reject outcome mechanism `RunQuery` already uses compose cleanly into a streaming handler's own async setup phase.

## Acceptance Criteria
- AC-17-113: A Listen subscription whose filter satisfies a rule-protected collection's own rule is admitted; the stream opens and the initial snapshot runs.
- AC-17-114: A Listen subscription whose filter does not satisfy the rule is rejected outright, before any row is read for the initial snapshot.
- AC-17-115: `check_query_compliance()`, `QueryComplianceOutcome`, and `UnsatisfiedConjunct` (ADR-031) are reused completely unmodified.
- AC-17-116: The rejection is delivered as a terminal stream error, distinguishable from `authenticate()`-level rejections.

## Production-Data Taste Test
Real rule-protected `journal_entries`, real compliant and non-compliant `AddTarget` requests from real Maria/Dana sessions.

## Dependencies
Slice 02 (a real filter must exist to check against).

## Reference Class
Mirrors `security-rules-query-path`'s own US-01/US-02 admit/reject mechanism verbatim, composed into a new async call shape.
