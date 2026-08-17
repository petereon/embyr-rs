# Evolution: client-auth

**Date:** 2026-08-17
**Feature:** Firebase-style custom-token end-user identity verification — a new,
additive identity-verification layer that lets a customer's own backend mint
Ed25519-signed tokens for its end users, which embyr can then verify per-request
without ever custodying end-user credentials itself.
**Job:** JOB-16 (`client-identity-verification`), new job — P1 Alex (SDK Developer)
**ADRs:** ADR-024 (`docs/product/architecture/adr-024-client-identity-verification-mechanism.md`),
ADR-025 (`docs/product/architecture/adr-025-client-identity-credential-storage-rotation.md`),
ADR-026 (`docs/product/architecture/adr-026-client-identity-composition-with-api-key-auth.md`)

## Business Context

embyr's `api_key` bearer credential serves three simultaneous roles: project
identification, request authorization (Argon2id verify), and ECIES decryption key
material for the stored customer DSN/agent mTLS bundle. That three-role overload
surfaced during an unrelated user question about how embyr's `api_key` differs from
Firebase's actual public, non-secret `apiKey` — the answer exposed that embyr had
**no end-user identity layer at all**, unlike real Firebase (ID tokens + Security
Rules). This was not an oversight: `docs/feature/embyr-rs/discuss/feature-delta.md`
named it a deliberate founding exclusion ("Firebase Authentication integration ... no
Firebase Auth service dependency"). The user explicitly asked to close the gap,
sequenced as: Client Auth first (this feature, provides caller identity), Security
Rules second (a separate, larger, dependent follow-up epic — not started, deferred
because it needs the identity this feature now provides).

**Deliberate reversal of a founding decision, not scope creep.** DISCUSS documented
this explicitly (§ Job Discovery Framing Resolution) rather than silently changing
course, and justified it with fresh JTBD evidence: JOB-04 (Riley, CISO,
credential-isolation) and JOB-09 (agent-auditproof) show embyr's most
security-conscious segment actively minimizes third-party custody of secrets — ruling
out embyr becoming a password custodian. JOB-15 (Elena, customer-db-preflight)
independently confirms embyr's technical customer segment already runs its own
backend infrastructure capable of minting a custom token today. On that evidence,
v1 scope was locked to **custom-token-only verification** (mirrors Firebase's
`signInWithCustomToken()` bridge) — embyr verifies a token minted by the customer's
own backend and never becomes an identity provider itself. Embyr-hosted
email/password auth (Option B) was evaluated and explicitly deferred as a named
follow-up epic candidate, not ruled out.

## Key Decisions

### DESIGN-wave decisions (ADR-024, ADR-025, ADR-026)

| ID | Decision | Verdict |
|----|----------|---------|
| DDD-CA-1 | Verification mechanism: directly-registered Ed25519 public key, JWT envelope, EdDSA-only | ADR-024 |
| DDD-CA-2 | Credential storage: new `client_identity_credentials` table, plaintext public key, current/previous rotation columns | ADR-025 |
| DDD-CA-3 | Identity-carrying mechanism: stateless per-request re-verification of the original customer-minted token; no embyr-issued session token | ADR-026 |
| DDD-CA-4 | Wire composition: new `x-embyr-client-identity` metadata key/header, additive to the unchanged `authorization`/`api_key` slot | ADR-026 (OQ-CA-01 flags empirical-fidelity risk) |
| DDD-CA-5 | Data-plane failure mode on invalid/expired identity header: attach nothing, never reject the underlying data call | ADR-026 |
| DDD-CA-6 | Sign-in endpoint transport: new REST endpoint on `:8081`, Identity-Toolkit-shaped naming (`accounts:signInWithCustomToken`) | ADR-026 (provisional pending OQ-CA-01 spike) |
| DDD-CA-7 | Bounded context placement: BC-1 Tenant Management extension, no new context | matches ADR-002 |
| DDD-CA-8 | `jsonwebtoken` becomes an `embyr-core` dependency for the first time (previously `embyr-server`-only) | verified IO-free; `cargo deny check bans` confirmed clean |

Reuse verdict: 6 EXTEND / 2 CREATE NEW (both justified — the new table mirrors the
existing `sdk_api_keys`-own-table precedent; the new sign-in call site is
architecturally required, ADR-024 rejected reusing the browser-session-shaped
`oidc_callback` code directly). Zero unjustified CREATE NEW.

### The mid-DESIGN targeted security review (notable finding: none — clean pass)

Unlike a prior feature in this codebase where a mid-DESIGN security review caught and
revised an over-broad grant, this feature's targeted review — a proactive, narrowly
scoped pass over ADR-024/025/026's security-critical points (algorithm-confusion
defense, the structural-unreachability claim for AC-16-08, verification ordering
[signature validity checked before expiry/audience are trusted], the rotation-window
and replay surface, and credential-fingerprint non-invertibility) — returned
**APPROVED with zero issues**, run before DISTILL/DELIVER began. All three ADRs carry
a "security-review-approved" annotation confirmed present in DISTILL's own
prior-wave reading confirmation. Two independent later checkpoints reconfirmed the
same properties held all the way through implementation: the mandatory
end-of-DISTILL consolidated review (covering all four waves), and the post-DELIVER
adversarial review (APPROVED, zero blocking findings). No security-relevant
production-code change was required at any of the three checkpoints — the design
held as specified.

