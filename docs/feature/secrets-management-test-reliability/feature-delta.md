# Feature Delta: secrets-management-test-reliability

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/known-gaps.md` #7 — exact current wording confirmed: "`secrets_management`
sm01/sm02 tests fail consistently — server+LocalStack container doesn't exit within the test's
own 10s wait... Test-only — Docker/AWS-SDK timing, not a runtime code path a real client
hits... Test flakiness/reliability, not a production data/crash risk... Not started — found as
a byproduct of firestore-transaction-read-consistency's own regression testing,
bisection-confirmed pre-existing (reproduces without that feature's diff)."
✓ `tests/secrets_management/acceptance/sm01_admin_key_secrets_manager.rs` — read in full.
Exact failing test: `exits_1_when_admin_key_secret_fetch_fails` (line 207), tagged
`@error @real-io @US-SM-01 @AC-SM-01-05`. Spawns a real `embyr-server` subprocess with
`EMBYR_ADMIN_KEY_AWS_SECRET_ARN` pointed at `nonexistent_secret_arn()` against a real
LocalStack container (`start_localstack()`), then calls
`server.wait_for_exit(Duration::from_secs(10))` and asserts `exit_code == Some(1)` plus a
specific stderr substring (`"could not fetch EMBYR_ADMIN_KEY"` or `"SecretFetchFailed"`) plus
`!ServerProcess::port_is_bound(server.admin_port)`.
✓ `tests/secrets_management/acceptance/sm02_encryption_key_secrets_manager.rs` — read in full.
Exact GCP-equivalent-shaped sibling test (also AWS-path, "GCP-equivalent" per the task brief
refers to it being sm02's own analogue of sm01's fetch-failure case, not a literal GCP test —
sm02 has no separate GCP fetch-failure scenario; its `_gcp_` naming is file-level only, all its
own scenarios including this one exercise the AWS path): `exits_1_when_encryption_key_secret_fetch_fails`
(line 295), tagged `@error @real-io @US-SM-02 @AC-SM-02-05`. Identical shape to sm01's test:
same `start_localstack()`, same `nonexistent_secret_arn()`, same
`server.wait_for_exit(Duration::from_secs(10))`, same `exit_code == Some(1)` +
stderr-substring + port-not-bound assertion pattern, targeting
`EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN` instead of the admin-key var.
✓ `docs/feature/firestore-tls-support/feature-delta.md`,
`docs/feature/firestore-is-null-filter-support/feature-delta.md` — read in full to confirm
this project's own established single-file, Tier-1-only `feature-delta.md` convention
(`## Wave: DISCUSS / [REF] {Section}` heading format, no standalone
`acceptance-criteria.md`/`outcome-kpis.md`/`story-map.md` files). This DISCUSS mirrors that
convention, at reduced depth appropriate to a lightweight infrastructure-only feature (no
journey visualization, no persona/JTBD phase — see § Orchestrator Decisions).
✓ Explicitly did NOT investigate `startup_localstack()`'s own implementation, the AWS SDK's own
retry/backoff configuration, or any other root-cause hypothesis beyond what the gap itself
already states — that is DESIGN's own job in the next wave, per this DISCUSS's explicit scope
limit.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Infrastructure**.
- **Decision 4 = infrastructure-only escape valve** (explicitly invoked, not the default JTBD
  path): this feature has ZERO user-visible production behavior change. No real Firestore
  client, SDK, or operator interaction changes at all — `ServerConfig::from_env()`'s own
  fetch-failure exit-1 behavior (the thing these 2 tests verify) is not being modified; only the
  TEST'S OWN ability to reliably observe that already-correct behavior within a bounded wait is
  in scope. JTBD analysis, persona work, and journey design are skipped entirely for this
  feature per the task brief's explicit instruction — there is no user job a flaky test's own
  fix serves; the beneficiary is this session's own engineering velocity (see § Business
  Context).
- Walking Skeleton: **N/A** — this feature IS its own walking skeleton (single story, both
  tests share one root pattern; see § Walking Skeleton Strategy).
- UX Research Depth: **None** — no journey, no persona, no emotional arc. Skipped per task
  brief instruction (Phase 1 JTBD skipped; going straight to lightweight Phase 2/3 requirements
  crafting).

## Wave: DISCUSS / [REF] Business Context

Two acceptance tests in the `secrets_management` binary —
`sm01_admin_key_secrets_manager.rs::exits_1_when_admin_key_secret_fetch_fails` and
`sm02_encryption_key_secrets_manager.rs::exits_1_when_encryption_key_secret_fetch_fails` — fail
consistently enough that they are excluded from "clean regression run" expectations every time
this session runs a full-workspace `cargo test`. Both spawn a real `embyr-server` subprocess
configured with a nonexistent AWS Secrets Manager ARN against a real LocalStack container,
expecting the server to exit 1 within a 10-second test-level `wait_for_exit` timeout. The gap's
own existing hypothesis: the server+LocalStack container doesn't exit within that 10s wait —
not investigated further here (DESIGN's own job).

**Evidenced recurring cost**: known-gaps.md #7 documents this was first found and
bisection-confirmed pre-existing as a byproduct of `firestore-transaction-read-consistency`'s
own regression testing (2026-09-06) — a feature whose own diff never touched
`secrets_management` at all, proving the failure is independent of feature work, not a symptom
of any one change. Per this project's own established convention (CLAUDE.md: full-workspace
regression reserved for pre-commit/FINALIZE gates, not every intermediate check), every feature
that has run a full-workspace suite at its own FINALIZE gate since 2026-09-06 — at minimum
`firestore-end-cursor-support`, `firestore-or-filter-support`, `firestore-is-null-filter-support`,
and `firestore-tls-support` (4 FINALIZE gates spanning 2026-09-07 through 2026-09-08) — has
had to re-apply the same "triage, confirm pre-existing, dismiss" judgment call these 2 tests
force every single time, a real if narrow engineering-velocity cost paid repeatedly rather than
fixed once. This session's own `feedback_triage_before_dismissing_as_flaky` memory entry exists
precisely because that triage step is not free or risk-free — treating a failure as "known
flaky" without re-verification is exactly the mistake that memory entry warns against, and this
gap is the recurring source of the temptation to skip that verification.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — entirely
confined to the `secrets_management` test binary's own harness code (both tests share one root
pattern: `start_localstack()` + `nonexistent_secret_arn()` + `wait_for_exit(10s)`). Walking
skeleton >5 integration points? No (1 real LocalStack container + 1 real subprocess, exercised
identically by both tests). Estimated effort >2 weeks? No — a timing/readiness-wait mechanism
touching test-harness code only, no new domain concept, no new adapter trait, no new bounded
context, no production code path. Multiple independent user outcomes? No — "these 2 tests pass
reliably" is a single outcome; sm01 and sm02 are 2 verification points of that ONE outcome
(identical failure shape), not 2 separable stories.

