# ADR-024: Client-Identity Token Verification Mechanism

## Status

Accepted

## Context

`client-auth` (JOB-16) requires embyr to verify a "custom token" that a customer's own
backend mints per end user, without embyr ever custodying an end-user password
(DISCUSS § Framing Resolution, locked option A). The mechanism question DISCUSS
explicitly deferred to DESIGN: what does a customer register with embyr, what format is
the token, and what algorithm verifies it?

Two hard constraints from DISCUSS carry directly into this decision:

1. **No impersonation capability.** § System Constraints: "Whatever verification
   material DESIGN chooses ... must be something embyr can use to *verify* a token
   Trailmark's backend signs, not something that lets embyr *impersonate* Trailmark's
   own identity system or *learn* an end-user credential."
2. **Additive, not a replacement** for the existing `api_key` (see ADR-026 for the
   request-path composition decision this ADR feeds).

A relevant fact discovered during read-confirmation: the Firebase JS SDK's
`signInWithCustomToken(auth, token)` call treats the token as an **opaque string** — the
client SDK performs no local validation or algorithm inspection before POSTing it to the
verifying backend. This means the algorithm and wire format are a private contract
between the customer's minting backend and embyr's verifier; the SDK does not constrain
the choice (unlike, say, a browser enforcing a TLS cipher suite).

## Decision Drivers

1. No impersonation: embyr must never hold anything that lets it *mint* a token that
   verifies as authentic for a project it does not control.
2. No new custody liability: registered material must not itself be a secret whose
   disclosure lets an attacker do more than *verify* (already satisfied by driver 1 if
   asymmetric).
3. Reuse over reinvention: the workspace already depends on `jsonwebtoken` v10
   (`aws_lc_rs` backend) for RS256/JWKS verification in the admin-console OIDC path
   (`admin/handlers/auth.rs::oidc_callback`).
4. Operational simplicity for V1 (team size, timeline) — avoid introducing a new
   outbound network call (JWKS fetch) on the sign-in hot path.
5. Team's own established precedent: AD-03 in the Application Architecture section
   already rejected "short-lived JWT issued after first Argon2id verify" for the
   *existing* `api_key` on the grounds that it "adds token issuance, rotation, and
   revocation logic not present in SPEC" — the same simplicity bias applies here.

## Considered Options

### Option A: Shared secret (HMAC, e.g. HS256)

Trailmark registers a symmetric secret; its backend signs with HS256; embyr verifies
with the same secret.

**Rejected.** The verification material *is* the signing material. Any read access to
`client_identity_credentials` (a bug, an insider, a backup snapshot) grants the ability
to *mint* valid tokens for that project — a direct violation of driver 1 and the
DISCUSS-locked non-impersonation constraint. This is a materially different security
posture from Argon2id-hashed `api_key`s (`api_key`'s Argon2id hash cannot be used to
*derive* a valid `api_key`; an HMAC shared secret can trivially be used to mint new valid
tokens).

### Option B: Customer-hosted JWKS endpoint (RS256, discovery-based)

Trailmark registers a JWKS URL; embyr fetches and caches the customer's public keys,
verifying RS256 tokens against them — the exact shape already coded in
`oidc_callback` (`DecodingKey::from_jwk`, `kid` matching).

**Rejected for V1, not rejected in principle.** This adds a new outbound HTTP dependency
on the sign-in hot path (fetch/cache/refresh JWKS, handle the customer's discovery
endpoint being unreachable, stale, or slow) — a new Earned Trust probe surface
(`JwksFetcher.probe()`, cache staleness handling) disproportionate to a V1 walking
skeleton. It also requires Trailmark to stand up and operate a discovery endpoint, which
is a heavier integration lift than pasting a public key into an admin API call — directly
working against JOB-16's push force (make multi-user migration *tractable*). The
`oidc_callback` code is confirmed reusable as a *reference pattern* only (per DISCUSS
reading confirmation), not adopted directly, because it is architecturally shaped for a
browser-session admin login, not a stateless per-request verifier (see ADR-026).
Flagged as a plausible V2 upgrade path if a customer segment specifically needs key
rotation without an admin API call (Open Questions).

### Option C: Directly-registered asymmetric public key (Ed25519 / EdDSA) — Accepted

Trailmark generates an Ed25519 keypair, keeps the private key on its own backend
(mints tokens with it, never shares it), and registers only the 32-byte **public** key
with embyr via the admin API. embyr verifies signatures against the registered public
key. No network fetch, no discovery endpoint, no shared secret.

## Decision

**Option C: Ed25519 (EdDSA) signature over a JWT-shaped envelope, verified against a
directly-registered public key.**

- **Token format**: standard JWT (`header.payload.signature`, base64url, JSON claims) —
  reuses `jsonwebtoken` (already a workspace dependency) rather than a bespoke binary
  envelope. Any language's JWT library (Node `jsonwebtoken`, PyJWT, `jjwt`, etc.)
  supports EdDSA today, so Trailmark's backend is not constrained to a particular
  language or library. Chosen over a bespoke signed-envelope format because JWT parsing,
  claim validation, and expiry handling are already solved, audited library code — a
  bespoke format would duplicate that for no benefit, given the SDK-opacity fact above
  removes the "must match Firebase's exact custom-token format" pressure.