### AC-16-08 — structural unreachability, not just untested reachability

The single most load-bearing design property in this feature: a request carrying
**no** `x-embyr-client-identity` header structurally cannot reach the new
verification code path in `grpc/handler.rs::authenticate()` — the new step 4 is
gated entirely on the header's presence, appended strictly after the existing
unchanged three-role `api_key` check completes. This is what let the feature ship as
a strictly additive change with zero risk to the pre-existing 72-scenario regression
suite: an expired/invalid identity header attached to an ordinary `getDoc` call never
turns a previously-working call into a failure (AC-16-08(b)), and a session that
never presents the header never enters the new branch at all (AC-16-08(c)), proven
by a real gRPC integration test rather than by convention. `AC-16-14`'s debug-verify
handler (US-04) received the identical construction-not-convention treatment: it is
read-only by construction (loads credential, calls the pure verify function, returns
the result — no code path touches the `sessions` table or mutates
`client_identity_credentials`), independently proven by a real row-count assertion
across the call.

## Steps Completed

All 5 roadmap steps (`docs/feature/client-auth/deliver/execution-log.json`) show
complete `PREPARE → RED_ACCEPTANCE → GREEN → COMMIT` DES traces (3 steps legitimately
`SKIPPED` the `RED_UNIT` phase with a documented `NOT_APPLICABLE` reason — thin
wiring over already-tested pure logic from 01-01, not a coverage gap).

| Step | Name | Status |
|------|------|--------|
| 01-01 | Walking Skeleton (US-01) — `embyr-core::client_identity` pure module (`verify_client_identity_token` + `credential_fingerprint`, 14 unit/property tests) + credential registration admin handler | PASS |
| 02-01 | Walking Skeleton (US-02) — REST sign-in handler (`signInWithCustomToken`), all 4 rejection reasons, ADR-024 algorithm-confusion regression (ca05) | PASS |
| 02-02 | `authenticate()` gRPC extension — additive, structurally-unreachable-without-the-header client-identity attach step (AC-16-08 guardrail) | PASS |
| 03-01 | Credential rotation — `system_db` rotate mutation + admin rotate handler, dual-generation window | PASS |
| 04-01 | Standalone debug-verify check — read-only wrapper over the shared verify function | PASS |

`des-verify-integrity docs/feature/client-auth/deliver/` reports exit 0: "All 5 steps
have complete DES traces."

Post-roadmap hardening, all on `master`:
- L1-L6 refactor pass (`201d051`, `911d279`) — dropped stale RED-scaffold/panic
  comments, extracted duplicated public-key length validation, extracted a `rotate()`
  test helper.
- Adversarial review — APPROVED, zero blocking findings.
- Mutation-testing hardening (`214a8ca`, `f9022e5`, `2c6b7d0`) — see below.

## Scenarios (verified by direct count, not by trusting a prior summary)