**Scope Assessment: PASS** — 1 user story, 1 bounded context (`tests/secrets_management`
acceptance-test harness), estimated ≤1 day, 4 UAT scenarios (right-sized, below the 3-7 range's
midpoint given the narrow, mechanical nature of the fix).

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

This feature IS the walking skeleton — both failing tests share one root pattern
(`start_localstack()` → configure nonexistent ARN → spawn real `embyr-server` subprocess →
`wait_for_exit(Duration::from_secs(10))` → assert exit code + stderr + port-not-bound), so
there is no meaningful sub-slicing: whatever DESIGN's own timing/readiness mechanism turns out
to be, it is proven end-to-end against BOTH tests together, not sequentially against one first.
Proof consists of: (1) the regression path — every other currently-passing test in
`secrets_management` and the wider workspace stays green, (2) the reliability target — both
named tests pass N/N consecutive runs (see AC-STR-01), (3) the correctness-preservation
guard — the mechanism must not silently mask a genuine detection failure (see AC-STR-03).

## Wave: DISCUSS / [REF] User Stories

### US-01: Secrets-Manager Fetch-Failure Tests Pass Reliably, Every Run

**job_id**: infrastructure-only

**infrastructure_rationale**: This feature touches only test harness/timing code in
`tests/secrets_management/acceptance/` — it does not change `ServerConfig::from_env()`'s own
fetch-failure exit-1 behavior, any gRPC/REST/Admin route, or any code path a real Firestore
client, SDK, or operator ever exercises. The 2 tests already correctly verify production
behavior that already works (per the gap's own framing: "not a runtime code path a real client
hits... Test flakiness/reliability, not a production data/crash risk"); this feature only makes
the TEST'S OWN observation of that already-correct behavior reliable within a bounded wait. No
user job (JOB-01 through JOB-13 in `docs/product/jobs.yaml`) covers "this session's own
regression-suite reliability" — the beneficiary is engineering velocity for whoever runs this
workspace's test suite, not an end user or operator persona. This satisfies Decision 4's escape
valve exactly as scoped: a real, narrow, evidenced exception, not a default-path shortcut.

**Note for peer review (Dimension 0, Elevator Pitch Test)**: this story is correctly
`@infrastructure` and intentionally carries no Elevator Pitch — per the task brief's explicit
instruction, Elevator Pitch is mandatory only for non-`@infrastructure` stories. Dimension 0's
own "slice-level" concern (an all-infrastructure slice has no release value) does not apply
here: this is a single-story feature reusing the documented Decision 4 escape valve, not a
slice within a larger release that should have contained a user-visible story.

#### Who

Not a user/persona — internal stakeholder is this session's own regression-testing workflow
(the engineer or agent running `cargo test` against this workspace, who currently must
manually triage 2 known-failing tests every full-suite run instead of trusting a clean pass/fail
signal).

#### Problem

Every time this session runs a full-workspace `cargo test`, `sm01`'s
`exits_1_when_admin_key_secret_fetch_fails` and `sm02`'s
`exits_1_when_encryption_key_secret_fetch_fails` fail non-deterministically-but-consistently-
enough that they must be manually triaged and dismissed as "known pre-existing, not caused by
this diff" before trusting the rest of the run — a real, evidenced, recurring cost (see §
Business Context) rather than a one-time annoyance.

#### Solution

DESIGN decides the exact mechanism. After this feature, both tests either (a) pass reliably in
this dev environment as-is, or (b) if the root cause is a genuinely environment-dependent timing
issue that can't be eliminated outright, gain an increased timeout, retry, or readiness-wait
mechanism that makes them reliably pass within a reasonable bound — without becoming a "wait
longer and hope" test that would no longer catch a genuine regression in the production
fetch-failure-exit-1 behavior. Candidate hypothesis (from the gap's own wording, not
investigated further here): LocalStack container readiness racing the server's own AWS SDK
retry/backoff behavior against a nonexistent-ARN response. DESIGN may find a different or
additional cause; this DISCUSS does not lock a mechanism.

#### Domain Examples

**1 (Happy Path — target state)**: An engineer runs
`cargo test --test secrets_management -- exits_1_when_admin_key_secret_fetch_fails` and
`cargo test --test secrets_management -- exits_1_when_encryption_key_secret_fetch_fails` 5
times each, consecutively, on this dev environment. All 10 runs (5 + 5) pass with no manual
retry and no manual triage step — the same experience every other passing test in the suite
already provides.

