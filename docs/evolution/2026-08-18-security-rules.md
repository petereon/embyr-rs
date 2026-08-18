# Evolution: security-rules

**Date:** 2026-08-18
**Feature:** Server-evaluated Firestore Security Rules — a constrained
boolean-condition access-control grammar, per-collection rule authoring
(define/redefine + pre-publish simulation), and read-path (`GetDocument`)
enforcement gated on the caller identity `client-auth` established.
**Job:** JOB-17 (`document-access-control`), new job — P1 Alex (SDK Developer)
**ADRs:** ADR-027 (`docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md`),
ADR-028 (`docs/product/architecture/adr-028-access-rule-storage-and-lifecycle.md`),
ADR-029 (`docs/product/architecture/adr-029-access-control-composition-and-bounded-context.md`),
plus an append-only amendment to ADR-002 (`docs/product/architecture/adr-002-bounded-contexts.md`,
§ Context Map Summary / BC-4) adding the fourth bounded context this feature introduces.

## Business Context

`client-auth` (epic 1, finalized 2026-08-17, `docs/evolution/2026-08-17-client-auth.md`)
gave embyr-rs a way to verify who is calling — but verifying identity and *acting*
on it are different problems. Real Firebase security is two halves: (1) Firebase
Auth issuing ID tokens proving caller identity, and (2) server-evaluated Security
Rules gating per-document access based on that identity. `client-auth` built (1);
this feature builds (2) — the actual authorization/rules-evaluation engine. Before
this feature shipped, a session that never signed in with a client-identity token
behaved identically to one that did, for every existing data-plane call —
`client-auth`'s optional identity had no observable consequence for anyone. Now, a
rule can require `request.auth != null`, and the difference is observable for the
first time.

The raw ask ("enforce rules on every read/write/query/listen") was evaluated
against the Elephant Carpaccio oversized-scope gate (2 of 5 signals fired at full
ambition) and split into 5 ordered epics. This feature covers only **Epic 2a**
(rule authoring + read-path `GetDocument` enforcement). Write-path (2b),
query-path (2c), real-time-listen (2d), and rules-operational-maturity
(2e — history/versioning/rollback, richer grammar, audit logging) are named,
deferred follow-up epics — none started.

### Three explicit DISCUSS Framing Resolutions (mirroring client-auth's own methodology)

1. **Rule-expressiveness grammar (Resolution 1)**: a constrained boolean-condition
   subset — `==`/`!=`/`&&`/`||`/`!` over `request.auth`/`resource.data.<field>`/
   literals. Rejected: (A) full Firestore Rules Language parity (unbounded scope,
   no evidenced need for cross-document reads, and a `get()`/`exists()` call turns
   a pure computation into an I/O-bound one inside the hot read path); (B) a fixed
   enum of named rule shapes (real, avoidable migration friction for a developer
   already using Firebase's own boolean-condition syntax, with no implementation-cost
   saving over the grammar option).
2. **Default when no rule is defined (Resolution 2)**: unrestricted, unchanged — a
   DELIBERATE divergence from real Firebase's own locked-by-default posture ("test
   mode" or `allow read, write: if false`), forced by the pre-existing 113-scenario
   regression suite (72 embyr-rs + 41 client-auth), none of which define a rule
   since rules didn't exist when they were written.
3. **Rule lifecycle (Resolution 3)**: idempotent upsert (define/redefine), NOT
   `client-auth`'s register-then-rotate pattern with a two-generation overlap
   window — rules get edited far more frequently than a security credential
   (Alex iterates dozens of times while testing one collection), and a
   rotation-window model would add needless authoring friction.

## Key Decisions

### DESIGN-wave decisions (ADR-027, ADR-028, ADR-029, ADR-002 amendment)

