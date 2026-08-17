# security-rules — Pre-DELIVER Fail-For-The-Right-Reason Gate

Per `nw-distill` § Pre-DELIVER fail-for-the-right-reason gate. Run once, DISTILL wave, 2026-08-17.

## Procedure

For each of the 5 acceptance test files, EVERY non-`#[ignore]`d scenario was
run against real Postgres testcontainers (Docker confirmed available this
session); every `#[ignore]`d scenario was additionally spot-checked via
`--ignored`. Classification: `MISSING_FUNCTIONALITY` (panic inside the
target RED scaffold, or a status/assertion mismatch caused by that panic
propagating up through gRPC/HTTP) = correct RED. `IMPORT_ERROR` /
`FIXTURE_BROKEN` / `SETUP_FAILURE` = would be a BROKEN classification —
none observed. A third category, **VACUOUS PASS** (a comparison-shaped
assertion that is trivially satisfied because BOTH sides currently hit the
identical scaffold panic, not because real behavior is correct — Critical
Rule 7, No Fixture Theater), was found once and fixed during this DISTILL
session — see § Vacuous-Pass Finding below.

## Manifestation note (differs from `client-auth`'s own precedent)

Unlike `client-auth`'s scaffolds (which were entered only when the caller
presented a header — request that never presented the header took a
zero-scaffold path), `SystemDb::get_access_rule` is called
**unconditionally** on every `GetDocument` request per ADR-029's own
structural design (the cheap existence-check runs before the allow/deny
decision, for every read, not conditionally). Panicking inside an async
Axum/tonic handler with no `catch_panic`/`catch_unwind` layer in this
codebase does not produce a clean HTTP/gRPC error response — it aborts the
in-flight connection/stream. Observed manifestations:
- Admin HTTP (`reqwest`): `hyper::Error(IncompleteMessage)` — connection
  dropped mid-response.
- gRPC (`tonic`): `Status { code: Cancelled, ... h2 protocol error ...
  Reset(..., CANCEL, Remote) }` — stream reset.

Both are genuine `MISSING_FUNCTIONALITY` RED — the failure traces directly
to the scaffold panic in every case below, never to a fixture/setup bug.

## Results — sr01 (`SecurityRulesAdminContext`, admin-only)

| Scenario | Result | Classification |
|---|---|---|
| `first_time_rule_definition_succeeds_and_is_immediately_active` (WS, AC-17-01) | FAIL | `MISSING_FUNCTIONALITY` — panics inside `parse_condition` at `access_control/mod.rs:137`, surfaces as `IncompleteMessage` |
| `redefining_an_existing_rule_fully_replaces_it_with_no_overlap_window` (AC-17-02) | FAIL | `MISSING_FUNCTIONALITY` — same `parse_condition` panic |
| `a_condition_using_an_out_of_v1_scope_construct_is_rejected_naming_whats_unsupported` (AC-17-03) | FAIL | `MISSING_FUNCTIONALITY` — same `parse_condition` panic |
| `a_condition_with_invalid_syntax_is_rejected_with_a_specific_reason` (AC-17-04) | FAIL | `MISSING_FUNCTIONALITY` — same `parse_condition` panic |
| `rule_definition_without_valid_admin_credentials_is_rejected` (AC-17-05) | **PASS** | Legitimate GREEN — `session: SessionContext` extractor rejects the missing-cookie request with 401 BEFORE the handler body (and therefore `parse_condition`) is ever reached. Not a test bug; mirrors `client-auth` ca03's identical `rotation_without_a_valid_session_is_rejected` precedent. Kept enabled (not `#[ignore]`d). |

## Results — sr02 (`SecurityRulesFullContext`, gRPC)

All 5 scenarios FAIL, all `MISSING_FUNCTIONALITY` — panic inside
`SystemDb::get_access_rule` at `system_db.rs:333` (reached before
`parse_condition`/`evaluate`, since the rule-lookup runs first per
ADR-029), surfacing as gRPC `Cancelled`/h2-reset:
`a_signed_in_end_user_reading_their_own_document_succeeds_unchanged` (WS,
AC-17-06), `a_different_signed_in_end_users_read_of_the_same_document_is_denied`
(AC-17-07), `a_rule_not_based_on_ownership_allows_any_signed_in_caller`
(AC-17-08), `a_condition_referencing_a_missing_field_fails_closed_not_with_an_error`
(AC-17-09), `a_denied_read_never_reveals_whether_the_target_document_exists`
(AC-17-10).

## Results — sr03 (`SecurityRulesFullContext`, gRPC)