**24 acceptance scenarios**, counted directly from `tests/client_auth/acceptance/*.rs`
(excluding shared test-helper functions such as `sign_in()`/`rotate()`):

| Suite | Count | Notes |
|-------|-------|-------|
| ca01 (register) | 6 | US-01, AC-16-01..05 |
| ca02 (sign-in + AC-16-08 guardrail) | 7 | US-02, AC-16-06/07/08 |
| ca03 (rotate) | 6 | US-03, AC-16-10..13; includes 2 new boundary tests added by mutation hardening (originally 4) |
| ca04 (standalone verify) | 4 | US-04, AC-16-14/15 |
| ca05 (algorithm-confusion regression) | 1 | ADR-024 Enforcement |

Plus 14 layer-1 unit/property tests in `crates/embyr-core/src/client_identity/mod.rs`
(3 of the 14 are `proptest!` properties at 64 cases each: claims round-trip,
past-expiry always rejected, project-mismatch always rejected), and 3 pre-existing,
untouched, legitimately-GREEN pure-routing unit tests in `grpc/handler.rs` (not new
to this feature).

Total: 41 tests directly exercising this feature's code (24 acceptance + 14 unit/property
+ 3 pre-existing routing tests confirmed still passing).

Full mandatory 72-scenario `embyr-rs` regression suite (`us_01`..`us_14` +
`walking_skeleton`): confirmed 0 regressions, exit code 0, immediately before this
finalize dispatch.

## Mutation Testing (`per-feature`, per `CLAUDE.md`)

`cargo-mutants -p embyr-core --filter client_identity`, plus a targeted pass over the
new acceptance-test assertions, found and closed **3 genuine gaps** — all fixed with
new/strengthened tests, not production-code changes, since the production logic was
already correct; the tests hadn't pinned it tightly enough:

1. **`expiresIn` presence-only assertion.** ca02's happy-path sign-in scenario
   asserted `expiresIn` was present but never checked its value — a mutant turning
   `expires_at_unix - now` into `+` or `/` would have survived silently. Fixed
   (`f9022e5`) to assert the actual remaining-seconds value falls in `3590..=3600`.
2. **Rotate's own Viewer-role gate, under-covered.** `rotate()`'s
   `session.role < Role::Admin` check is a *separate source occurrence* from
   `register()`'s identical guard (not shared code) and had no dedicated test. Fixed
   (`f9022e5`) with `a_viewer_role_cannot_rotate_a_verification_credential`.
3. **Rotate's Admin-exactly boundary, under-covered.** Every existing rotate scenario
   used an Owner-role session (`=3`), which passes under either `<` or `<=` against
   `Role::Admin` (`=2`) — so the mutant `<` → `<=` survived with zero test failures.
   Only a session at exactly `Role::Admin` distinguishes the two operators. Fixed
   (`2c6b7d0`) with `an_admin_role_session_can_rotate_a_verification_credential`.

A separate finding was investigated and **documented as a caveat rather than an open
defect**: one `cargo-mutants` "survived" report on a full-function-body-replacement
mutant against `verify_client_identity_credential` (the ca04 debug-verify handler).
Direct code inspection shows all 4 ca04 acceptance scenarios collectively already pin
every branch of that function's behavior — a `Default::default()` response cannot
simultaneously satisfy the distinct 200/400/400/4xx assertions those 4 tests make —
and manually re-running the real (unmutated) suite showed all 4 passing/failing
exactly as expected. This is very likely a tooling/shared-`CARGO_TARGET_DIR`
build-caching false-negative rather than a genuine coverage gap, consistent with the
`us_12_agent_backend` harness issue found the same session (below). Recorded here
plainly rather than hidden or silently closed.

## Incidental Fix Found During Verification

