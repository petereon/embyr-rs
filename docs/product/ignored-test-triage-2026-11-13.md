# Ignored-test triage — 2026-11-13

Source: CI run executing only `#[ignore]`d tests (`cargo test --workspace --no-fail-fast -- --ignored`, 2026-11-11): 78 passed, 40 failed of 118 real `#[ignore]` attributes under `tests/` (the "213" figure also counted doc-comment mentions). Panic messages were read from the full log; product code and feature evolution docs were consulted per failure.

Classifications: **PASS** (un-ignored), **FAIL-stale** (test wrong/scaffold; fixed, implemented or deleted), **FAIL-real** (product gap; left ignored with `REAL GAP` reason), **FAIL-env** (needs Docker image/Stripe/GCP emulator etc.; left ignored with a reason).

Counts: FAIL-env: 9, FAIL-real: 4, FAIL-stale: 27, PASS: 75, PASS (kept ignored): 3 (total 118).

The newly implemented / fixed / un-ignored tests are unverified locally (8 GB laptop, Docker-backed); the PR CI run is their verification.

| File | Test | Class | Action | Reason |
|---|---|---|---|---|
| `acceptance/embyr_agent/us_a02_write_operations.rs` | `updating_with_mask_preserves_unmentioned_fields` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `acceptance/embyr_agent/us_a02_write_operations.rs` | `removing_absent_document_succeeds_without_error` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `acceptance/embyr_agent/us_a02_write_operations.rs` | `removing_document_leaves_deletion_record` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `acceptance/embyr_agent/us_a02_write_operations.rs` | `increment_transform_on_absent_field_treats_starting_value_as_zero` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `acceptance/embyr_agent/us_a02_write_operations.rs` | `creating_document_without_identifier_generates_one` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `acceptance/embyr_agent/us_a02_write_operations.rs` | `concurrent_write_on_same_generation_is_rejected` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `acceptance/embyr_agent/us_a02_write_operations.rs` | `updating_absent_document_returns_not_found` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `acceptance/embyr_agent/us_a02_write_operations.rs` | `creating_document_that_already_exists_is_rejected` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `acceptance/embyr_agent/us_a02_write_operations.rs` | `updating_with_malformed_nanos_precondition_returns_invalid_argument` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `acceptance/embyr_agent/us_a02_write_operations.rs` | `document_generation_advances_by_one_on_every_successful_write` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `acceptance/embyr_agent/us_a02_write_operations.rs` | `creating_new_document_stores_it_at_generation_one` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `acceptance/us_05_listen_realtime.rs` | `subscription_with_multi_field_filter_without_index_is_rejected` | FAIL-real | kept #[ignore] (REAL GAP) | Listen has no composite-index gate (only RunQuery); test body is also still a panic! scaffold. |
| `card_payments_backend/acceptance/cpb05_usage_metering.rs` | `a_single_projects_stripe_failure_does_not_abort_the_whole_run` | FAIL-env | kept #[ignore], reason string added | Real-Stripe fixture cannot fail exactly 1 of 3 projects sharing one Stripe customer (ESCALATED step 02-01); run_metering isolation is code-implemented. |
| `deployment_release_process/acceptance/drp02_compose_lifecycle.rs` | `compose_up_restart_and_reset_round_trip` | PASS (kept ignored) | kept #[ignore], reason string added | Real docker compose I/O, ~8 min, 2 containers; documented never-in-default-run (8 GB laptop). Passes when run explicitly. |
| `distributed_rate_limiting/acceptance/b11_provision_initializes_bucket.rs` | `provision_creates_rate_bucket_row` | FAIL-stale | implemented + un-ignored | Was todo!(); now POSTs /admin/v1/projects against a separate customer PG and asserts the rate_buckets row. |
| `distributed_rate_limiting/acceptance/b11_provision_initializes_bucket.rs` | `provision_rate_bucket_is_transactional` | FAIL-stale | implemented + un-ignored | Was todo!(); now forces the rate_buckets INSERT to fail (NOT VALID CHECK(false)) and asserts no orphan projects row. |
| `distributed_rate_limiting/acceptance/b12_postgres_rate_limit.rs` | `tokens_refill_after_one_second` | FAIL-stale | implemented + un-ignored | Was todo!(); now asserts exhausted bucket rejects, then refills after 1.1 s. |
| `distributed_rate_limiting/acceptance/b12_postgres_rate_limit.rs` | `rate_limit_enforced_at_configured_rps` | FAIL-stale | DELETED (duplicate) | todo!() scaffold; capacity-then-RESOURCE_EXHAUSTED is covered by b19 pg_healthy_rate_limiting_behaves_unchanged (AC-RLFO-01). |
| `distributed_rate_limiting/acceptance/b12_postgres_rate_limit.rs` | `rate_bucket_tokens_decrease_atomically` | FAIL-stale | implemented + un-ignored | Was todo!(); now asserts rate_buckets.tokens ~4/3/2 after 3 allowed requests. |
| `distributed_rate_limiting/acceptance/b13_fallback_on_pg_failure.rs` | `fallback_resolves_within_20ms` | FAIL-stale | DELETED (duplicate; file b13 removed) | todo!() scaffold; covered by b19 AC-RLFO-06 (starved pool -> 20 ms timeout -> fallback + pg_timeout counter). |
| `distributed_rate_limiting/acceptance/b13_fallback_on_pg_failure.rs` | `fallback_allows_requests_when_pg_unavailable` | FAIL-stale | DELETED (duplicate; file b13 removed) | todo!() scaffold; covered by b19 AC-RLFO-02/03 (PG error -> per-instance fallback allows). |
| `distributed_rate_limiting/acceptance/b13_fallback_on_pg_failure.rs` | `fallback_capped_at_configured_limit` | FAIL-stale | DELETED (duplicate; file b13 removed) | todo!() scaffold; covered by b19 AC-RLFO-05 (fallback bucket enforces capacity per project). |
| `distributed_rate_limiting/acceptance/b14_ratelimit_headers.rs` | `rate_limited_response_includes_retry_after_ms` | FAIL-stale | implemented + un-ignored | Was todo!(); asserts positive retry-after-ms on RESOURCE_EXHAUSTED. |
| `distributed_rate_limiting/acceptance/b14_ratelimit_headers.rs` | `all_grpc_handlers_return_ratelimit_headers` | FAIL-stale | implemented + un-ignored | Was todo!(); asserts x-ratelimit-* on an allowed GetDocument (representative handler), retry-after-ms absent. |
| `distributed_rate_limiting/acceptance/b14_ratelimit_headers.rs` | `ratelimit_remaining_decrements_per_request` | FAIL-stale | implemented + un-ignored | Was todo!(); asserts remaining header 4,3,2. |
| `observability/acceptance/obs01_prometheus_endpoint.rs` | `metrics_endpoint_not_exposed_on_grpc_port` | FAIL-stale | implemented + un-ignored, RENAMED metrics_endpoint_not_exposed_on_data_port | Was todo!(); gRPC port is HTTP/2-only tonic so the check targets the REST data port (404, no metrics). |
| `observability/acceptance/obs01_prometheus_endpoint.rs` | `metrics_endpoint_returns_200_with_prometheus_content_type` | FAIL-stale | implemented + un-ignored | Was todo!(); asserts 200, text/plain; charset=utf-8, # TYPE lines. |
| `observability/acceptance/obs01_prometheus_endpoint.rs` | `metrics_response_contains_help_and_type_lines` | FAIL-real | implemented, kept #[ignore] (REAL GAP) | Product never calls describe_*!(), so /metrics has # TYPE but no # HELP (AC-OBS-01-01). |
| `observability/acceptance/obs01_prometheus_endpoint.rs` | `metrics_endpoint_requires_operator_bearer_auth` | FAIL-stale | implemented + un-ignored | Was todo!(); asserts 401 without Authorization. |
| `observability/acceptance/obs01_prometheus_endpoint.rs` | `metrics_endpoint_rejects_wrong_bearer_token` | FAIL-stale | implemented + un-ignored | Was todo!(); asserts 401 with wrong token. |
| `observability/acceptance/obs02_grpc_request_counters.rs` | `pg_timeout_increments_embyr_rate_limit_pg_timeout_total` | FAIL-stale | DELETED (duplicate) | todo!() scaffold; covered by b19 existing_timeout_fallback_is_unchanged_by_this_fix (AC-RLFO-06). |
| `observability/acceptance/obs02_grpc_request_counters.rs` | `grpc_error_increments_requests_total_with_error_status` | FAIL-stale | implemented + un-ignored | Was todo!(); asserts status="unauthenticated" counter >= 1. |
| `observability/acceptance/obs02_grpc_request_counters.rs` | `rate_limit_rejection_increments_embyr_rate_limit_rejected_total` | FAIL-stale | DELETED (duplicate) | todo!() scaffold; covered by obs04 rejected_request_increments_rate_limit_rejected_total. |
| `observability/acceptance/obs03_grpc_latency_histograms.rs` | `grpc_request_records_latency_histogram` | FAIL-stale | implemented + un-ignored | Was todo!(); asserts _count{method="GetDocument"} >= 1. |
| `observability/acceptance/obs03_grpc_latency_histograms.rs` | `histogram_output_contains_bucket_and_sum_and_count` | FAIL-stale | implemented + un-ignored | Was todo!(); asserts 12 buckets + +Inf, _sum, _count. |
| `observability/acceptance/obs04_rate_limit_metrics.rs` | `allowed_request_increments_rate_limit_allowed_total` | FAIL-stale | implemented + un-ignored | Was todo!(); asserts outcome="allowed" per project. |
| `observability/acceptance/obs04_rate_limit_metrics.rs` | `rejected_request_increments_rate_limit_rejected_total` | FAIL-stale | implemented + un-ignored | Was todo!(); asserts outcome="rejected" per project (limiter sized to 2 tokens). |
| `observability/acceptance/obs04_rate_limit_metrics.rs` | `rate_limit_metrics_labeled_by_project_id` | FAIL-stale | implemented + un-ignored | Was todo!(); asserts a noisy project does not produce rejected samples for a quiet one. |
| `observability/acceptance/obs05_postgres_pool_metrics.rs` | `pool_size_gauge_present_in_metrics` | FAIL-stale | implemented + un-ignored | Was todo!(); asserts embyr_pg_pool_size{pool="system"} >= 1. |
| `observability/acceptance/obs05_postgres_pool_metrics.rs` | `pool_idle_gauge_present_in_metrics` | FAIL-stale | implemented + un-ignored | Was todo!(); asserts embyr_pg_pool_idle{pool="system"} <= size. |
| `production_readiness/acceptance/pr01_config_from_env.rs` | `exits_1_when_admin_key_missing` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr01_config_from_env.rs` | `exits_1_when_encryption_key_missing` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr01_config_from_env.rs` | `exits_1_when_encryption_key_not_64_hex_chars` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr01_config_from_env.rs` | `exits_1_when_database_unreachable` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr01_config_from_env.rs` | `exits_1_when_db_probe_fails_projects_table_missing` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr01_config_from_env.rs` | `no_port_bound_on_missing_required_env_var` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr01_config_from_env.rs` | `non_default_ports_respected` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr01_config_from_env.rs` | `grpc_port_env_var_respected` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr01_config_from_env.rs` | `rest_port_env_var_respected` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr01_config_from_env.rs` | `default_rate_limit_rps_is_1000` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr01_config_from_env.rs` | `exits_1_when_database_url_missing` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr02_dockerfile.rs` | `docker_image_under_100mb` | FAIL-real | kept #[ignore] (REAL GAP) | Built image is 150 MB (150468042 B); evolution doc/AC-PR-02-03 claim < 100 MB. |
| `production_readiness/acceptance/pr02_dockerfile.rs` | `docker_image_exposes_correct_ports` | FAIL-env | kept #[ignore], reason string added | Same shared-image-tag race / needs pre-built image. |
| `production_readiness/acceptance/pr02_dockerfile.rs` | `docker_image_entrypoint_is_embyr_server` | FAIL-env | kept #[ignore], reason string added | Same shared-image-tag race / needs pre-built image. |
| `production_readiness/acceptance/pr02_dockerfile.rs` | `cargo_chef_rebuild_fast` | PASS (kept ignored) | kept #[ignore], reason string added | Asserts <60 s rebuild; only true with warm Docker layer cache. |
| `production_readiness/acceptance/pr02_dockerfile.rs` | `docker_build_succeeds` | PASS (kept ignored) | kept #[ignore], reason string added | Full `docker build` (minutes uncached); CI `docker` job already builds the image. |
| `production_readiness/acceptance/pr02_dockerfile.rs` | `docker_image_runs_as_non_root` | FAIL-env | kept #[ignore], reason string added | Needs pre-built image; failed because 2 tests concurrently `docker build -t` the same tag. |
| `production_readiness/acceptance/pr03_ci.rs` | `ci_yaml_has_test_job_with_postgres_service` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr03_ci.rs` | `ci_yaml_has_lint_job_with_clippy_and_deny` | FAIL-stale | fixed + un-ignored | `cargo deny check` moved to cargo-deny.yml (finding #23); test now reads that workflow. |
| `production_readiness/acceptance/pr03_ci.rs` | `ci_yaml_has_docker_job_depending_on_test` | FAIL-stale | fixed + un-ignored | docker job uses docker/build-push-action (finding #42), not a literal `docker build`; test updated. |
| `production_readiness/acceptance/pr03_ci.rs` | `ci_yaml_clippy_uses_deny_warnings` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr03_ci.rs` | `ci_yaml_triggers_on_push_to_master` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr03_ci.rs` | `ci_yaml_triggers_on_pull_request` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr04_graceful_shutdown.rs` | `in_flight_request_completes_before_shutdown` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr04_graceful_shutdown.rs` | `sigterm_drains_within_30s` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr04_graceful_shutdown.rs` | `sigterm_causes_graceful_shutdown` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr05_tls_support.rs` | `exits_nonzero_when_only_cert_path_is_set` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr05_tls_support.rs` | `exits_nonzero_when_only_key_path_is_set` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr05_tls_support.rs` | `exits_nonzero_when_key_file_does_not_exist` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr05_tls_support.rs` | `exits_nonzero_when_cert_content_is_not_valid_pem` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr06_stripe_webhook_secret_required.rs` | `exits_nonzero_when_stripe_secret_key_set_without_webhook_signing_secret` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr06_stripe_webhook_secret_required.rs` | `exits_nonzero_when_stripe_webhook_signing_secret_is_empty_string` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr06_stripe_webhook_secret_required.rs` | `webhook_route_is_reachable_when_stripe_correctly_configured` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr06_stripe_webhook_secret_required.rs` | `webhook_route_not_reachable_when_stripe_fully_unconfigured` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr09_wire_secret_fetchers.rs` | `gcp_secret_project_wiring_reached_when_token_configured` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr09_wire_secret_fetchers.rs` | `server_with_neither_cloud_configured_starts_and_direct_pg_unaffected` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr09_wire_secret_fetchers.rs` | `aws_secret_project_fails_closed_when_aws_credentials_absent` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr09_wire_secret_fetchers.rs` | `gcp_secret_provisioning_rejected_when_token_unset` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr09_wire_secret_fetchers.rs` | `transaction_sweeper_reclaims_aws_secret_project_when_aws_credentials_available` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr11_pool_sizing_tenant_pool.rs` | `one_tenants_saturated_pool_does_not_affect_another_tenants_pool` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr11_pool_sizing_tenant_pool.rs` | `tenant_pool_default_max_connections_preserved_when_unset` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr12_pool_sizing_invalid_config.rs` | `non_numeric_listener_max_connections_fails_startup` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr12_pool_sizing_invalid_config.rs` | `non_numeric_listener_acquire_timeout_fails_startup` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr12_pool_sizing_invalid_config.rs` | `zero_tenant_max_connections_fails_startup` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr12_pool_sizing_invalid_config.rs` | `zero_listener_acquire_timeout_fails_startup` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr12_pool_sizing_invalid_config.rs` | `non_numeric_system_max_connections_fails_startup` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr12_pool_sizing_invalid_config.rs` | `non_numeric_tenant_acquire_timeout_fails_startup` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `production_readiness/acceptance/pr13_pool_sizing_listener_pool.rs` | `listen_still_delivers_document_changes_with_listener_pool_fields_threaded_through` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `secrets_management/acceptance/sm01_admin_key_secrets_manager.rs` | `server_starts_with_admin_key_from_gcp_secret_manager` | FAIL-env | kept #[ignore] (reason already accurate) | Blocked on OQ-SM-4: no GcpSecretFetcher base_url override; body is a not-implemented stub. |
| `security_rules/acceptance/sr04_untouched_collections_and_regression_suite_unaffected.rs` | `full_113_scenario_regression_suite_passes_unmodified` | FAIL-env | kept #[ignore] (reason already accurate) | Structural marker (unreachable!() proof obligation over EXTERNAL test binaries); documented permanent ignore. |
| `security_rules_cel_parity/acceptance/cp05_non_interference_and_idempotency.rs` | `full_security_rules_regression_suite_passes_unmodified` | FAIL-env | kept #[ignore] (reason already accurate) | Structural marker (unreachable!() proof obligation over EXTERNAL test binaries); documented permanent ignore. |
| `security_rules_query_path/acceptance/no_rule_and_sibling_collection_regression_guardrail.rs` | `full_198_scenario_regression_suite_passes_unmodified` | FAIL-env | kept #[ignore] (reason already accurate) | Structural marker (unreachable!() proof obligation over EXTERNAL test binaries); documented permanent ignore. |
| `security_rules_write_path/acceptance/read_write_independence_and_regression.rs` | `full_133_scenario_regression_suite_passes_unmodified` | FAIL-env | kept #[ignore] (reason already accurate) | Structural marker (unreachable!() proof obligation over EXTERNAL test binaries); documented permanent ignore. |
| `user_admin_ui/acceptance/slice_02_database_management.rs` | `delete_nonexistent_database_is_noop` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_02_database_management.rs` | `enable_query_logging_updates_model` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_02_database_management.rs` | `disable_query_logging_updates_model` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_02_database_management.rs` | `set_log_retention_stores_period` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_02_database_management.rs` | `enable_logging_wrong_id_is_noop` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_02_database_management.rs` | `delete_database_removes_it_and_cascades_sdk_keys` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_03_connections_sdk_keys.rs` | `revoke_sdk_key_removes_it_from_model` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_03_connections_sdk_keys.rs` | `delete_database_cascades_revokes_all_sdk_keys` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_03_connections_sdk_keys.rs` | `revoke_sdk_key_nonexistent_is_noop` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_03_connections_sdk_keys.rs` | `create_sdk_key_for_unknown_db_is_noop` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_03_connections_sdk_keys.rs` | `create_sdk_key_appends_to_model` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_04_identity_admin_keys.rs` | `demoting_sole_owner_is_rejected` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_04_identity_admin_keys.rs` | `remove_member_phantom_uid_is_noop` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_04_identity_admin_keys.rs` | `create_service_account_appends_to_model` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_04_identity_admin_keys.rs` | `delete_service_account_cascades_admin_keys` | FAIL-real | kept #[ignore] (REAL GAP) | update(Msg::DeleteServiceAccount) only removes the SA; its admin keys stay (AC-010-03, Msg doc says cascades). |
| `user_admin_ui/acceptance/slice_04_identity_admin_keys.rs` | `create_admin_key_appends_with_prefix` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_04_identity_admin_keys.rs` | `revoke_admin_key_removes_from_model` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_04_identity_admin_keys.rs` | `revoke_admin_key_phantom_is_noop` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_04_identity_admin_keys.rs` | `removing_sole_owner_is_rejected` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_06_settings.rs` | `remove_oidc_provider_removes_it` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_06_settings.rs` | `danger_zone_action_pushes_error_toast` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_06_settings.rs` | `dismiss_toast_removes_it_from_queue` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_06_settings.rs` | `dismiss_toast_phantom_id_is_noop` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_06_settings.rs` | `toggle_one_oidc_provider_does_not_affect_others` | PASS | un-ignored | Passed in the ignored-only CI run. |
| `user_admin_ui/acceptance/slice_06_settings.rs` | `toggle_oidc_phantom_id_is_noop` | PASS | un-ignored | Passed in the ignored-only CI run. |

## REAL GAPS

1. **Listen does not reject multi-field filters without a READY composite index** — `tests/acceptance/us_05_listen_realtime.rs::subscription_with_multi_field_filter_without_index_is_rejected`. Documented (AC-04f applied to Listen): FAILED_PRECONDITION. Observed: `handle_listen` has no `requires_composite_index` check (only `RunQuery`, `handler.rs` ~L3247). The test body is still a `panic!` scaffold. Severity: Low-Medium (real Firestore rejects; embyr would serve the unindexed listen).
2. **`/metrics` has no `# HELP` lines** — `tests/observability/acceptance/obs01_prometheus_endpoint.rs::metrics_response_contains_help_and_type_lines`. Documented (AC-OBS-01-01, observability evolution doc): body has `# HELP` and `# TYPE`. Observed: no `describe_counter!/describe_histogram!/describe_gauge!` calls exist in `crates/`, and metrics-exporter-prometheus emits `# HELP` only for described metrics. (Inferred from code; not seen failing in CI — the ignored-only run only had the `todo!()` body.) Severity: Low (Prometheus scrapes fine; Grafana metric help text missing).
3. **Docker image is ~150 MB, limit < 100 MB** — `tests/production_readiness/acceptance/pr02_dockerfile.rs::docker_image_under_100mb`. Documented: AC-PR-02-03 / production-readiness evolution doc ("< 100 MB image"). Observed in CI: 150468042 bytes (debian:bookworm-slim runtime + release binary). Severity: Low (size only; fix the claim or slim the image).
4. **UI model: `DeleteServiceAccount` does not cascade admin keys** — `tests/user_admin_ui/acceptance/slice_04_identity_admin_keys.rs::delete_service_account_cascades_admin_keys`. Documented: AC-010-03 and the `Msg::DeleteServiceAccount` doc comment ("cascades to revoke its admin keys"). Observed: `crates/embyr-admin-ui/src/update.rs` only `retain`s service_accounts; keys with that `service_account_id` remain (`88 passed; 1 ignored` locally on the tea_state target). Note migration 0017 (`admin_api_keys_sa_fk_set_null`) sets the server-side FK to NULL rather than cascading, so the AC itself may be the stale side — needs a product decision. Severity: Low (mock-data SPA model).

## Notes / unsure

- Four `full_*_regression_suite_passes_unmodified` tests and `server_starts_with_admin_key_from_gcp_secret_manager` keep their pre-existing documented ignore reasons.
- `a_single_projects_stripe_failure_does_not_abort_the_whole_run` is FAIL-env, not a product bug: `projects_failed: 0` is the real-Stripe fixture limit documented by the ESCALATION comment.
- pr02 Docker tests: three of them failed only because parallel tests re-tag one image concurrently; all six stay ignored (the CI `docker` job already builds the image).
- Doc comments claiming "All tests are #[ignore] (RED)" remain in files with no real `#[ignore]` attribute left (e.g. `tests/card_payments/**`, `tests/admin_api_v2/**`, `tests/card_payments_backend/**`); out of scope here.
