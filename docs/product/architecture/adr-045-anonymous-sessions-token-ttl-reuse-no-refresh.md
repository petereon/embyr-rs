# ADR-045: Anonymous Sessions — Reuse `TOKEN_TTL_SECS` Unchanged; Defer
# Refresh Tokens as Cross-Cutting Future Work

## Status

Accepted

## Context

DISCUSS's Escalation 1 (`docs/feature/anonymous-sessions/feature-delta.md`
§ Handoff Package — "the primary open question this DESIGN wave must
resolve") observes that no refresh-token mechanism exists anywhere in this
codebase, for any identity path. Every existing mechanism can re-establish a
session after token expiry by re-presenting SOME credential (a custom token
minted again by Trailmark's own backend; the same password; the same Google
account). Anonymous sign-in has nothing to re-present, by design — once its
token expires, calling `signInAnonymously()` again mints a brand-new,
different `end_user_id` (locked as accepted v1 behavior, AC-20-10). DISCUSS
asks DESIGN to choose among: (a) a materially longer TTL for anonymous
tokens specifically; (b) build a real refresh-token mechanism inside this
feature; (c) keep the existing TTL unchanged, document the residual gap
honestly.

Ground-truth: `TOKEN_TTL_SECS = 3600` (1 hour), `pub(crate)` in
`crates/embyr-server/src/rest/sign_up.rs:66`. Confirmed by direct grep and
read to be the single, shared constant already reused, unchanged, by
`sign_in_with_password.rs` and `sign_in_with_idp.rs` — every "embyr mints
its own token" flow shipped so far uses this identical constant, not three
independently-chosen values that happen to agree.

## Decision

**(c): reuse `TOKEN_TTL_SECS` unchanged. No new constant, no per-mechanism
TTL, no refresh-token mechanism built in this feature.** Anonymous sign-in's
minted token expires after exactly 3600 seconds, identically to hosted
identity's and Google sign-in's own tokens.

## Reasoning — Why Not (a)

A longer, anonymous-specific TTL was considered and rejected on three
grounds:

1. **It does not solve the problem it targets, only shrinks its frequency.**
   AC-20-10 already locks "re-sign-in after expiry mints a new identity" as
   accepted v1 behavior regardless of the TTL's numeric value — a longer TTL
   makes Maria's session survive longer before hitting the gap, but any
   session that outlives the TTL (a browser tab left open, a mobile app
   backgrounded overnight) still hits it. The fix is qualitative
   (session-resumability), not quantitative (a bigger number); a longer TTL
   is a knob on the wrong parameter.
2. **It introduces a new axis of per-mechanism configuration this codebase
   does not have anywhere else.** All three shipped "embyr mints" flows
   (hosted-identity signup, hosted-identity password sign-in, Google
   sign-in) share exactly one constant, defined once, `pub(crate)` for
   cross-module reuse (`sign_up.rs`'s own doc comment names this explicitly:
   "one source of truth, not a second hand-copied constant"). A
   fourth flow with its own, different TTL breaks that established
   uniformity for a partial benefit — a maintainer reasoning about "when
   does an embyr-minted token expire" would now need to know it depends on
   which of four mechanisms minted it, a new, surprising asymmetry with no
   corresponding new invariant justifying it (unlike, say, a security
   reason a shorter-lived token would need for a higher-risk mechanism —
   here the direction is the opposite, a *longer* TTL for the *zero-credential*
   mechanism, the weakest-proof one of the four, which if anything argues
   for parity or a *shorter* TTL, not a longer one).
3. **Ponytail/simplest-first**: `TOKEN_TTL_SECS.to_string()` is already
   `pub(crate)` and reused verbatim by two sibling flows with zero
   modification. Reusing it a third time for anonymous sign-in is the
   laziest correct choice available; inventing `ANONYMOUS_TOKEN_TTL_SECS`
   would be a new constant, a new decision about its value (with no
   evidence to ground a specific number beyond "bigger"), and a new
   asymmetry to document — for a problem it does not actually solve (point 1).