**2 (Edge Case — recurring cost this feature closes)**: During `firestore-transaction-read-
consistency`'s own regression run (2026-09-06), these 2 tests failed despite that feature's
diff never touching `secrets_management`; the session had to bisection-confirm pre-existing
status before trusting the rest of that run's signal. This feature closes that exact repeated
triage cost for every subsequent feature's own FINALIZE gate, not just retroactively explains
one incident.

**3 (Error/Boundary — the correctness trap DESIGN must avoid)**: If DESIGN's fix simply raised
`Duration::from_secs(10)` to, say, `Duration::from_secs(60)` with no other change, AND a future
unrelated change broke `ServerConfig::from_env()`'s own fetch-failure detection so the server
incorrectly exited 0 or incorrectly bound its admin port on a genuine fetch failure, the test
must still fail — `exit_code == Some(1)` and `!port_is_bound(...)` are asserted regardless of
how long the wait was extended. A "just wait longer" fix that also weakened or removed either
assertion would pass the reliability bar (AC-STR-01) while silently breaking the meaningful
correctness check (AC-STR-03) — this example is why AC-STR-03 exists as a distinct,
independently-verified acceptance criterion.

#### UAT Scenarios (BDD)

```gherkin
Scenario: The admin-key fetch-failure test passes reliably across consecutive runs
  Given a nonexistent AWS Secrets Manager ARN is configured for EMBYR_ADMIN_KEY_AWS_SECRET_ARN
  And a real LocalStack container is running
  When exits_1_when_admin_key_secret_fetch_fails is run 5 consecutive times
  Then all 5 runs pass with no manual retry
  And each run's server process exits with code 1
  And each run's stderr names the fetch failure
  And each run's admin port is never bound

Scenario: The encryption-key fetch-failure test passes reliably across consecutive runs
  Given a nonexistent AWS Secrets Manager ARN is configured for EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN
  And a real LocalStack container is running
  When exits_1_when_encryption_key_secret_fetch_fails is run 5 consecutive times
  Then all 5 runs pass with no manual retry
  And each run's server process exits with code 1
  And each run's stderr names the fetch failure
  And each run's admin port is never bound

Scenario: No other currently-passing test regresses
  Given the timing/readiness mechanism chosen by DESIGN is applied
  When the full secrets_management test binary is run
  And the full workspace regression suite is run
  Then every test that passed before this feature still passes
  And no new flakiness is introduced in any other test

Scenario: The fix does not mask a genuine fetch-failure-detection regression
  Given a hypothetical build where the server incorrectly exits 0 or binds its admin port on a genuine secret-fetch failure
  When exits_1_when_admin_key_secret_fetch_fails or exits_1_when_encryption_key_secret_fetch_fails is run against that hypothetical build
  Then the test still fails
  And the failure is attributable to the exit-code or port-bound assertion, not a timeout artifact
```

#### Acceptance Criteria

- [ ] **AC-STR-01** (reliability target): both `exits_1_when_admin_key_secret_fetch_fails`
      (sm01) and `exits_1_when_encryption_key_secret_fetch_fails` (sm02) pass 5/5 consecutive
      runs each, with no manual retry, on this dev environment.
- [ ] **AC-STR-02** (no regression): every other currently-passing test in the
      `secrets_management` binary, and in the wider workspace regression suite, still passes
      after the fix — no new flakiness introduced anywhere else.
- [ ] **AC-STR-03** (correctness preserved, not just "wait longer and hope"): the chosen
      mechanism must not remove, weaken, or bypass either test's own `exit_code == Some(1)`,
      stderr-substring, or `!port_is_bound(...)` assertions. Both tests must remain able to
      correctly FAIL if the server incorrectly exits 0, or incorrectly binds its port, on a
      genuine secret-fetch failure — verified by DESIGN/DELIVER reasoning about the mechanism's
      own construction (e.g. a mechanism that only changes HOW LONG or HOW OFTEN the test waits
      for the existing exit-code/stderr/port checks, never what those checks assert).
- [ ] **AC-STR-04** (bounded, not open-ended): whatever timeout/retry/readiness-wait bound
      DESIGN chooses is a concrete, finite value with a stated rationale (not an unbounded retry
      loop) — the fix must not trade "consistently fails" for "occasionally hangs for an
      unreasonable duration."

#### Technical Notes

- Candidate hypothesis from the gap's own wording (not investigated further here, DESIGN's own
  job): LocalStack container readiness racing the server's own AWS SDK retry/backoff behavior
  against a nonexistent-ARN response.
- Both tests share one root pattern (`start_localstack()` + `nonexistent_secret_arn()` +
  `wait_for_exit(Duration::from_secs(10))`) in `tests/secrets_management/acceptance/common/mod.rs`
  and the 2 acceptance test files — a shared-helper-level fix is plausible but not prescribed;
  DESIGN decides whether the fix lives in the shared test-harness helper, in each test
  individually, or in the LocalStack startup helper itself.
- No production code (`crates/embyr-core`, `crates/embyr-server`, `crates/embyr-pg-storage`) is
  expected to change for this feature — if DESIGN's own investigation finds a production-code
  cause, that would upgrade this feature's severity and scope beyond what this DISCUSS assumed;
  flagged as a risk, not resolved here.
- Depends on nothing new — the test harness, LocalStack container helper, and `ServerProcess`
  subprocess wrapper already exist and are unchanged in purpose by this feature.

#### Outcome KPIs

- **Who**: this session's own engineering workflow (whoever runs `cargo test` against this
  workspace, human or agent).