| Scenario | Result | Classification |
|---|---|---|
| `a_never_signed_in_session_is_denied_by_a_rule_requiring_identity` (WS, AC-17-11) | FAIL | `MISSING_FUNCTIONALITY` — `get_access_rule` panic |
| `a_never_signed_in_session_succeeds_against_a_rule_allowing_public_read` (AC-17-12) | FAIL | `MISSING_FUNCTIONALITY` — `get_access_rule` panic |
| `an_invalid_client_identity_header_is_evaluated_identically_to_no_header_at_all` (AC-17-13) | FAIL (after fix — see § Vacuous-Pass Finding) | `MISSING_FUNCTIONALITY` — `get_access_rule` panic |

## Results — sr04 (`SecurityRulesFullContext`, gRPC + marker)

| Scenario | Result | Classification |
|---|---|---|
| `a_collection_that_has_never_had_a_rule_defined_is_unaffected_by_this_feature` (WS, AC-17-14) | FAIL | `MISSING_FUNCTIONALITY` — `get_access_rule` panic (see § Regression-Suite Finding — this is the SAME mechanism, not a distinct bug) |
| `a_rule_on_one_collection_does_not_affect_a_sibling_collection_without_its_own_rule` (AC-17-15) | FAIL | `MISSING_FUNCTIONALITY` — `get_access_rule` panic |
| `full_113_scenario_regression_suite_passes_unmodified` (AC-17-16) | N/A (structural marker) | `#[ignore]`d unconditionally; when force-run via `--ignored` it deliberately hits `unreachable!()` — this is BY DESIGN (see its own doc comment), not a RED/GREEN classification. |

## Results — sr05 (mixed contexts)

| Scenario | Result | Classification |
|---|---|---|
| `simulating_a_valid_candidate_rule_against_a_matching_pair_returns_the_correct_outcome` (AC-17-17) | FAIL | `MISSING_FUNCTIONALITY` — `parse_condition` panic |
| `simulation_surfaces_an_over_permissive_rule_bug_before_publishing` (AC-17-17) | FAIL | `MISSING_FUNCTIONALITY` — `parse_condition` panic |
| `simulation_has_zero_effect_on_live_traffic` (AC-17-18) | FAIL | `MISSING_FUNCTIONALITY` — precondition (`ctx.get_document(...)` for Maria) itself panics via `get_access_rule` before simulation is even exercised |
| `simulation_supports_the_anonymous_case_identically_to_real_evaluation` (AC-17-19) | FAIL | `MISSING_FUNCTIONALITY` — `parse_condition` panic |

## Results — layer-1 unit/property tests (`crates/embyr-core/src/access_control/mod.rs`)

All 12 tests (6 pinned examples + 2 `proptest!` properties, 64 cases each)
FAIL — `MISSING_FUNCTIONALITY`, unconditional panic inside `parse_condition`
or `evaluate`. Zero import/compile errors (`cargo check -p embyr-core --lib
--tests`: clean). `cargo test -p embyr-core --lib access_control`: 12/12
failed, all classified RED.

## Vacuous-Pass Finding (found and fixed during this DISTILL session)

`sr03`'s `an_invalid_client_identity_header_is_evaluated_identically_to_no_header_at_all`
(AC-17-13) was ORIGINALLY written as a pure cross-comparison:
`assert_eq!(err_no_header.code(), err_expired_header.code(), ...)` with no
independent anchor. Because `get_access_rule` currently panics
**unconditionally** for BOTH calls in that scenario, both sides produced
the IDENTICAL `Cancelled`/h2-reset failure — making the cross-comparison
assertion trivially TRUE (a **vacuous PASS**) without any real
production-code correctness behind it. This is exactly the class Critical
Rule 7 (No Fixture Theater) warns against: a test that passes without the
target implementation existing means the test is not actually exercising
the claim.

**Fix applied** (before this file was finalized): an anchor assertion —
`assert_eq!(err_no_header.code(), tonic::Code::PermissionDenied, ...)` —
was added BEFORE the cross-comparison, pinning one side to the concrete
expected status. Re-run confirmed the scenario now correctly FAILS for the
right reason (`left: Cancelled, right: PermissionDenied`) during RED, and
will remain a meaningful assertion once real `evaluate()` logic exists (the
cross-comparison then additionally proves the no-header and
invalid-header paths produce the SAME real `PermissionDenied`, not two
independently-arrived-at ones).