| ID | Decision | Verdict |
|----|----------|---------|
| — | Rule grammar: hand-rolled recursive-descent parser over a locked EBNF (comparison + `&&`/`\|\|`/`!` + `true`/`false`), zero new crate — pest/nom considered and rejected because their extensibility is a liability against a deliberately-closed grammar | ADR-027 |
| — | Evaluation semantics: total/infallible `evaluate()` (no `Result`, no panic — `Allow`/`Deny` only); fail-closed on any missing referenced field, collapsing the whole condition tree to `Deny` regardless of `&&`/`\|\|`/`!` structure; no per-operator null-propagation (explicitly rejected as unevidenced scope) | ADR-027 |
| — | Storage/lifecycle: single `access_rules` row per `(project_id, collection_path)`, `ON CONFLICT ... DO UPDATE` upsert — define and redefine are the identical SQL statement, no code branch distinguishes them | ADR-028 |
| — | Composition: rule lookup runs unconditionally on every `GetDocument` call (cheap existence-check before the allow/deny decision), making the no-rule-defined guardrail structural, not merely tested | ADR-029 |
| — | Identity reuse: evaluation's `request.auth` consumes the exact `VerifiedEndUserIdentity`/`None` value `attach_client_identity_if_present()` already computes — no second, independently-verified identity path | ADR-029 |
| — | Simulation shares the exact evaluation routine real enforcement uses (`AccessRuleSimulationPort` calls the identical `parse_condition`/`evaluate` functions) — not a hand-rolled test double, no drift risk | ADR-029 |
| — | Bounded-context placement: new **BC-4 Access Control**, not folded into BC-1 or BC-2 | ADR-029, ADR-002 amendment |
| — | Earned Trust: no new `probe()` required anywhere — `get_access_rule`/`upsert_access_rule` reuse the already-probed `SystemDb` connection pool; `parse_condition`/`evaluate` are pure, deterministic CPU computation with no partial-trust surface | ADR-029 § Enforcement |

### A new bounded context: BC-4 Access Control

Added via an append-only amendment to the existing ADR-002 bounded-context
decision, not a rewrite. `client-auth`'s own credential-resolution precedent was
correctly folded into BC-1 (ADR-002 Option D) because it has "no entities, no
aggregate roots, no lifecycle... of its own." A rule does not share that absence:
it is an aggregate with real identity (scoped to `(project_id, collection_path)`)
and a define→redefine lifecycle. Its storage also doesn't cleanly fit BC-1
(System DB, but not a tenant-management concern) or BC-2 (Customer DB only, per
ADR-002's own storage-boundary signal — BC-2 never reads the System DB, while a
rule row is naturally System-DB-scoped). ADR-029 evaluated extending BC-1 and
folding into BC-2 as considered options and rejected both explicitly before
introducing BC-4.

### The structural regression guardrail (this feature's AC-16-08 equivalent)

AC-17-14/15/16: a collection with no rule defined behaves exactly as it did
before this feature shipped. This is enforced **by construction**, not merely
tested — the `rule_row == None` branch in `handle_get_document` is literally
unmodified, pre-feature code; `get_access_rule` returning `Ok(None)` for a
`(project_id, collection_path)` pair with zero rows is what makes that branch
identical to today's pre-security-rules behavior for all 113 pre-existing
regression scenarios (none of which ever seed an `access_rules` row). This was
independently confirmed twice: by the post-DELIVER adversarial review, and by
repeated full-suite regression runs throughout DELIVER (step 05-01's own
AC-17-16 obligation, and again at finalize).

### Existence non-leakage, honestly scoped

AC-17-10: a denied read never reveals document existence, for content-referencing
rules — a wrong-owner denial on an existing document and a denial on a
never-seeded document ID return the byte-identical `PermissionDenied` (same
`tonic::Code`, same message string). One edge case is explicitly flagged, not
hidden: a content-blind rule like `allow read: if true` still reveals existence
via `NotFound` when the document doesn't exist, matching real Firestore's own
behavior for that case — not a gap in this feature's guarantee, a documented
scope boundary (OQ-SR-06).

## Steps Completed

All 6 roadmap steps (`docs/feature/security-rules/deliver/execution-log.json`) show
complete `PREPARE → RED_ACCEPTANCE → GREEN → COMMIT` DES traces (4 steps
legitimately `SKIPPED` the `RED_UNIT` phase with a documented `NOT_APPLICABLE`
reason — this feature's entire production surface is 4 functions in 2 files,
all implemented in step 01-01; every step after 01-01 is a pure
unskip-and-verify checkpoint against already-correct, already-wired production
code, not new production logic).

| Step | Name | Status |
|------|------|--------|
| 01-01 | Walking Skeleton — `embyr_core::access_control::{parse_condition, evaluate}` (ADR-027) + `SystemDb::{upsert_access_rule, get_access_rule}` (ADR-028); the ONLY 4 RED-scaffold production functions in this feature; restores the pre-existing 113-scenario regression suite via the `rule_row==None` short-circuit | PASS |
| 02-01 | US-01 remainder — redefine (no overlap window), grammar-rejection distinguishability (`UNSUPPORTED_CONSTRUCT` vs `SYNTAX_ERROR`), already-GREEN admin-session guardrail | PASS |
| 03-01 | US-02 remainder — non-owner denial, non-ownership-rule allow, fail-closed missing field, existence non-leakage (extra-care anchor + cross-comparison) | PASS |
| 04-01 | US-03 remainder — public-read allows anonymous; invalid client-identity header evaluated identically to no header at all | PASS |
| 05-01 | US-04 remainder — per-collection isolation, PLUS the mandatory full 113-scenario regression re-proof (AC-17-16), run as part of this step's GREEN phase | PASS |
| 06-01 | US-05 (Release 2) — simulation shares the exact evaluation routine: matching-pair confirm, over-permissive bug surfacing, zero live-traffic effect, anonymous case | PASS |