- **Does what**: runs a full-workspace regression suite and trusts a clean pass/fail signal
  from the `secrets_management` binary without manual triage of 2 known-failing tests.
- **By how much**: from 0/5 reliable (tests fail consistently enough to require triage every
  run, per known-gaps.md #7) to 5/5 consecutive passes for both tests, with 0 regressions
  elsewhere.
- **Measured by**: 5 consecutive local runs of each named test (AC-STR-01); one full-workspace
  regression run pre- and post-fix diffed for new failures (AC-STR-02).
- **Baseline**: 0/5 — both tests are documented as failing consistently today (known-gaps.md
  #7), first bisection-confirmed pre-existing 2026-09-06.

## Wave: DISCUSS / [REF] Definition of Done

1. Both named tests (`exits_1_when_admin_key_secret_fetch_fails`,
   `exits_1_when_encryption_key_secret_fetch_fails`) pass 5/5 consecutive runs (AC-STR-01).
2. No regression in `secrets_management` or the wider workspace regression suite (AC-STR-02).
3. The mechanism is proven to preserve correctness — it does not silently mask a genuine
   fetch-failure-detection bug (AC-STR-03), and uses a bounded, rationale-backed wait/retry
   value, not an open-ended one (AC-STR-04).
4. Full regression suite clean (any OTHER pre-existing flake, e.g. known-gaps.md #8's CEL
   rule-import gap, is unrelated and out of scope — triaged not assumed, per this session's own
   established `feedback_triage_before_dismissing_as_flaky` practice).
5. Mutation testing: per this project's `per-feature` strategy — applied to whatever new/changed
   test-harness logic DESIGN introduces, if any is non-trivial enough to warrant it (a pure
   timing-constant change may not produce meaningful mutants; DELIVER/QUALITY_GATE decide).
6. Evolution doc written; `docs/product/known-gaps.md` #7 updated to CLOSED.
7. Memory updated.

## Wave: DISCUSS / [REF] Out of Scope

- **Deep root-cause investigation** of WHY the test times out (LocalStack startup timing vs.
  AWS SDK retry/backoff behavior vs. something else) — DESIGN's own job, this DISCUSS
  deliberately did not investigate `startup_localstack()`'s own implementation or the AWS SDK's
  own retry configuration.
- **known-gaps.md #8** (CEL rule-import chaining-`get()` detection gap) — a separate, unrelated
  pre-existing issue found alongside #7 in the same regression run; not addressed by this
  feature.
- **Any production code change** — unless DESIGN's own investigation surfaces a genuine
  production-code cause (flagged as a risk in § Technical Notes, not assumed or resolved here).
- **sm02's own GCP-path scenario** (`server_starts_with_admin_key_from_gcp_secret_manager` in
  sm01, already `#[ignore]`d pending OQ-SM-4/ADR-018 Alternatives A6) — unrelated pre-existing
  gap in GCP local-emulator test coverage, not one of the 2 tests this feature targets.

## Wave: DISCUSS / [REF] Driving Ports

None — internal test infrastructure only, no user-facing driving port for this feature itself.
The 2 tests in scope drive `embyr-server`'s existing production driving port (subprocess via
ENV/STDERR/exit-code, `ServerConfig::from_env()`) unchanged by this feature — only the test's
own wait/observation mechanism around that existing port is in scope.

## Wave: DISCUSS / [REF] Pre-requisites

None beyond what already exists — the `ServerProcess` subprocess wrapper, `start_localstack()`
container helper, and `nonexistent_secret_arn()` fixture already exist in
`tests/secrets_management/acceptance/common/mod.rs` and are reused, not replaced, by this
feature (their own implementation is not inspected here per this DISCUSS's explicit scope
limit).

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.90**

### DoR Checklist (9-item hard gate)

1. [x] Story traces to a job_id — `infrastructure-only` with a real, specific
   `infrastructure_rationale` (not the literal string alone).
2. [x] Elevator Pitch — N/A, correctly exempted (`@infrastructure` story per task brief and
   Decision 4).
3. [x] Every AC is testable without ambiguity (4 ACs: 5/5 consecutive-run count, zero-regression
   diff, assertion-preservation reasoning, bounded-wait rationale).
