# Slice 03: Auth-Presence-Only and Public Query Compliance (Walking Skeleton)

**Story**: US-03 | **Release**: 1 | **Estimate**: 1 day

## Goal
A `RunQuery` against a collection whose rule doesn't reference document
fields at all — `request.auth != null` (auth-required) or `true` (public) —
is decided from the query-time `AuthContext` alone, with no filter
requirement, matching what `GetDocument` already does for the equivalent
rule.

## IN Scope
- `Condition::Literal(true)` branch — always compliant, no filter check.
- `Condition::Literal(false)` branch — never compliant (deny-all).
- `Condition::Compare(AuthNullSentinel, Ne|Eq, NullLiteral)` branch —
  decided from `auth.is_some()`/`is_none()`, independent of filter shape.
- Anonymous-session handling reusing ADR-026's existing "attach nothing"
  semantics unchanged.

## OUT Scope
- Field-referencing rules (Slices 01/02, distinct branches).
- AND-composition of these shapes with a field-referencing rule (Slice 04).

## Learning Hypothesis
**Disproves**: "A rule that doesn't reference document fields at all cannot
be decided for queries using only `request.auth`, independent of filter
shape." Confirmed false if a signed-in caller's unfiltered query is admitted
against `request.auth != null` while an anonymous caller's identical query
is rejected, and a `true`-rule query is admitted regardless of identity.

## Acceptance Criteria
- AC-17-57: A signed-in caller's query is admitted against
  `request.auth != null`, with or without a filter.
- AC-17-58: A never-signed-in caller's query is rejected against the same
  rule.
- AC-17-59: A query against `allow read: if true` is admitted regardless of
  identity or filter shape.
- AC-17-60: An invalid identity header is evaluated identically to no header
  at all.

## Dependencies
- Slice 01 (shared compliance function scaffold).
- `client-auth`'s `attach_client_identity_if_present()` (unchanged, reused).

## Reference Class
Mirrors `security-rules`' US-03 (anonymous-session `GetDocument`
evaluation) almost exactly — same rule shapes, applied to `RunQuery`.

## Pre-Slice SPIKE
Not required.