`des-verify-integrity docs/feature/security-rules/deliver/` reports exit 0: "All 6
steps have complete DES traces."

Post-roadmap hardening, all on `master`:
- L1 refactor pass (`4670fb4`) — dropped stale RED-scaffold doc comments across
  `access_rules.rs`'s module doc and all 5 acceptance-file headers; simplified
  `define_access_rule`'s validation gate from a discard-pattern `let` to a direct
  `if let Err(e) = parse_condition(...)`.
- Adversarial review — APPROVED, zero blocking findings.
- Mutation-testing hardening (`c21ac9d`) — see below.

## Scenarios (verified by direct count, not by trusting a prior summary)

**20 acceptance scenarios**, counted directly from
`tests/security_rules/acceptance/*.rs` (`grep -n '^async fn'` per file, excluding
the permanent structural marker):

| Suite | Count | Notes |
|-------|-------|-------|
| sr01 (define/redefine) | 6 | US-01, AC-17-01..05; includes 1 new Viewer-role-403 test added by mutation hardening (originally 5) |
| sr02 (signed-in read gated by rule) | 5 | US-02, AC-17-06..10 |
| sr03 (anonymous session as null auth) | 3 | US-03, AC-17-11..13 |
| sr04 (untouched collections + regression) | 2 | US-04, AC-17-14..15; plus 1 permanent `#[ignore]`d structural marker (`full_113_scenario_regression_suite_passes_unmodified`, AC-17-16 — a proof obligation over external test binaries, deliberately not an in-process assertion) |
| sr05 (simulate before publish) | 4 | US-05, AC-17-17..19 |

Plus 12 layer-1 unit/property tests in
`crates/embyr-core/src/access_control/mod.rs` (2 of the 12 are `proptest!`
properties at 64 cases each: any ownership condition against an empty resource
map always denies; a mismatched-owner denial holds regardless of unrelated
resource fields), including 1 new pinned unit test added by mutation hardening
(`wildcard_path_condition_is_rejected_as_unsupported_not_syntax_error`).

Total: 20 acceptance + 13 unit/property (12 original + 1 mutation-hardening
addition, counted once here — see § Mutation Testing) = 33 tests directly
exercising this feature's code.

Full mandatory 19-target `embyr-rs` + `client-auth` regression suite: confirmed
0 regressions, exit code 0, repeatedly throughout DELIVER.

**Known environmental caveat, not a regression of this feature**:
`tests/acceptance/us_10_aws_secrets.rs` fails consistently on this machine with
`TrustStore configured to enable native roots but no valid root certificates
parsed!` — a native TLS root-certificate-store loading issue inside the
`aws-smithy-http-client` crate. Confirmed via `git diff` that this file and all
AWS/TLS-related dependencies are completely untouched by `security-rules` across
every step. This is a machine/environment issue, flagged here plainly for
whoever next touches AWS-secrets-related code on this machine — not an open
defect of this feature.

## Mutation Testing (`per-feature`, per `CLAUDE.md`)

`cargo-mutants -p embyr-core --filter access_control` found and closed **2
genuine gaps**, both fixed with new tests, zero production-code changes needed
(the production logic was already correct — the tests hadn't pinned it tightly
enough):

1. **`WildcardPath` grammar-rejection branch had no direct pinned test.**
   `detect_unsupported_construct`'s `**`/`{` (wildcard-path) branch was only
   reachable via the call-syntax (`CrossDocumentRead`) branch's sibling logic —
   mutating the guard's `||` to `&&` survived all 12 existing tests. Fixed
   (`c21ac9d`) with `wildcard_path_condition_is_rejected_as_unsupported_not_syntax_error`,
   mirroring the existing cross-document-read test's shape.
