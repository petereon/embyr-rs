# RED Classification — embyr-rs DISTILL scaffolds

> Wave: DISTILL
> Date: 2026-05-24
> Purpose: documents that all scaffold tests are RED (panic! body), not BROKEN
>   (no import errors, compile errors, or setup failures)

---

## Classification method

Each scaffold test file:
1. Contains no imports of production modules (production crates do not exist yet)
2. Has a `panic!("RED scaffold — not yet implemented")` as the sole test body
3. Is annotated `#[ignore = "..."]` so it is skipped by default
4. Is annotated `#[tokio::test]` for async compatibility

When `cargo test -- --ignored` is run against these files:
- All tests fail with `panicked at 'RED scaffold — not yet implemented'`
- No compilation errors (Rust compiles the test bodies regardless of `#[ignore]`)
- The failure type is `PanicInfo` — classified as RED (assertion-level failure), not BROKEN

**Rust classification rule:**
- `panic!(...)` → RED (assertion failure — implementation missing, test correct)
- `compile_error!(...)` or missing imports → BROKEN (infrastructure failure)
- Skipped with `#[ignore]` marker → PENDING (scaffolded, not yet enabled)

---

## Scaffold inventory

| File | Tests | Classification |
|------|-------|----------------|
| `tests/acceptance/walking_skeleton.rs` | 1 | RED |
| `tests/acceptance/us_01_configure_sdk.rs` | 3 | RED |
| `tests/acceptance/us_02_write_document.rs` | 5 | RED |
| `tests/acceptance/us_03_read_document.rs` | 4 | RED |
| `tests/acceptance/us_04_query_collection.rs` | 8 | RED |
| `tests/acceptance/us_05_listen_realtime.rs` | 8 | RED |
| `tests/acceptance/us_06_transactions.rs` | 4 | RED |
| `tests/acceptance/us_07_provision_project.rs` | 7 | RED |
| `tests/acceptance/us_08_monitor_project.rs` | 4 | RED |
| `tests/acceptance/us_09_suspend_project.rs` | 5 | RED |
| `tests/acceptance/us_10_aws_secrets.rs` | 4 | RED |
| `tests/acceptance/us_11_gcp_secrets.rs` | 4 | RED |
| `tests/acceptance/us_12_agent_backend.rs` | 7 | RED |
| `tests/acceptance/us_13_browser_transport.rs` | 4 | RED |
| `tests/acceptance/us_14_rate_limiting.rs` | 4 | RED |

**Total scaffold tests:** 72 tests classified RED

---

## Pre-DELIVER gate requirement

Before DELIVER begins work on any user story, the crafter must:
1. Unskip the first test for that story (remove `#[ignore]` annotation)
2. Confirm the test fails with `panicked at 'RED scaffold — not yet implemented'`
3. Proceed with the RED → GREEN → COMMIT cycle

A test that fails for any other reason (import error, compile error, setup failure) is
classified BROKEN and must be fixed before the TDD cycle begins. This gate is the
fail-for-the-right-reason check per ADR-025.

---

## Implementation order (from prioritization.md)

| Order | Story | First test to unskip |
|-------|-------|----------------------|
| 1 | US-01 / US-03 (S01) | `walking_skeleton.rs::sdk_developer_retrieves_written_document_via_grpc` |
| 2 | US-02 (S02) | `us_02_write_document.rs::write_then_read_returns_same_document_fields` |
| 3 | US-07 (S10) | `us_07_provision_project.rs::provision_project_returns_201_with_key_and_applies_migrations` |
| 4 | US-03 additional | `us_03_read_document.rs::get_existing_document_returns_correct_fields` |
| 5 | US-04 (S04) | `us_04_query_collection.rs::where_filter_returns_only_matching_documents` |
| 6 | US-06 (S09) | `us_06_transactions.rs::ten_concurrent_transaction_increments_produce_correct_final_value` |
| 7 | US-04 composite (S05) | `us_04_query_collection.rs::query_without_ready_index_returns_failed_precondition` |
| 8 | US-05 snapshot (S06) | `us_05_listen_realtime.rs::listen_delivers_full_initial_snapshot` |
| 9 | US-05 live (S07) | `us_05_listen_realtime.rs::write_triggers_listen_callback_within_two_seconds` |
| 10 | US-05 resume (S08) | `us_05_listen_realtime.rs::reconnect_with_resume_token_delivers_only_delta` |
| 11 | US-09 (S11) | `us_09_suspend_project.rs::suspend_causes_sdk_permission_denied_within_one_second` |
| 12 | US-13 (S12) | `us_13_browser_transport.rs::grpc_web_client_reads_and_writes_documents` |
| 13 | US-12 (S13) | `us_12_agent_backend.rs::agent_starts_with_required_env_vars_and_logs_readiness` |
| 14 | US-10 + US-11 (S14) | `us_10_aws_secrets.rs::provision_with_aws_secret_stores_arn_not_dsn` |
| 15 | US-14 (S15) | `us_14_rate_limiting.rs::burst_above_default_returns_resource_exhausted` |
