# RED Classification — occ-precondition-validation

Per `nw-distill` § Pre-DELIVER fail-for-the-right-reason gate. Run 2026-09-10 against
current (unfixed) `crates/embyr-pg-storage/src/backend_adapter.rs` — `to_datetime` still
`.expect("valid timestamp")` at line 212 (confirmed unfixed, DESIGN-locked change not yet
applied — that is DELIVER's job).

## Scenario 1 — `update_document_with_malformed_nanos_precondition_returns_invalid_argument`
File: `tests/acceptance/us_02_write_document.rs`
Classification: **MISSING_FUNCTIONALITY (correct RED)**
Observed: server task panics at `backend_adapter.rs:212:10` (`"valid timestamp"`, the
`.expect()` this feature replaces). Tokio's per-task panic isolation contains the panic to
that one request — the test harness process does NOT crash. The client observes the broken
connection as `tonic::Code::Cancelled` ("h2 protocol error: http2 error"), not
`INVALID_ARGUMENT`. The test's own `assert_eq!(status.code(), tonic::Code::InvalidArgument, ...)`
fires cleanly (a business-logic assertion, not a setup/import/fixture error) — this is the
right kind of RED.

## Scenario 2 — `update_document_with_malformed_seconds_precondition_returns_invalid_argument`
File: `tests/acceptance/us_02_write_document.rs`
Classification: **MISSING_FUNCTIONALITY (correct RED)**
Observed: identical failure mode to Scenario 1 — panic at `backend_adapter.rs:212:10`,
client sees `Cancelled` ("h2 protocol error"), assertion on `InvalidArgument` fails cleanly.
Confirms `seconds`-out-of-range (`i64::MIN`) hits the SAME `.expect()` as `nanos` (the second
branch of `Utc.timestamp_opt(...).single()`), exactly as DISCUSS's own investigation predicted.

## Scenario 3 — `commit_transaction_with_malformed_nanos_precondition_returns_invalid_argument`
File: `tests/acceptance/us_06_transactions.rs`
Classification: **MISSING_FUNCTIONALITY (correct RED)**
Observed: identical failure mode, this time via the SECOND `to_datetime` call site
(`commit_transaction`'s own per-write OCC-verification loop, `backend_adapter.rs:1051`) —
confirms the transactional path panics independently of the single-document
`update_document` path, matching DESIGN's own two-call-site accounting. Panic still contained
to the one request; harness observed `Cancelled`, not a crash.

## Scenario 4 — `updating_with_malformed_nanos_precondition_returns_invalid_argument`
File: `tests/acceptance/embyr_agent/us_a02_write_operations.rs` (module `us_a02_write_operations`)
Classification: **MISSING_FUNCTIONALITY (correct RED)**
Observed: run via `cargo test -p embyr-agent --test embyr_agent -- --ignored` (test carries
the sibling-convention `#[ignore = "requires Docker"]` tag). Identical failure mode as
Scenarios 1-3 — panic at the SAME shared `backend_adapter.rs:212:10`, reached this time via
`embyr-agent`'s own independent `parse_precondition` → `PostgresBackendAdapter::update_document`
chain (not `embyr-server`'s `convert_precondition`). Empirically confirms DISCUSS's
Investigation Finding 2: both crates' independent precondition-parsing functions funnel into
the SAME shared helper, so a single fix in `embyr-pg-storage` closes both request paths.

## Gate result

All 4 scenarios: **PASS** (correct RED — MISSING_FUNCTIONALITY). Zero scenarios in the
IMPORT_ERROR / FIXTURE_BROKEN / SETUP_FAILURE / WRONG_ASSERTION categories. All 3
production-code files compile clean (`cargo test --no-run` for
`us_02_write_document`, `us_06_transactions`, `embyr_agent` targets — 0 errors). No RED
scaffolding was required (Mandate 7 N/A): `to_datetime`, both call sites, `CoreError`, and
both `core_error_to_status` implementations already exist in production code — this is a
signature-fallibility fix to an existing function, not new functionality requiring a stub.

Handoff to DELIVER: cleared.