4. [x] Walking Skeleton identified (US-01 is the whole feature's walking skeleton).
5. [x] Scope Assessment passed.
6. [x] 3+ domain examples with real data (session dates, real test/file names, a concrete
   hypothetical-regression scenario for the correctness trap).
7. [x] Out of Scope explicitly named (4 items, each reasoned).
8. [x] Outcome KPIs have numeric targets (0/5 → 5/5) and measurement methods.
9. [x] Prior-wave artifacts read and reconciled (known-gaps.md #7's exact wording, both test
   files read in full, this project's own feature-delta.md convention confirmed).

### DoR Status: **PASSED**

### Note on Dimension 0 (Elevator Pitch Test) for peer review

This story is intentionally `@infrastructure` with no Elevator Pitch, per the task brief's
explicit instruction that Elevator Pitch is mandatory only for non-`@infrastructure` stories.
The `infrastructure_rationale` field substitutes for it here, providing the same "why does this
deserve to exist" grounding an Elevator Pitch would for a user-facing story. Reviewer should
verify the rationale is specific and evidenced (it cites known-gaps.md #7, the exact failing
test names, and the recurring-triage-cost argument from § Business Context), not a generic
placeholder.

## Wave: DISCUSS / [REF] Open Questions

None carried forward unresolved. The task brief's own framing (fix reliably OR add a
timing/readiness mechanism, mechanism TBD) is deliberately left open for DESIGN — not an
unresolved DISCUSS-level question, but an explicit handoff of the "how" decision.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions

- [D1] Feature type: Infrastructure, using Decision 4's infrastructure-only escape valve —
  explicitly justified (zero production behavior change, test-harness-only scope).
- [D2] Single story (US-01), no sub-slicing — both tests share one root pattern, proven
  together, not sequentially.
- [D3] Reliability bar: 5/5 consecutive runs per test, no manual retry (AC-STR-01).
- [D4] Correctness-preservation is a first-class, independently-verified AC (AC-STR-03) — the
  fix must not become a "wait longer and hope" change that could mask a genuine detection
  regression.
- [D5] Root-cause investigation explicitly deferred to DESIGN — this DISCUSS names only the
  gap's own existing candidate hypothesis, does not investigate further.

### Requirements Summary

- Primary need: 2 specific, named tests must stop consuming manual triage time on every
  full-workspace regression run, without weakening what they actually verify.
- Walking skeleton scope: US-01, the entire feature — single story, 4 UAT scenarios.
- Feature type: Infrastructure (Decision 4 escape valve).

### Constraints Established

- No production code change assumed or required (flagged as an open risk if DESIGN's
  investigation finds otherwise).
- The fix must be a bounded, rationale-backed mechanism — not an open-ended retry loop.
- Both tests' existing exit-code/stderr/port-not-bound assertions must remain intact.

### Upstream Changes

None — this is a new, standalone feature-delta.md; no other feature's DISCUSS artifacts are
amended by this work.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 5 locked Decisions (D1-D5), 1-story walking-skeleton
plan, 4 ACs (AC-STR-01 through 04) to design a concrete timing/readiness/retry mechanism
against, with the explicit correctness-preservation constraint (AC-STR-03) as a hard design
requirement, not a nice-to-have.

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ This feature-delta.md's DISCUSS section — read in full (D1-D5, 4 ACs, the explicit
root-cause-investigation deferral to this wave).
✓ `docs/product/architecture/brief.md` — checked; no entry for this feature or the
`secrets_management` test suite exists (infrastructure-only, test-harness-scoped feature; no
`## Application Architecture` section applies). Proceeding as the sole architect for this
feature, consistent with Decision 4's escape valve.
✓ `tests/secrets_management/common/mod.rs` — read in full: `start_localstack()` (lines
144-159), `make_sm_client()` (161-175), `create_raw_secret()` (176-195), `nonexistent_secret_arn()`
(197-200), `ServerProcess::start_env_only`/`spawn_with` (453-519), `wait_for_exit` (588-602).
✓ `crates/embyr-server/src/adapters/aws_secret_fetcher.rs` — read in full: no explicit
`RetryConfig`/`timeout_config` on the AWS SDK client.
✓ `crates/embyr-server/src/config.rs` lines 380-530 — read: `from_env()` call order (line
245-252, admin key resolved first, before any DB pool creation — confirms the hardcoded
unreachable `DATABASE_URL` in the two failing tests is never contacted before the secret-fetch
failure path returns, ruling out a DB-connect-timeout confound), `fetch_from_secret_manager`
(454-490, AWS branch at 476-488: `aws_config::load_defaults(BehaviorVersion::latest())`, no
explicit retry/timeout override).
✓ `tests/secrets_management/acceptance/sm01_admin_key_secrets_manager.rs` and
`sm02_encryption_key_secrets_manager.rs` — re-read with attention to every scenario that touches
`start_localstack()`, not just the 2 named failing tests, to compare warm-up patterns and
timeout values across all LocalStack-touching scenarios in both files.
✓ `tests/acceptance/us_10_aws_secrets.rs` — read the `no_iam_access_returns_backend_secret_fetch_failed`
scenario (lines 177-227), the only other nonexistent-ARN-against-LocalStack scenario in the
workspace, to check whether it is a sibling site needing the same fix.
✓ Workspace-wide grep for `retry_config(`/`RetryConfig::`/`timeout_config(`/`TimeoutConfig::` —
zero matches anywhere in the repo (test or production code) — this directly rules out the
"AWS SDK retry/backoff misconfiguration" hypothesis from the task brief; see § Root Cause below.

## Wave: DESIGN / [REF] Root Cause Investigation

### Finding 1 (ruled out): AWS SDK retry/backoff amplification

Neither `AwsSecretFetcher::new()` (`crates/embyr-server/src/adapters/aws_secret_fetcher.rs:37-44`)
nor the production call site (`crates/embyr-server/src/config.rs:477`,
`aws_config::load_defaults(aws_config::BehaviorVersion::latest())`) nor the test-harness client
builder (`tests/secrets_management/common/mod.rs:162-175`, `make_sm_client`) sets an explicit
`RetryConfig`/`TimeoutConfig` — a workspace-wide grep for those constructors returns zero matches.
The AWS SDK for Rust's default Smithy retry classifier does not retry
`GetSecretValueError::ResourceNotFoundException` — it is a modeled client error, not a
transient/throttling/5xx error, so it is not eligible for the default standard-mode retry policy
regardless of `max_attempts`. **Ruled out**: no evidence of multi-attempt amplification: the
production code path (`aws_secret_fetcher.rs:104-123`, `map_sdk_error`) explicitly maps this
exact error variant to `AwsSecretError::NotFound` from a single `SdkError::ServiceError` — the
code assumes and is written for a single round trip.

### Finding 2 (ruled out): generic LocalStack-container-not-ready race

`start_localstack()` (`tests/secrets_management/common/mod.rs:146-159`) already blocks on
`WaitFor::message_on_stdout("Ready.")` before returning — this is not "start the container and
race it," contrary to one framing in the task brief. The container's edge/gateway process is
confirmed listening before any test code proceeds.