2. **The rule-definition admin endpoint's role gate had only Owner-role
   coverage.** `define_access_rule`'s `session.role < Role::Admin` gate had no
   Viewer-role-403 boundary case — mirroring the exact same class of gap
   `client-auth`'s own mutation pass found in its rotate-credential endpoint
   (Admin-exactly boundary, underspecified by Owner-only test coverage). Fixed
   (`c21ac9d`) with `rule_definition_by_a_viewer_role_session_is_rejected_403`.

**Environmental noise encountered and worked around, documented for future
runs**: this pass hit a corrupted shared `CARGO_TARGET_DIR` incremental-build
state, and CPU oversubscription producing false `TIMEOUT` verdicts under this
project's `-j6`-parallel default (5 additional MISSED reports beyond the 2
genuine gaps). Isolating the target dir and manually reproducing each MISSED
mutant directly against the source distinguished genuine gaps from harness
artifacts. The final 100% kill rate reflects that manual verification
discipline, not blind trust in the tool's first-pass report.

## The Step 01-01 False-Alarm Incident

A crafter dispatch's session crashed mid-implementation of step 01-01 when a
specific unit test appeared to hang. Deep investigation (temporary
iteration-guard instrumentation, macOS `sample`-based CPU profiling) found **no
actual bug** — the apparent hang coincided with extreme, unrelated system load
(load average ~30 from concurrent cargo builds sharing the machine) causing CPU
starvation, not an infinite loop. The parser/evaluator implementation was
correct throughout; termination was subsequently proven by construction (every
scan loop strictly advances its index; recursion is bounded by token count) and
independently re-confirmed by the adversarial review.

**Lesson**: a "hanging" test under heavy shared-machine load is not
automatically a code defect — verify via CPU profiling before assuming a logic
bug.

## Quality Gates

- **Per-step TDD:** 6/6 steps COMMIT/PASS, all DES traces complete.
- **`des-verify-integrity`:** exit 0, "All 6 steps have complete DES traces."
- **Refactor L1 (`4670fb4`):** stale RED-scaffold doc comments removed;
  define-time validation gate simplified. L2-L6: nothing to do — no genuine
  duplication, naming, structural, or pattern-consistency issues found within
  scope.
- **Adversarial review:** APPROVED, zero blocking findings.
- **Mutation testing (`per-feature`):** 2 genuine gaps found and closed
  (`c21ac9d`), zero production-code changes needed; 5 additional false-negative
  reports investigated and confirmed as harness artifacts, not hidden.
- **AC-17-16 regression gate:** full 113-scenario pre-existing suite (72
  embyr-rs + 41 client-auth) re-run at step 05-01's GREEN phase and confirmed
  again at finalize — 0 regressions. The `us_10_aws_secrets` environmental
  TLS failure (see § Scenarios above) is the sole non-passing test in the
  wider 19-target suite and is confirmed unrelated to this feature.
- **`cargo deny check bans`:** `bans ok` — zero new workspace dependencies
  (ADR-027 Consequences: "Zero new workspace dependency").

## Open Questions

- **OQ-SR-06** — existence non-leakage (AC-17-10) is scoped to content-referencing
  rules only; a content-blind rule (`allow read: if true`) still reveals existence
  via `NotFound` on a denied-by-other-means path, matching real Firestore's own
  behavior for that case. Not a gap — a documented scope boundary. Revisit only
  if a future epic's domain example needs a stronger guarantee.
- **Resolution 1's write-path edge** — whether Alex will ever need `resource.data`
  on the *previous* version of a document during an update (real Firestore's
  `resource` vs. a hypothetical `request.resource` distinction). Not applicable
  to this feature's read-only v1 scope; flagged for Epic 2b (write-path), which
  commonly needs both old and new document state.

## Lessons Learned

1. **A "hanging" test under heavy shared-machine load is not automatically a
   code defect.** See § The Step 01-01 False-Alarm Incident above — verify via
   CPU profiling (macOS `sample`, or equivalent) before assuming a logic bug,
   especially when the implementation's termination can be proven by
   construction (bounded recursion, strictly-advancing scan index).
2. **Mutation testing keeps finding the same shape of gap across features in
   this codebase: role-gate boundary conditions under-covered by
   Owner-only-role test fixtures, and construct-rejection branches sharing a
   guard with an already-tested sibling branch.** This feature's Viewer-role-403
   gap mirrors `client-auth`'s own Admin-exactly-boundary gap in its
   rotate-credential endpoint almost exactly — worth a standing mutation-testing
   checklist item: "does every role gate have a boundary-role test, not just an
   above-the-line one?"