- **Claims** (minimum required):
  - `sub` — the end-user identifier (Maria Santos's app-level ID). Becomes
    `VerifiedEndUserIdentity.end_user_id`.
  - `aud` — the embyr `project_id` this token is minted for. Checked against the
    project_id in the verification request; mismatch is the `ProjectMismatch` rejection
    reason (US-02 AC-16-07).
  - `exp` — standard JWT expiry claim. Drives the `Expired` rejection reason.
- **Algorithm pinning**: verification MUST reject any token whose header `alg` is not
  exactly `EdDSA`. This is a deliberate defense against the well-known JWT
  algorithm-confusion class of attack (e.g., a verifier that accepts both RSA/EdDSA and
  HMAC can be tricked into treating a *public* key's bytes as an HMAC secret, allowing an
  attacker who knows the public key — which is, by definition, not secret — to forge a
  valid signature). Because the registered material here is explicitly public
  (Option C's entire security argument), a verifier that is not strictly single-algorithm
  is not just weaker but actively broken. `jsonwebtoken::Validation::new(Algorithm::EdDSA)`
  enforces this in one call; DELIVER must not construct a `Validation` that accepts a
  broader algorithm set for this verifier.
- **Verification is stateless and per-request** — no embyr-issued second-order session
  token is minted (see ADR-026 for why: this mirrors and directly reuses AD-03's already
  -accepted rationale for the existing `api_key` check, and eliminates an entire class of
  session-issuance/rotation/revocation machinery this feature does not need).
- **Rejection taxonomy** (AC-16-07's four distinguishable reasons) maps directly onto
  `jsonwebtoken`'s own `ErrorKind`:

  | AC-16-07 reason | Condition | `jsonwebtoken::ErrorKind` (implementation pointer, not mandated API) |
  |---|---|---|
  | Missing token | No token presented at all | N/A — checked before calling `decode()` |
  | Malformed | Structurally invalid JWT, OR signature verification fails under both current and previous registered keys (ADR-025) | `InvalidToken`, `Base64`, `Json`, `InvalidSignature` |
  | Expired | Structurally valid, correctly signed, `exp` in the past | `ExpiredSignature` |
  | Wrong project | Structurally valid, correctly signed, `aud` does not match the project being verified against | `InvalidAudience` |

  Signature validity is always checked before expiry/audience are trusted — an
  attacker-forged token with an arbitrary `aud`/`exp` and an invalid signature must never
  be reported as `ProjectMismatch` or `Expired`; it is `Malformed`.

## Consequences

### Positive

- Structurally satisfies the non-impersonation constraint: `client_identity_credentials`
  contains only public keys. A full table read (breach, misconfigured backup,
  insider) grants an attacker nothing beyond what Trailmark already made public by
  registering it — it cannot be used to mint a single valid token.
- Zero new outbound network dependency, zero new Earned Trust probe on the sign-in hot
  path (contrast Option B).
- Reuses an already-vetted, already-a-dependency library (`jsonwebtoken`) rather than
  adding a new crate or a bespoke format.
- Ed25519 verification is fast (tens of microseconds), cheap enough to run per-request
  with no cache required for V1 correctness (a future performance optimization, not a
  correctness requirement — see ADR-026).

### Negative / Trade-offs

- Departs from Firebase's real custom-token format (RS256, Google-specific claims). This
  is acceptable *only* because of the SDK-opacity fact established in Context — DELIVER
  and DISTILL must not assume any other part of the Firebase Auth wire contract is
  similarly free to diverge without the same opacity argument applying (see ADR-026 Open
  Question on sign-in transport fidelity).
- No customer-side key-rotation-without-an-embyr-admin-call path (Option B would have
  given this). Rotation is handled entirely by US-03 / ADR-025's dual-key window,
  requiring an explicit admin API call — acceptable per DISCUSS's own no-automatic
  -expiry-timer precedent (ADR-018 D-SM-7) for the shape of secret rotation.

## Alternatives Considered

See Options A and B above (evaluated inline per MADR-style convention for this ADR).

## Enforcement

- Unit tests in `embyr-core::client_identity` assert: valid token verifies; each of the
  four AC-16-07 rejection reasons is independently triggerable and distinguishable;
  algorithm-confusion attempt (`alg: HS256` with the public key bytes as HMAC secret) is
  rejected as `Malformed`, not accepted.
- The verification function is pure (`embyr-core`, no IO) — testable without a database,
  network, or async runtime, consistent with the crate's zero-IO invariant enforced by
  `cargo-deny` (see AD-06 in the Application Architecture section).

## References

- `docs/feature/client-auth/feature-delta.md` §§ Framing Resolution, System Constraints
- `docs/product/architecture/brief.md` § Application-Level Decisions Table (AD-03)
- `crates/embyr-server/src/admin/handlers/auth.rs::oidc_callback` (reference pattern, not
  reused directly)
- RFC 8037 (EdDSA for JOSE), RFC 8725 (JWT Best Current Practices — algorithm confusion)