### Finding 3 (confirmed root cause): cold vs. warm LocalStack Secrets-Manager backend, not container readiness

LocalStack's `"Ready."` stdout message signals only that its edge proxy is accepting
connections — it does not initialize each individual AWS service's backend eagerly. LocalStack
Community lazily initializes a given service's backend (here, Secrets Manager) on that service's
*first* API call in the container's lifetime; that first call pays an extra
initialization cost on top of normal request latency.

**Direct evidence, comparing every LocalStack-touching scenario in the two files named in
DISCUSS:**

| Test | File:line | Calls `create_raw_secret` (warms SM backend) before spawning server? | `wait_for_exit` bound | Reliable today? |
|---|---|---|---|---|
| `server_starts_with_admin_key_from_aws_secrets_manager` | sm01:49-84 | **Yes** (line 55) | 15s (after `wait_for_healthy(30s)`) | Yes |
| sentinel-value scenario | sm01:255-293 | **Yes** (line 260) | 15s (after `wait_for_healthy(30s)`) | Yes |
| `exits_1_when_admin_key_secret_fetch_fails` | sm01:207-237 | **No** | 10s | **No — the named failing test** |
| `totp_signin_*` rotation scenarios | sm02:48-193 | **Yes** (lines 52, 96) | 15s (after `wait_for_healthy(30s)`) | Yes |
| `sm02-bad-length-key` scenario | sm02:208-227 | **Yes** (line 213) | **10s** | **Yes** |
| `exits_1_when_encryption_key_secret_fetch_fails` | sm02:295-315ish | **No** | 10s | **No — the named failing test** |

The `sm02-bad-length-key` scenario (sm02:208-227) is the decisive control: it uses the *same*
10-second `wait_for_exit` bound as the two failing tests, against the *same* LocalStack
container-startup mechanism, but it calls `create_raw_secret` first — and it passes reliably
(it is not named in known-gaps.md #7). The only structural difference between it and the two
failing tests is the warm-up call. This isolates the variable: **10 seconds is already
sufficient once the Secrets Manager backend is warm; it is not sufficient when the server's own
`GetSecretValue` call is also the first-ever Secrets Manager call against that LocalStack
container.**

The two named failing tests (sm01:207-237, sm02:295ish) are the *only* LocalStack scenarios in
either file that call `start_localstack()` and then go straight to spawning the server with
`nonexistent_secret_arn()` — no `create_raw_secret`/`make_sm_client` call of any kind first. The
server subprocess's own `AwsSecretFetcher::get_raw_secret` → `client.get_secret_value()` call
(`aws_secret_fetcher.rs:90-102`) is therefore the first-ever request LocalStack's Secrets Manager
backend has ever seen in that container's lifetime — landing the lazy-initialization cost
squarely inside the same 10-second window that must also cover subprocess spawn, Tokio runtime
init, and `aws_config::load_defaults()`'s credential-chain resolution. On a loaded dev/CI
machine this combination reliably exceeds 10s. This matches known-gaps.md #7's own framing
("server+LocalStack container doesn't exit within the test's own 10s wait") precisely, and
explains why it reproduces consistently rather than randomly: the missing warm-up call is a
structural property of these 2 tests, not an environment fluke.

### Root cause statement

**Both failing tests omit the warm-up Secrets-Manager call that every other LocalStack-backed
scenario in these two files performs before starting its own timed wait, causing the timed
window to also absorb LocalStack's one-time, lazy, first-call Secrets-Manager backend
initialization cost — a cost the other scenarios pay before their clock starts.** This is a
test-harness construction gap, not a production-code defect and not a generic "10s is too
short" problem: the same 10s bound already passes reliably today whenever the backend is warm.

## Wave: DESIGN / [REF] Architecture Decision — ADR-style (inline, per this feature's
established single-file convention)

**Status**: Accepted.

**Context**: See § Root Cause Investigation above. AC-STR-03 requires the fix not weaken the
`exit_code == Some(1)` / stderr-substring / `!port_is_bound(...)` assertions. AC-STR-04 requires
a bounded, rationale-backed value, not an open-ended retry loop. DISCUSS's § Technical Notes
flagged that a production-code cause would be a scope escalation; this investigation found none
— production code (`aws_secret_fetcher.rs`, `config.rs`) requires no change.

**Decision**: Two changes, both confined to the two named test files, both test-harness-only:

1. **Primary (root-cause) fix** — in both `exits_1_when_admin_key_secret_fetch_fails`
   (sm01:207) and `exits_1_when_encryption_key_secret_fetch_fails` (sm02:295ish), after
   `start_localstack()` and before configuring the env vars for the doomed subprocess, build a
   client via the already-imported `make_sm_client(&endpoint_url)` and call the already-imported
   `create_raw_secret(&sm_client, <throwaway-name>, <throwaway-value>)` for a throwaway secret
   name distinct from `nonexistent_secret_arn()`'s target — exactly mirroring the pattern already
   proven reliable at sm02:208-227 (`sm02-bad-length-key`). This forces LocalStack's Secrets
   Manager backend to complete its lazy initialization during untimed setup, so the timed
   `wait_for_exit` window only has to cover what the already-reliable warmed-backend scenarios
   cover. No new helper functions are needed — `make_sm_client` and `create_raw_secret` already
   exist in `tests/secrets_management/common/mod.rs` and are already imported in both target
   files.
