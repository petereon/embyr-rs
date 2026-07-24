# RED Classification — user-admin-ui

**Feature:** `user-admin-ui`
**Wave:** DISTILL
**Date:** 2026-06-14
**Classifier:** nw-acceptance-designer (Quinn)

Purpose: DELIVER reads this file at the RED phase entry gate to confirm every
ignored test fails for the right reason (MISSING_FUNCTIONALITY), not for setup
errors, import errors, or fixture bugs.

---

## Walking Skeleton Tests

| Test | File | Classification | Notes |
|------|------|----------------|-------|
| `admin_spa_http_probe` | `walking_skeleton.rs` | `GREEN_PENDING_SLICE_01` | Scaffold returns 501 Not Implemented; test asserts 200. Will fail with assertion error (right reason) once Slice 01 wires ServeDir. **Not #[ignore]** — runs every CI pass. |
| `wasm_bundle_size_gate` | `walking_skeleton.rs` | `GREEN_PENDING_TRUNK_BUILD` | Marked `#[ignore]` — skips if `admin-ui/dist/` absent. Once trunk build runs, test asserts `size < 5_000_000` bytes. Will be RED (assertion failure) if trunk hasn't run. |

---

## TEA State Machine Scenarios (tea_state_scenarios.rs)

All tests are `#[ignore]`. Classification is `RED` — `update()` panics with
`"Not yet implemented -- RED scaffold"` in every arm. The panic is a Rust
`panic!()` which is classified as an assertion failure (not an import/setup
error) by the DELIVER RED gate.

| Test name | US | Classification | Failure reason |
|-----------|----|----------------|----------------|
| `sign_in_sets_authed` | US-001 | RED | `update(_, Msg::SignIn)` panics |
| `sign_out_clears_authed` | US-001 | RED | `update(_, Msg::SignOut)` panics |
| `three_totp_failures_lock_account` | US-001 | RED | `update(_, Msg::TotpFailure)` panics |
| `totp_success_resets_failure_counter` | US-001 | RED | `update(_, Msg::TotpSuccess)` panics |
| `set_databases_populates_model` | US-002 | RED | `update(_, Msg::SetDatabases)` panics |
| `set_databases_excludes_deleted` | US-002 | RED | `update(_, Msg::SetDatabases)` panics |
| `database_created_appends_db` | US-003 | RED | `update(_, Msg::DatabaseCreated)` panics |
| `delete_database_removes_db_and_sdk_keys` | US-003 | RED | `update(_, Msg::DeleteDatabase)` panics |
| `set_db_status_updates_database` | US-003 | RED | `update(_, Msg::SetDbStatus)` panics |
| `set_db_status_missing_id_is_noop` | US-003 | RED | `update(_, Msg::SetDbStatus)` panics |
| `set_db_logging_enables_logging` | US-004 | RED | `update(_, Msg::SetDbLogging)` panics |
| `set_db_logging_disables_logging` | US-004 | RED | `update(_, Msg::SetDbLogging)` panics |
| `set_db_logging_wrong_id_is_noop` | US-004 | RED | `update(_, Msg::SetDbLogging)` panics |
| `patch_db_updates_backend_config` | US-005 | RED | `update(_, Msg::PatchDb)` panics |
| `patch_db_missing_id_is_noop` | US-005 | RED | `update(_, Msg::PatchDb)` panics |
| `sdk_key_created_appends_key` | US-006 | RED | `update(_, Msg::SdkKeyCreated)` panics |
| `revoke_sdk_key_removes_key` | US-006 | RED | `update(_, Msg::RevokeSdkKey)` panics |
| `revoke_sdk_key_missing_key_is_noop` | US-006 | RED | `update(_, Msg::RevokeSdkKey)` panics |
| `sdk_key_created_unknown_db_is_noop` | US-006 | RED | `update(_, Msg::SdkKeyCreated)` panics |
| `member_invited_appends_member` | US-009 | RED | `update(_, Msg::MemberInvited)` panics |
| `set_member_role_updates_role` | US-009 | RED | `update(_, Msg::SetMemberRole)` panics |
| `remove_member_removes_non_owner` | US-009 | RED | `update(_, Msg::RemoveMember)` panics |
| `sole_owner_invariant_holds_after_member_removal` | US-009 | RED | `update(_, Msg::RemoveMember)` panics |
| `sole_owner_invariant_holds_after_role_change` | US-009 | RED | `update(_, Msg::SetMemberRole)` panics |
| `remove_member_missing_uid_is_noop` | US-009 | RED | `update(_, Msg::RemoveMember)` panics |
| `service_account_created_appends` | US-010 | RED | `update(_, Msg::ServiceAccountCreated)` panics |
| `delete_service_account_removes_it` | US-010 | RED | `update(_, Msg::DeleteServiceAccount)` panics |
| `admin_key_created_appends` | US-010 | RED | `update(_, Msg::AdminKeyCreated)` panics |
| `revoke_admin_key_removes_key` | US-010 | RED | `update(_, Msg::RevokeAdminKey)` panics |
| `revoke_admin_key_missing_is_noop` | US-010 | RED | `update(_, Msg::RevokeAdminKey)` panics |
| `toggle_oidc_flips_enabled` | US-011 | RED | `update(_, Msg::ToggleOidc)` panics |
| `toggle_oidc_double_toggle_restores` | US-011 | RED | `update(_, Msg::ToggleOidc)` panics |
| `push_toast_appends` | US-011 | RED | `update(_, Msg::PushToast)` panics |
| `dismiss_toast_removes_toast` | US-011 | RED | `update(_, Msg::DismissToast)` panics |
| `dismiss_toast_missing_id_is_noop` | US-011 | RED | `update(_, Msg::DismissToast)` panics |
| `push_many_toasts_does_not_panic` | US-011 | RED | `update(_, Msg::PushToast)` panics |

