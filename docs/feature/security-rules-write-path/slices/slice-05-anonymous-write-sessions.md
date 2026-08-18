# Slice 05: A Session With No Verified Identity Is Evaluated as Anonymous on Writes

**Story**: US-05 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1 day

## Goal
Wire the existing, unmodified `attach_client_identity_if_present()` into `handle_create_document`, `handle_update_document`, and `handle_delete_document` (currently called by none of them), so `request.auth` is meaningful on every gated write call.

## IN Scope
- Call `attach_client_identity_if_present()` from all three write handlers, exactly as `handle_get_document` already does — no modification to the function itself.
- Thread the resulting `Option<VerifiedEndUserIdentity>` into Slices 02–04's evaluation calls as `Option<AuthContext>`, reusing the exact translation `security-rules`' ADR-029 already established (`v.end_user_id` → `AuthContext.uid`).
- Confirm invalid (malformed/expired/wrong-project) identity headers on write calls are evaluated identically to absent headers — reusing `client-auth`'s existing ADR-026 "attach nothing" semantics unchanged.

## OUT Scope
- Any change to `attach_client_identity_if_present()`'s own logic or to `client-auth`'s credential-verification code.
- Query/listen identity wiring (out of this feature's scope entirely — 2c/2d).

## Learning Hypothesis
Disproves: extending `attach_client_identity_if_present()` to three new call sites cannot be done without either modifying the function itself (risking `client-auth`'s own regression suite) or inventing a second identity-resolution path for writes.
Confirms (if it passes): the function's existing additive-only, "attach nothing on absent/invalid, never reject" contract composes cleanly with new consumers, exactly as `security-rules`' own single new consumer (`handle_get_document`) already proved for reads.

## Acceptance Criteria
- AC-17-39, AC-17-40, AC-17-41.

## Dependencies
- Slices 02–04 (the evaluation call sites this identity threads into must exist).

## Production-Data Taste Test
Real never-signed-in sessions and real sessions presenting malformed/expired/wrong-project client-identity headers, against real write-rule-gated `journal_entries` and `trail_guides` collections.

## Reference Class
Direct precedent: `security-rules`' own US-03 (`docs/feature/security-rules/slices/slice-03-anonymous-session-evaluation.md`), now applied to three new call sites instead of one.