2. **Secondary (defense-in-depth) margin** — bump both tests' `wait_for_exit(Duration::from_secs(10))`
   to `Duration::from_secs(15)`. This reuses the exact `15`-second value already established and
   proven in these same two files for real subprocess-exit observation (sm01:84, sm01:149,
   sm02:67, sm02:161, and the `us_10_aws_secrets.rs`/rotation-window siblings) rather than
   inventing a new number — it is not a "wait longer and hope" guess, it is adopting an
   already-battle-tested bound from the same codebase for the same class of real-subprocess-exit
   wait. Given fix 1 already removes the actual race, this is pure headroom for ordinary
   process-spawn/network variance under CI load, at zero cost when unused (the wait returns as
   soon as the child exits; the bound is a ceiling, not a fixed delay).

**Alternatives considered**:
- *Make `start_localstack()` poll a real Secrets-Manager health check before returning* (task
  brief's option (c)) — rejected: `start_localstack()` already blocks on container readiness
  (`WaitFor::message_on_stdout("Ready.")`, Finding 2); the actual gap is service-level lazy
  initialization, not container-level readiness, and LocalStack does not expose a
  per-service "is Secrets Manager warm" health probe distinct from the general `/_localstack/health`
  endpoint (which reports service *availability*, not backend warm-state, and would not have
  caught this). Baking a Secrets-Manager-specific warm-up into the shared `start_localstack()`
  helper would also silently change behavior for the ~10 *other* callers of that helper across
  both files that don't need it (most already warm themselves explicitly) — broader blast radius
  for no additional benefit over fixing it at the two call sites that actually lack it.
- *Add explicit `RetryConfig`/`timeout_config` to `AwsSecretFetcher`* (task brief's option (b))
  — rejected: Finding 1 shows retries are not the mechanism; this would be a production-code
  change addressing a hypothesis the investigation disproved, and would contradict DISCUSS's "no
  production code change expected" framing without evidence to justify the escalation.
  **Scope decision, stated explicitly per the task brief's instruction**: no production-code
  opportunity was found; DISCUSS's assumption holds.
- *Timeout bump alone, no warm-up call* — rejected as the sole fix: it would be exactly the
  "wait longer and hope" pattern Domain Example 3 (DISCUSS) warns against — it treats the
  symptom (insufficient time) without removing the actual asymmetry (cold vs. warm backend), and
  offers no principled way to pick a value with a real rationale (how long does lazy-init take
  under worst-case CI load? unmeasured, unbounded confidence). The warm-up call is the
  root-cause fix; the timeout bump is deliberately secondary and optional-strength, not the
  primary mechanism.

**Consequences**:
- Positive: root cause addressed at the 2 actual defective call sites, using existing,
  already-imported helpers — zero new test infrastructure, zero production-code change, minimal
  diff (2 files, a few lines each).
- Positive: the fix is provably consistent with the rest of the codebase's own pattern (the
  `sm02-bad-length-key` control case) rather than a novel mechanism.
- Negative/accepted: the throwaway secret created for warm-up purposes is never asserted on and
  adds a small amount of setup time (one extra LocalStack round trip) to both tests — negligible
  next to the container startup cost already paid.

### AC-STR-03 compliance reasoning

Neither change touches `exit_code == Some(1)`, the stderr-substring assertions, or
`!ServerProcess::port_is_bound(server.admin_port)` in either test — those three lines are
byte-for-byte unchanged. The warm-up call creates a secret under a *different* name/ARN than
`nonexistent_secret_arn()`; the ARN the server is actually configured with and tested against
remains genuinely nonexistent in LocalStack, so the fetch the test exercises still genuinely
fails with a real `ResourceNotFoundException` — the correctness check is not weakened, only the
untimed setup phase changed. The timeout bump (10s→15s) changes only *how long* the test is
willing to wait for the existing assertions to become checkable; if a future regression made the
server incorrectly exit 0 or bind its admin port on a genuine fetch failure, `wait_for_exit`
would still return as soon as the (now-wrong) exit happens — well under 15s — and the existing
`assert_eq!`/`assert!` calls would still fail exactly as they do today. A hang (no exit at all)
would still time out and fail via `exit_code != Some(1)` (`wait_for_exit` returns `None` on
timeout), just 5 seconds later than before. No assertion's pass/fail outcome changes as a
function of either fix.

### AC-STR-04 compliance reasoning

Both values are concrete and finite (no unbounded retry loop introduced). The 15s bound is not
arbitrary: it is the same value already used and proven reliable in these same two files for
the same class of "wait for a real subprocess to exit" observation (sm01:84/149, sm02:67/161).
The warm-up call itself has no open-ended retry — `create_raw_secret` performs exactly one
`create_secret()` SDK call, matching the pattern already used identically in 4+ other passing
scenarios in these files.

## Wave: DESIGN / [REF] Handoff Package

**Files requiring a change (DELIVER):**
- `tests/secrets_management/acceptance/sm01_admin_key_secrets_manager.rs` — in
  `exits_1_when_admin_key_secret_fetch_fails` (currently starting at line 207): add one
  `make_sm_client` + `create_raw_secret` warm-up call pair before the existing env-var setup;
  bump the existing `wait_for_exit(Duration::from_secs(10))` (line 221) to
  `Duration::from_secs(15)`.
- `tests/secrets_management/acceptance/sm02_encryption_key_secrets_manager.rs` — in
  `exits_1_when_encryption_key_secret_fetch_fails` (currently starting around line 295): same
  two changes, mirrored.

**Files NOT requiring a change (production code, confirmed by investigation):**
- `crates/embyr-server/src/adapters/aws_secret_fetcher.rs` — no retry/timeout config exists; none
  needed (Finding 1).
- `crates/embyr-server/src/config.rs` — `from_env()`'s call order and AWS config construction
  are unrelated to the race; no change needed.
- `tests/secrets_management/common/mod.rs` — `start_localstack()`, `make_sm_client()`,
  `create_raw_secret()`, `nonexistent_secret_arn()`, and `ServerProcess` are reused as-is; no
  shared-helper change needed (Alternatives Considered: rejected baking warm-up into
  `start_localstack()` itself, to avoid changing behavior for the ~10 other callers that don't
  need it).

**Blast-radius check — other sites sharing the same pattern, confirmed NOT requiring the same
fix:**
- `tests/secrets_management/acceptance/sm01_admin_key_secrets_manager.rs:179` and
  `sm02_encryption_key_secrets_manager.rs:267` (`AmbiguousSecretSource` scenarios) — also 10s
  `wait_for_exit`, but `resolve_secret_source` returns before any I/O is attempted
  (`config.rs`'s own documented precedent), so no LocalStack call ever happens on either path;
  unaffected by this race by construction.
- `tests/secrets_management/acceptance/sm03_encryption_key_rotation.rs:352` and
  `sm04_admin_key_rotation.rs:289` (`startup_rejects_identical_current_and_previous_*_key`) — same
  pre-I/O-validation shape as above; no `start_localstack()` call in either scenario at all.
  Unaffected.
- `tests/production_readiness/acceptance/pr01_config_from_env.rs` (4 occurrences) and
  `pr04_graceful_shutdown.rs:60` — grepped; none call `start_localstack()`. Unrelated to this
  race, out of scope.
- `tests/acceptance/us_10_aws_secrets.rs::no_iam_access_returns_backend_secret_fetch_failed`
  (lines 177-227) — the only other nonexistent-ARN-against-LocalStack scenario in the workspace.
  Read in full: it asserts on an **HTTP response** (`reqwest::Client::post(...).send().await`,
  line 208-219) from an in-process test server, not a subprocess `wait_for_exit` deadline — the
  `reqwest` call simply blocks until LocalStack responds (no artificial ceiling), so it cannot
  exhibit this specific "deadline too short for cold backend" failure mode regardless of
  warm/cold state. Different mechanism; confirmed unaffected, no change needed.
- No other `start_localstack()` callers exist in the workspace (workspace-wide grep, confirmed
  exhaustive).

**External integrations note**: LocalStack here is a test-only emulator for AWS Secrets
Manager, not a production external dependency exercised by this feature — no contract-testing
annotation applies (the real AWS Secrets Manager integration and its own reliability are
unchanged by this feature; DISCUSS confirmed zero production behavior change).

**Mutation testing note** (per this project's `per-feature` strategy, DISCUSS DoD item 5): the
changed logic is entirely test-harness setup/timing (one extra SDK call, one `Duration` literal
change) — no new branching or domain logic is introduced. Per DISCUSS's own framing, this is
unlikely to produce meaningful mutants; QUALITY_GATE should confirm and may skip mutation testing
for this feature with that reasoning recorded, consistent with the DoD's own conditional wording.

## Wave: DESIGN / [REF] Peer Review

**Reviewer**: solution-architect-reviewer, iteration 1 of max 2.
**Scope note given to reviewer**: severity calibrated to actual artifact size (2-file
test-harness timing fix) — explicitly instructed not to flag absence of C4 diagrams or a
technology-stack ADR, which are correctly out of scope for this size of change.

**Result**: `approval_status: approved`, `critical_issues_count: 0`, `high_issues_count: 0`,
`issues_identified: {}` (empty across all 5 dimensions).

**Reviewer's verification findings** (summarized from full YAML):
- Q1 root-cause evidenced, not speculative: confirmed — control-case comparison table (§ Root
  Cause Investigation, Finding 3) and workspace-wide grep for retry/timeout config cited as
  decisive evidence.
- Q2 alternatives genuine, not strawmen: confirmed — all 3 rejected with specific rationale
  (blast-radius concern, Finding 1 cross-reference, Domain-Example-3 cross-reference).
- Q3 AC-STR-03/AC-STR-04 reasoning holds: confirmed — neither criterion quietly weakened.
- Q4 blast-radius/handoff package complete: confirmed exhaustive — 5 sibling sites checked with
  line-number evidence plus a workspace-wide grep confirming no other `start_localstack()`
  callers exist.
- Q5 (reviewer flagged): this DESIGN draft had initially declined a separate reviewer pass as
  disproportionate to scope; the reviewer noted that reasoning was sound in intent but that this
  invocation itself supersedes it — a review was in fact run, not skipped. Resolved by replacing
  the original "no reviewer pass" note (this section) with this actual review record.

**Priority validation** (from reviewer YAML): Q1 largest-bottleneck = YES (evidenced by
known-gaps.md #7 recurrence across 4+ FINALIZE gates). Q2 simple-alternatives = ADEQUATE. Q3
constraint-prioritization = CORRECT. Q4 data-justified = JUSTIFIED.

No revisions required — approved on iteration 1.

## Wave: DESIGN / [REF] Next Wave

**Handoff To**: nw-acceptance-designer (DISTILL wave), then nw-software-crafter (DELIVER wave).
**Deliverables**: root cause (§ Root Cause Investigation), the 2-part fix decision with
alternatives and AC-STR-03/04 compliance reasoning (§ Architecture Decision), the full Handoff
Package (files to change, files ruled out, blast-radius check), and the completed peer review
(§ Peer Review, approved, 0 critical/high issues) above. DISTILL should confirm the existing 4
UAT scenarios (DISCUSS § UAT Scenarios) still fully cover this mechanism as designed — no new
user-observable behavior is introduced, so no new scenario is expected, but DISTILL owns that
confirmation.
