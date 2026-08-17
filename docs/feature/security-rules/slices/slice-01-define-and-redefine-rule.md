# Slice 01: Define and Redefine an Access-Control Rule (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **Estimate**: 1.5 days | **job_id**: JOB-17

## Goal
Alex can define a per-collection access-control rule via the admin API, and redefine (fully replace) it later with the same action — no register-then-rotate lifecycle.

## IN Scope
- Admin-API action to define/redefine a rule for a named collection within a project.
- Syntax validation against the v1 grammar (comparison `==`/`!=`, boolean `&&`/`||`/`!`, `request.auth`/`resource.data.<field>`, literals).
- Explicit rejection, naming the reason, for constructs outside v1 scope (`get()`, `exists()`, functions, wildcard paths) — distinguishable from a plain syntax error.
- Idempotent replace semantics: redefining a collection's rule fully replaces the prior condition immediately, no overlap window.
- Standard admin-auth rejection (401 missing/invalid credential, 404 unknown/deleted project), consistent with existing admin-endpoint conventions.
- Never storing/echoing anything requiring confidentiality — rule text is not a secret (unlike `client-auth`'s credential), so no "never echo back" constraint applies here.

## OUT Scope
- Evaluating the rule against any real request (Slices 02–04).
- Simulating a rule before it's live (Slice 05).
- Full Firestore Rules Language parity (`get()`/`exists()`/functions/wildcards) — locked out for v1, see feature-delta.md § Job Discovery Framing Resolution, Resolution 1.
- Rule history/versioning/rollback — deferred to a future operational-maturity epic (candidate `security-rules-operations`).

## Learning Hypothesis
**Disproves if it fails**: a per-collection rule cannot be defined and idempotently redefined, with syntax validated against the v1 grammar, using the existing admin-API auth/response conventions (Owner/Admin session auth, `sdk_keys.rs`/`client_identity.rs`-shaped handlers) without inventing a materially new pattern.
**Confirms if it succeeds**: the existing admin-handler shape (session auth + validate + store + confirm) generalizes cleanly to a rule-definition action with no new auth mechanism.

## Acceptance Criteria
- AC-17-01: Valid first-time rule definition is stored and active.
- AC-17-02: Redefining fully and immediately replaces the prior condition — no merge, no overlap window.
- AC-17-03: Out-of-v1-grammar constructs rejected with a specific, distinguishable reason.
- AC-17-04: Invalid syntax rejected with a specific reason.
- AC-17-05: Missing/invalid admin credential rejected 401.

## Dependencies
- `client-auth` (DONE, merged) — no direct code dependency for this slice specifically, but establishes the admin-handler conventions this slice reuses (`admin/handlers/sdk_keys.rs`, `admin/handlers/client_identity.rs` shape).
- None blocking.

## Effort Estimate
1.5 days. Reference class: `client-auth` US-01 (credential registration, 1 day) plus incremental cost for grammar validation (new, not present in any existing admin handler).

## Pre-Slice SPIKE
Not required — the v1 grammar's scope is locked at DISCUSS (Resolution 1); DESIGN selects the parsing/validation mechanism, which is a bounded, well-understood problem (small boolean-expression grammar), not an open unknown warranting a spike.