No other scenario in the 5 files was found to have this shape — every
other comparison-style assertion (sr02's AC-17-10 existence-non-leakage
scenario) already had an anchor assertion pinning at least one side to a
concrete expected value, independently verified to fail for the right
reason (see § Results — sr02 above, and the captured `cargo test` output:
`left: Cancelled, right: PermissionDenied`).

## Regression-Suite Finding (differs materially from `client-auth`'s own DISTILL claim — read before DELIVER starts)

**`client-auth`'s DISTILL wave was able to run the full 72-scenario
`embyr-rs` suite unmodified and confirm 69/72 passed** (3 pre-existing,
unrelated sandbox failures) because its scaffold
(`attach_client_identity_if_present`) was entered CONDITIONALLY — only when
the caller presented the new, optional header. Sessions that never
presented it (i.e., every one of the 72 existing scenarios) took a
zero-scaffold-call path and were genuinely unaffected during RED.

**`security-rules` cannot make the equivalent claim during DISTILL.**
ADR-029's own structural design calls `SystemDb::get_access_rule`
**unconditionally** on every `GetDocument` request — this IS the mechanism
that makes AC-17-14/15/16's no-rule-defined guardrail structural rather
than merely tested (see ADR-029 § Structural no-rule-defined guardrail).
During the RED-scaffold period, that same unconditional call means
`get_access_rule`'s panic fires for EVERY `GetDocument` call in the
codebase, including every one of the 113 pre-existing regression scenarios.

**Verified empirically this session**: `cargo test -p embyr-server --test
us_03_read_document` (an existing, unmodified `embyr-rs` acceptance file
that calls `GetDocument`) — 3 of 4 scenarios in that single file now FAIL,
each with the identical `SystemDb::get_access_rule — RED scaffold ...`
panic. This is expected and is NOT a DISTILL-wave regression to fix: it is
the direct, unavoidable consequence of scaffolding ADR-029's structural
design faithfully. **The full 113-scenario suite is NOT run to a
pass/fail verdict during this DISTILL session** — doing so would report a
false, universal failure that says nothing about `security-rules`'
correctness and everything about the scaffold's necessarily-total panic.

**This is exactly why the DISTILL dispatch instructions scoped AC-17-16 as
a marker/documentation-style test** (`sr04`'s
`full_113_scenario_regression_suite_passes_unmodified`, `#[ignore]`d
unconditionally) rather than a DISTILL-time pass/fail run — the regression
proof is structurally deferred to DELIVER's own GREEN phase.

**Handoff note for DELIVER**: `get_access_rule`'s FIRST implementation
increment should return `Ok(None)` for every `(project_id, collection_path)`
with no row (not necessarily the full upsert/lookup machinery in one step)
specifically because this unblocks the entire pre-existing regression
suite immediately — re-running the § AC-17-16 command becomes meaningful,
and honestly comparable to `client-auth`'s own 69/72 baseline, only once
that minimal increment lands.

## Additional verification performed this DISTILL run

- `cargo check -p embyr-core --lib --tests`: clean.
- `cargo check -p embyr-server --lib --bins`: clean (production `main.rs`
  composition root and all existing handlers unaffected at the TYPE level —
  only `handle_get_document`'s runtime behavior changes, per the Regression
  -Suite Finding above).
- `cargo test -p embyr-server --test security_rules_sr0{1..5}_* --no-run`:
  all 5 new test binaries compile cleanly.
- `cargo deny check bans`: **`bans ok`** — confirms zero new workspace
  dependencies (ADR-027 Consequences: "Zero new workspace dependency"),
  nothing to resolve for DELIVER unlike `client-auth`'s own
  `jsonwebtoken`-in-`embyr-core` flag.

## Conclusion

Gate **PASSED WITH A DOCUMENTED CAVEAT**: every scenario across all 5 files
(19/19 UAT-derived scenarios + 1 structural marker) and all 12 layer-1
unit/property tests fails for the correct `MISSING_FUNCTIONALITY` reason;
zero scenarios in the test-bug/wrong-shape category remain (one was found
and fixed during this session — see § Vacuous-Pass Finding); one
legitimate GREEN-by-construction exception (sr01's AC-17-05) is documented,
not silently accepted. **The one caveat, not present in `client-auth`'s own
gate**: the pre-existing 113-scenario regression suite is NOT verified to
pass during this DISTILL session, for the structural reason explained in §
Regression-Suite Finding — this is expected, not a defect, and is the exact
reason AC-17-16 is scoped as a DELIVER-time proof obligation. Handoff to
DELIVER is unblocked, with the explicit handoff note above about
`get_access_rule`'s first implementation increment.