---

## Per-Slice Focused Scenarios (slice_0N_*.rs)

All tests are `#[ignore]`. Classification: `RED` for the same reason — every
`update()` arm panics. Listed below are the unique scenario names per slice;
the failure reason is always `update(_, Msg::XYZ)` panics.

### Slice 01 — Auth + Dashboard (slice_01_auth_dashboard.rs)

| Test | US | Classification |
|------|----|----------------|
| `sign_in_with_mock_credentials_sets_authed` | US-001 | RED |
| `sign_out_clears_session` | US-001 | RED |
| `three_totp_failures_lock_account_for_fifteen_minutes` | US-001 | RED |
| `dashboard_shows_all_databases_after_load` | US-002 | RED |
| `dashboard_empty_state_when_no_databases` | US-002 | RED |
| `dashboard_suspended_visible_deleted_hidden` | US-002 | RED |

### Slice 02 — Database Management (slice_02_database_management.rs)

| Test | US | Classification |
|------|----|----------------|
| `new_database_appears_in_list_after_creation` | US-003 | RED |
| `suspend_database_changes_status_to_suspended` | US-003 | RED |
| `reactivate_database_restores_active_status` | US-003 | RED |
| `delete_database_removes_it_and_cascades_sdk_keys` | US-003 | RED |
| `delete_nonexistent_database_is_noop` | US-003 | RED |
| `enable_query_logging_updates_model` | US-004 | RED |
| `disable_query_logging_updates_model` | US-004 | RED |
| `set_log_retention_stores_period` | US-004 | RED |
| `enable_logging_wrong_id_is_noop` | US-004 | RED |

### Slice 03 — Connections + SDK Keys (slice_03_connections_sdk_keys.rs)

| Test | US | Classification |
|------|----|----------------|
| `patch_db_updates_dsn_field` | US-005 | RED |
| `patch_db_updates_agent_endpoint` | US-005 | RED |
| `patch_db_unknown_id_is_noop` | US-005 | RED |
| `create_sdk_key_appends_to_model` | US-006 | RED |
| `revoke_sdk_key_removes_it_from_model` | US-006 | RED |
| `delete_database_cascades_revokes_all_sdk_keys` | US-006 | RED |
| `revoke_sdk_key_nonexistent_is_noop` | US-006 | RED |
| `create_sdk_key_for_unknown_db_is_noop` | US-006 | RED |

### Slice 04 — Identity + Admin Keys (slice_04_identity_admin_keys.rs)

| Test | US | Classification |
|------|----|----------------|
| `invite_member_appears_with_pending_status` | US-009 | RED |
| `promote_viewer_to_admin` | US-009 | RED |
| `remove_viewer_member` | US-009 | RED |
| `removing_sole_owner_is_rejected` | US-009 | RED |
| `demoting_sole_owner_is_rejected` | US-009 | RED |
| `remove_member_phantom_uid_is_noop` | US-009 | RED |
| `create_service_account_appends_to_model` | US-010 | RED |
| `delete_service_account_cascades_admin_keys` | US-010 | RED |
| `create_admin_key_appends_with_prefix` | US-010 | RED |
| `revoke_admin_key_removes_from_model` | US-010 | RED |
| `revoke_admin_key_phantom_is_noop` | US-010 | RED |

### Slice 05 — Billing + Logs (slice_05_billing_logs.rs)

| Test | US | Classification |
|------|----|----------------|
| `logs_tab_empty_when_logging_disabled` | US-007 | RED |
| `logs_become_visible_after_enabling_logging` | US-007 | RED |
| `log_retention_stored_when_logging_enabled` | US-007 | RED |
| `set_log_retention_wrong_db_is_noop` | US-007 | RED |
| `billing_data_source_populated_from_set_databases` | US-008 | RED |
| `billing_empty_state_when_no_databases` | US-008 | RED |

### Slice 06 — Settings (slice_06_settings.rs)

| Test | US | Classification |
|------|----|----------------|
| `add_oidc_provider_appends_to_list` | US-011 | RED |
| `toggle_oidc_disables_enabled_provider` | US-011 | RED |
| `toggle_oidc_enables_disabled_provider` | US-011 | RED |
| `toggle_oidc_phantom_id_is_noop` | US-011 | RED |
| `remove_oidc_provider_removes_it` | US-011 | RED |
| `danger_zone_action_pushes_error_toast` | US-011 | RED |
| `dismiss_toast_removes_it_from_queue` | US-011 | RED |
| `dismiss_toast_phantom_id_is_noop` | US-011 | RED |
| `toggle_one_oidc_provider_does_not_affect_others` | US-011 | RED |

---

## Summary

| Classification | Count |
|----------------|-------|
| RED (correct) | ~80 scenarios across tea_state + per-slice files |
| GREEN_PENDING_SLICE_01 | 1 (admin_spa_http_probe) |
| GREEN_PENDING_TRUNK_BUILD | 1 (wasm_bundle_size_gate, #[ignore]) |
| BROKEN | 0 |

**Pre-DELIVER gate result: PASS** — all ignored tests fail with `panic!("Not yet implemented -- RED scaffold")`, which is the Rust equivalent of `AssertionError`. No import errors, no fixture errors. DELIVER may begin enabling tests one at a time.

**DELIVER start point**: Enable `sign_in_with_mock_credentials_sets_authed` (slice 01, simplest auth state transition) or `set_databases_populates_model` (slice 01, dashboard load).