**`us_12_agent_backend` test-harness fix (`a700187`, landed just before this
feature's DELIVER work began).** 3 pre-existing tests were failing because this
project's shared `CARGO_TARGET_DIR` config broke a hardcoded `target/debug/<bin>`
path guess for a cross-crate binary (`embyr-server`'s test target building
`embyr-agent`'s `[[bin]]`). Fixed by parsing `cargo build`'s own JSON artifact output
for the `executable` field instead of guessing the path. Found while independently
verifying this feature's own claim that the existing 72-scenario suite passes
unmodified — it did not, until this fix. Confirmed unrelated to `client-auth` via
`git stash` reproduction against the clean tree.

## Quality Gates

- **Per-step TDD:** 5/5 steps COMMIT/PASS, all DES traces complete.
- **`des-verify-integrity`:** exit 0, "All 5 steps have complete DES traces."
- **Refactor L1-L6 (`201d051`, `911d279`):** stale RED-scaffold comments removed,
  duplicated public-key-length validation extracted, migration-coverage test renamed
  and extended to assert `client_identity_credentials` presence (`214a8ca`).
- **Adversarial review:** APPROVED, zero blocking findings.
- **Mutation testing (`per-feature`):** 3 genuine gaps found and closed (above); 1
  investigated survivor documented as a probable tooling false-negative, not hidden.
- **AC-16-08(a) regression gate, re-run at both required checkpoints (after 02-02,
  and again pre-DELIVER):** 69/72 pre-existing `embyr-rs` scenarios passed
  unmodified pre-fix (3 confirmed pre-existing/unrelated via `git stash`); 72/72
  post-fix (`a700187`). `admin_api_v2`'s 93 scenarios (the other consumer of
  `build_admin_router`, extended with 3 new session-router routes) also run as extra
  coverage: 93/93 passed unmodified. Full mandatory 72-scenario suite re-confirmed
  green immediately before this finalize dispatch.
- **`cargo deny check bans`:** `bans ok` — `embyr-core`'s new `jsonwebtoken`
  dependency (DDD-CA-8) confirmed not to violate the IO-prohibition ban list.
- **Security reviews (3 independent checkpoints, all clean):** mid-DESIGN targeted
  review (APPROVED, 0 issues), end-of-DISTILL consolidated review, post-DELIVER
  adversarial review (APPROVED, 0 blocking findings) — see § Key Decisions above.

## Open Questions

- **OQ-CA-01 (open, flagged for follow-up spike)** — whether the real closed-source
  Firebase JS SDK's `signInWithCustomToken()` actually POSTs to a URL embyr controls
  at the exact path shape implemented
  (`/v1/projects/{project_id}/accounts:signInWithCustomToken`, with the intentional
  literal `:` mirroring Firebase's real REST convention), and whether subsequent
  calls reuse the existing `Authorization` header vs. a new one. Documented fallback
  if the assumption is wrong: a `dual_auth`-style single-header precedence check,
  reusable from ADR-009's existing `dual_auth_middleware` shape. Requires an
  empirical spike against the real SDK — not yet done. Does not block this feature;
  `embyr-core::client_identity`'s logical verification contract is transport-independent.
- **OQ-CA-02** — Should `VerifiedEndUserIdentity` verification results be cached
  (keyed by token hash, mirroring `CredentialCache`)? Not required for V1
  correctness (Ed25519 verify is cheap); a pure performance follow-up once real
  traffic volume is known.
- **OQ-CA-03** — Is embyr-hosted email/password (Framing Resolution Option B) fully
  out of scope for all future work, or deferred-but-eventually-needed? Does not
  block this feature; triggered by future customer-segment evidence, not a code
  concern.

## Lessons Learned

1. **A targeted, narrowly-scoped security review before DISTILL/DELIVER is worth
   running even when it comes back clean.** This feature's mid-DESIGN review found
   nothing to fix — a genuinely different outcome from other features in this
   codebase where the equivalent review caught a real issue — but the same
   discipline (checking algorithm-confusion defense, verification ordering, and the
   structural-unreachability claim explicitly, before implementation begins) is what
   let two later independent checkpoints (end-of-DISTILL, post-DELIVER adversarial)
   reconfirm the same properties held through real code, rather than assume it.
2. **"Structurally unreachable" is a stronger and more testable claim than
   "untested."** AC-16-08's and AC-16-14's guarantees were designed as code-shape
   properties (a branch that cannot be entered without a specific header; a handler
   that cannot write to certain tables by construction) and proven with pointed
   integration tests, not property-based exploration — the right tool varies by what
   kind of guarantee is being made.
3. **Mutation testing keeps finding the same shape of gap: assertions that check
   presence, not value, and boundary conditions no existing test happens to reach.**
   Two of three genuine gaps this feature (the `expiresIn` value and the
   Admin-exactly rotation boundary) fit a now-recurring pattern in this codebase's
   mutation-testing phase.
4. **A reported mutation survivor is not automatically a real gap.** The
   `verify_client_identity_credential` "survived" report was investigated rather than
   either dismissed or treated as a blocking defect — direct code inspection and a
   manual full-suite re-run both supported the tooling-false-negative explanation,
   and that reasoning (not just the conclusion) is documented above.
5. **Always independently re-verify a DISTILL-wave "existing suite passes unmodified"
   claim before trusting it into DELIVER.** It did not, on first check — a
   pre-existing, unrelated test-harness bug (shared `CARGO_TARGET_DIR` breaking a
   hardcoded binary path) was masking as 3 client-auth-adjacent failures until fixed.

## Key Files

- `crates/embyr-core/src/client_identity/mod.rs` — pure `verify_client_identity_token()`,
  `credential_fingerprint()`, `ClientIdentityVerifyError`, 14 unit/property tests
- `crates/embyr-server/src/adapters/system_db.rs` — `insert_client_identity_credential`,
  `get_client_identity_credential`, `rotate_client_identity_credential`
- `crates/embyr-server/src/admin/handlers/client_identity.rs` — register/rotate/verify
  admin handlers
- `crates/embyr-server/src/admin/handlers/shared.rs` — promoted `verify_project_ownership`
  (shared with `sdk_keys.rs`)
- `crates/embyr-server/src/rest/sign_in.rs` — `signInWithCustomToken` REST handler
- `crates/embyr-server/src/grpc/handler.rs` — `authenticate()` step 4 (AC-16-08 guardrail)
- `crates/embyr-server/migrations/0021_client_identity_credentials.sql`
- `docs/product/architecture/adr-024-client-identity-verification-mechanism.md`
- `docs/product/architecture/adr-025-client-identity-credential-storage-rotation.md`
- `docs/product/architecture/adr-026-client-identity-composition-with-api-key-auth.md`
- `docs/product/architecture/brief.md` § Application Architecture — client-auth
- `tests/client_auth/acceptance/` — 24 acceptance scenarios (ca01-ca05) + shared
  `common/mod.rs` harness
- `docs/feature/client-auth/feature-delta.md` — full DISCUSS+DESIGN+DISTILL narrative
  (retained in place, not migrated — this project's established SSOT convention)
- `docs/feature/client-auth/distill/red-classification.md` — fail-for-right-reason
  gate results
- `docs/feature/client-auth/slices/` — 4 elephant-carpaccio slice briefs

## Follow-Up Work

- **OQ-CA-01** — empirical spike against the real Firebase JS SDK to confirm
  `signInWithCustomToken()` wire fidelity; fallback design already documented (above).
- **Security Rules epic** — remains queued, not started. Explicitly sequenced by the
  user to follow this feature, since it needs the caller identity `client-auth` now
  provides. `VerifiedEndUserIdentity` is attached to request context when present,
  but nothing in this feature's design consumes it for authorization decisions —
  by design, not omission.
- **02-02's gRPC extension is wired only into `handle_get_document`** — extending the
  identical additive step-4 check to the other 8 RPC methods (`BatchGetDocuments`,
  `RunQuery`, `CreateDocument`, `UpdateDocument`, `DeleteDocument`, `BeginTransaction`,
  `Commit`, `Listen`) was explicitly out of this roadmap's scope (only `getDoc` had an
  acceptance scenario) and is follow-through work for a later pass — likely bundled
  with the Security Rules epic, since that is what will actually consume the identity
  on those other RPCs.
- OQ-CA-02 (verification-result caching) and OQ-CA-03 (embyr-hosted email/password) —
  both non-blocking, triggered by future profiling/customer-segment evidence
  respectively, not scheduled work.
- Outcome KPI measurement — DEVOPS-wave scope, owner platform-architect, per the
  Measurement Plan in `feature-delta.md` § Outcome KPIs.
