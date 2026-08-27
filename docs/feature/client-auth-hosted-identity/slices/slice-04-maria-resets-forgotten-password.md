# Slice 04: Maria Resets Her Forgotten Password Without Contacting Alex

**Story**: US-04 | **Release**: 2 | **job_id**: JOB-18

## Goal

An end user who forgets her password can request and complete a self-service reset, using the existing `IEmailSender` port (ADR-011), with zero account-enumeration leak and single-use, time-bounded reset tokens.

## IN Scope

- Reset-request handler: always returns the identical generic "if this account exists, a reset was sent" response, regardless of whether the email is registered — extends AC-18-11's oracle-protection principle.
- Reset-token generation, storage, single-use enforcement, expiry.
- Reset-confirm handler: valid unexpired unused token + new password (meeting strength requirement) → password updated; rejected distinguishably for expired / already-used / malformed token, and for a new password below the strength requirement.
- Reuse of `crates/embyr-core/src/admin/email.rs::IEmailSender` and `crates/embyr-server/src/adapters/email.rs::NoopEmailSender` (V1) unchanged — no new email mechanism built.

## OUT of Scope

- Real SMTP delivery (`SmtpEmailSender` V2 adapter — cross-feature dependency on ADR-011's own already-named V2 slice, not built here).
- Session-invalidation-on-reset policy (whether a reset signs out other active sessions of the same user) — genuinely undecided, DESIGN's call.

## Learning Hypothesis

**Disproves**: a password-reset flow cannot be built on the existing `IEmailSender` port without the "reset requested" step leaking account-existence information via response-shape or timing differences.

**Confirms if it succeeds**: ADR-011's port abstraction is sufficient for this feature's needs with zero new email-delivery mechanism, and the oracle-protection discipline established in Slice 03 generalizes to a second, structurally distinct endpoint (reset-request) without special-casing.

## Acceptance Criteria

- [ ] AC-18-14: Reset-request always returns the identical generic response regardless of email registration status.
- [ ] AC-18-15: Valid unexpired unused token + strong-enough new password updates the credential; new password works for subsequent sign-in (US-03).
- [ ] AC-18-16: Expired token rejected, distinguishable from already-used/malformed.
- [ ] AC-18-17: A reset token can be consumed at most once.
- [ ] AC-18-18: "Send" step uses `IEmailSender` (ADR-011); V1 ships with `NoopEmailSender` — real delivery is a named, tracked cross-feature dependency.

## Dependencies

Depends on Slice 02/03 (account + credential storage must exist). Depends on the already-accepted `IEmailSender` port (ADR-011) — no new dependency introduced.

## Effort Estimate

1.5 days.

## Reference Class

Reuses ADR-011's `IEmailSender` port and V1→V2 adapter-swap composition-root pattern directly, matching the invitation flow's own already-accepted V1 scope (log-only, no real delivery, honestly flagged to Alex).

## Pre-Slice SPIKE

Not required — no new external wire-format uncertainty (email delivery is already abstracted behind a port with a working V1 adapter); `confirmPasswordReset()`'s own wire-format uncertainty is the same class already covered by Slice 02's spike.