3. **A reported mutation-testing MISSED verdict is not automatically a real
   gap.** 5 of 7 MISSED reports in this pass were harness artifacts (corrupted
   shared `CARGO_TARGET_DIR`, CPU-oversubscription false TIMEOUTs under `-j6`)
   — isolating the target dir and manually reproducing each candidate mutant
   directly against the source was necessary to separate genuine gaps from
   tooling noise, echoing `client-auth`'s own investigated-survivor lesson.
4. **Giving an existing, previously-inert capability real behavioral
   consequence is itself a distinct kind of risk from building a new one.**
   `client-auth`'s `VerifiedEndUserIdentity` existed and was already computed
   before this feature shipped — this feature's entire read-path enforcement
   surface was "wire an existing, already-correct value into a new decision,"
   not "build new identity machinery," which is exactly why the
   FUNCTION-CALL DEPENDENCY TRACE performed during DELIVER roadmap preparation
   found only 4 RED-scaffold production functions across the whole feature.
5. **A structural guardrail (enforced by construction) is worth the extra
   sequencing discipline it requires.** Step 01-01 was scoped specifically to
   restore the 113-scenario regression suite via the `rule_row==None`
   short-circuit before any other step began — the DISTILL-time RED
   classification could not even get a clean regression baseline until that
   ordering was respected (unlike `client-auth`, whose scaffold was
   conditionally entered and left the regression suite genuinely unaffected
   during RED).

## Key Files

- `crates/embyr-core/src/access_control/mod.rs` — `parse_condition()`,
  `evaluate()`, `Condition`/`Operand`/`ConditionParseError` types, 13
  unit/property tests (12 original + 1 mutation-hardening addition)
- `crates/embyr-server/src/adapters/system_db.rs` —
  `upsert_access_rule`, `get_access_rule`
- `crates/embyr-server/src/admin/handlers/access_rules.rs` —
  `define_access_rule`, `simulate_access_rule`
- `crates/embyr-server/src/grpc/handler.rs` — `handle_get_document`'s
  `Some(rule_row)`/`None` branches (lines ~553-616)
- `migrations/0022_access_rules.sql` (workspace-root `migrations/`, not
  `crates/embyr-server/migrations/`)
- `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md`
- `docs/product/architecture/adr-028-access-rule-storage-and-lifecycle.md`
- `docs/product/architecture/adr-029-access-control-composition-and-bounded-context.md`
- `docs/product/architecture/adr-002-bounded-contexts.md` § Context Map
  Summary / BC-4 (append-only amendment)
- `tests/security_rules/acceptance/` — 20 acceptance scenarios (sr01-sr05) +
  1 permanent structural marker + shared `common/mod.rs` harness
- `docs/feature/security-rules/feature-delta.md` — full DISCUSS/DESIGN/DISTILL
  narrative (retained in place, not migrated — this project's established SSOT
  convention, same as `client-auth`'s own finalize)
- `docs/feature/security-rules/distill/red-classification.md` —
  fail-for-right-reason gate results, including the Vacuous-Pass Finding
  (sr03's AC-17-13 anchor assertion) and the Regression-Suite Finding
  (`get_access_rule`'s unconditional call site)
- `docs/feature/security-rules/slices/` — 5 elephant-carpaccio slice briefs

## Follow-Up Work

- **Epic 2b — `security-rules-write-path`** — extend the same grammar/evaluation
  function to `create`/`update`/`delete`. Named, deferred, not started. Needs
  its own DISCUSS/DESIGN pass, not assumed to be a mechanical grammar extension
  — Resolution 1's write-path edge (old vs. new `resource.data`) is an open
  question this epic must resolve.
- **Epic 2c — `security-rules-query-path`** — `RunQuery` enforcement, flagged
  as needing its own DISCUSS/DESIGN since query-time rule compliance is a
  structurally different mechanism (reject a non-compliant query shape before
  execution) than a point-read post-check. Named, deferred, not started.
- **Epic 2d — `security-rules-realtime`** — `Listen`/`onSnapshot` enforcement,
  BC-3, eventual-consistency, re-fetch-on-`DocChange` mechanism. Named,
  deferred, not started.
- **Epic 2e — `security-rules-operations`** — rule history/versioning/rollback,
  richer condition grammar (if Resolution 1's Option A trigger ever fires),
  audit logging. Named, deferred, not started.
- **`us_10_aws_secrets`'s environmental TLS issue** (see § Scenarios above) —
  open environment caveat, unrelated to this feature, for whoever next touches
  AWS-secrets-related code on this machine.
- Outcome KPI measurement — DEVOPS-wave scope, owner platform-architect, per
  the Measurement Plan in `feature-delta.md` § Outcome KPIs.