## Reasoning — Why Not (b)

A genuine refresh-token mechanism is very likely the architecturally correct
fix for "session stability across an extended session" — but building it
scoped to this feature alone would be wrong on two counts:

1. **It is cross-cutting, not anonymous-specific.** Every existing identity
   mechanism has the identical "must re-present a credential after expiry"
   limitation; anonymous sign-in's version of the problem is more visible
   (it has *nothing* to re-present) but not architecturally distinct from
   hosted-identity's or oauth's own versions (Maria still has to retype her
   password, or re-click through Google's consent screen, after her token
   expires — an inconvenience, not a data-loss bug, for those three, but
   still the same underlying missing capability). A refresh mechanism scoped
   to only anonymous sessions would be a partial, inconsistent fix,
   mirroring exactly the "instrumenting one of five identically-exposed
   endpoints" anti-pattern ADR-043 Decision 7 already named and rejected
   for REST rate-limiting.
2. **It is a substantially larger investment than this feature's own Elephant
   Carpaccio sizing supports.** DISCUSS's own Scope Assessment sized this
   feature at 2 slices, ~3 days, explicitly smaller than
   `client-auth-hosted-identity`. A real refresh-token mechanism — new token
   type, new storage, new revocation semantics, new verification-time
   routing changes across all four mechanisms — is easily its own
   multi-slice feature. Building it here would silently balloon this
   feature's scope well past what DISCUSS sized and what the Elephant
   Carpaccio gate passed.

## Decision Recorded, Not Deferred Silently

This ADR does not merely decline to build a refresh mechanism — it names the
follow-up explicitly, as DISCUSS itself asked ("This DISCUSS does not choose
among these — it is flagged because it directly determines whether this
feature's own core promise is actually met"): a genuine refresh-token
mechanism, if ever built, is a **cross-cutting feature affecting all four
identity mechanisms** (`client-auth`, `client-auth-hosted-identity`,
`oauth-providers`, `anonymous-sessions`), not a follow-up scoped to
`anonymous-sessions` alone. Candidate future work, not built here, not
silently implied as "this feature's job."

AC-20-10 itself remains this feature's own honest documentation of the
resulting v1 limitation — a required, locked acceptance scenario, not a gap
this ADR papers over.

## Consequences

### Positive

- Zero new constant, zero new configuration surface, zero deviation from
  the established one-TTL-for-all-"embyr-mints" convention.
- The residual gap (re-sign-in after expiry mints a new identity) is
  honestly documented at the AC level (AC-20-10), not silently narrowed by
  a longer TTL that would only make the gap rarer and harder to notice in
  testing.
- Keeps this feature's own scope inside its Elephant Carpaccio sizing —
  no silent scope growth into a cross-cutting refresh-mechanism feature.

### Negative / Trade-offs

- Maria's anonymous session survives exactly 1 hour before her next
  `signInAnonymously()` call silently starts a fresh identity — a real,
  named product limitation for any Trailmark-class app whose guest sessions
  commonly outlive an hour (e.g., a long browsing session, an overnight-open
  tab). This is the accepted cost of Decision (c), not an oversight.
- A future refresh-token feature, when built, will need to design around
  anonymous sign-in's specific constraint that there is no credential to
  re-present at all (unlike the other three mechanisms) — named here as a
  design input for that future feature, not solved by it being named.

## References

- `docs/feature/anonymous-sessions/feature-delta.md` § Handoff Package,
  Escalation 1; § Elephant Carpaccio Slices (sizing)
- `docs/product/architecture/adr-043-anonymous-sessions-signing-key-custody-and-driving-port.md`
- `crates/embyr-server/src/rest/sign_up.rs:66` (`TOKEN_TTL_SECS` definition
  and doc comment)
- `crates/embyr-server/src/rest/sign_in_with_password.rs`,
  `crates/embyr-server/src/rest/sign_in_with_idp.rs` (existing reuse sites)
