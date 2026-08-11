# Pre-DELIVER Fail-for-the-Right-Reason Gate — card-payments

Per `nw-distill` § Pre-DELIVER fail-for-the-right-reason gate. Procedure: force-run every
`#[ignore]` test with `--ignored`, capture the panic, classify.

## Commands run

```
cargo test -p embyr-admin-ui --test card_payments_walking_skeleton -- --ignored --test-threads=1
cargo test -p embyr-admin-ui --test card_payments_tea_state        -- --ignored --test-threads=1
```

## Result summary

| Metric | Count |
|---|---|
| Total `#[test]` fns (both binaries) | 65 |
| MISSING_FUNCTIONALITY (correct RED) | 65 |
| IMPORT_ERROR / FIXTURE_BROKEN / SETUP_FAILURE (wrong RED — test bug) | 0 |
| WRONG_ASSERTION / OBSERVABLE_NOT_AT_PORT (wrong shape) | 0 |
| Compile errors (BROKEN) | 0 |

**Verdict: PASS.** All 65 tests fail for the right reason — every panic originates from a
`// SCAFFOLD: true` / `RED scaffold (card-payments): ... not yet implemented` marker inside
production code (`model.rs`, `update.rs`, or `data.rs`), never from an import error, missing
fixture, or a test harness problem. Handoff to DELIVER is unblocked.

Note: a single, pre-existing, unrelated failure exists in the sibling `user_admin_ui_tea_state`
binary (`slice_04_identity_admin_keys::delete_service_account_cascades_admin_keys`, AC-010-03) —
confirmed via `git stash` to predate this DISTILL session. It is a `user-admin-ui` regression, out
of scope for this feature, and does not affect the card-payments RED classification below.

## Classification detail (one line per test)

Legend: `model.rs` = `impl AppModel` derivation/projection method panic. `update.rs` = new `Msg`
match-arm panic. `data.rs` = new pure function panic (`next_invoice_estimate`, `bar_color`,
`detect_card_brand`, `card_number_is_complete`).

### `card_payments_walking_skeleton` (Slice 01, 6 tests)

| Test | Panics in | Classification |
|---|---|---|
| `chris_sees_free_plan_and_included_volume` | `model.rs` (`plan_summary`) | MISSING_FUNCTIONALITY |
| `dana_sees_pro_plan_base_price_and_renewal` | `model.rs` (`plan_summary`) | MISSING_FUNCTIONALITY |
| `chris_with_card_sees_payment_method_summary` | `model.rs` (`payment_method_summary`) | MISSING_FUNCTIONALITY |
| `priya_with_no_card_sees_add_card_cta` | `model.rs` (`payment_method_summary`) | MISSING_FUNCTIONALITY |
| `chris_can_open_card_modal_and_upgrade_modal_from_overview` | `update.rs` (`Msg::OpenCardModal`) | MISSING_FUNCTIONALITY |
| `chris_sees_invoices_tab_stub_empty_state` | `model.rs` (`invoices_empty_state`) | MISSING_FUNCTIONALITY |

### `card_payments_tea_state` (Slices 02–08 + properties, 59 tests)

| Test | Panics in | Classification |
|---|---|---|
| `slice_02::bar_color_boundary_at_exactly_100_percent_is_red` | `data.rs` (`bar_color`) | MISSING_FUNCTIONALITY |
| `slice_02::bar_color_boundary_at_exactly_80_percent_is_amber` | `data.rs` (`bar_color`) | MISSING_FUNCTIONALITY |
| `slice_02::bar_color_just_under_100_percent_is_amber` | `data.rs` (`bar_color`) | MISSING_FUNCTIONALITY |
| `slice_02::bar_color_just_under_80_percent_is_accent` | `data.rs` (`bar_color`) | MISSING_FUNCTIONALITY |
| `slice_02::brand_new_account_shows_zero_usage_without_false_alarms` | `model.rs` (`cap_ratios`) | MISSING_FUNCTIONALITY |
| `slice_02::deletes_bar_goes_red_at_or_above_cap` | `model.rs` (`cap_ratios`) | MISSING_FUNCTIONALITY |
| `slice_02::per_database_write_breakdown_sums_to_card_total` | `model.rs` (`usage_table_rows`) | MISSING_FUNCTIONALITY |
| `slice_02::pro_plan_never_treated_as_cap_exceeded_even_over_free_thresholds` | `model.rs` (`cap_exceeded`) | MISSING_FUNCTIONALITY |
| `slice_02::reads_well_under_cap_renders_accent` | `model.rs` (`cap_ratios`) | MISSING_FUNCTIONALITY |
| `slice_02::writes_bar_goes_amber_approaching_cap` | `model.rs` (`cap_ratios`) | MISSING_FUNCTIONALITY |
| `slice_03::account_with_no_databases_keeps_existing_empty_state` | `model.rs` (`usage_table_rows`) | MISSING_FUNCTIONALITY |
| `slice_03::kpi_summary_tiles_total_across_all_databases` | `model.rs` (`usage_table_rows`) | MISSING_FUNCTIONALITY |
| `slice_03::newly_created_database_shows_proportionally_low_not_missing_usage` | `model.rs` (`usage_table_rows`) | MISSING_FUNCTIONALITY |
| `slice_03::no_overage_projects_base_only_invoice` | `data.rs` (`next_invoice_estimate`) | MISSING_FUNCTIONALITY |
| `slice_03::overage_estimate_breaks_down_by_dimension_for_reads` | `data.rs` (`next_invoice_estimate`) | MISSING_FUNCTIONALITY |
| `slice_03::storage_overage_uses_flat_gb_rate_not_per_100k_unit` | `data.rs` (`next_invoice_estimate`) | MISSING_FUNCTIONALITY |
| `slice_03::total_equals_base_plus_sum_of_overage_line_items` | `data.rs` (`next_invoice_estimate`) | MISSING_FUNCTIONALITY |
| `slice_03::usage_table_shows_real_per_database_numbers` | `model.rs` (`usage_table_rows`) | MISSING_FUNCTIONALITY |
| `slice_04::brand_detected_as_mastercard_from_number_prefix` | `data.rs` (`detect_card_brand`) | MISSING_FUNCTIONALITY |
| `slice_04::brand_detected_as_visa_from_number_prefix` | `data.rs` (`detect_card_brand`) | MISSING_FUNCTIONALITY |
| `slice_04::brand_unknown_for_unrecognized_prefix` | `data.rs` (`detect_card_brand`) | MISSING_FUNCTIONALITY |
| `slice_04::empty_card_number_is_not_complete` | `data.rs` (`card_number_is_complete`) | MISSING_FUNCTIONALITY |
| `slice_04::formatted_sixteen_digit_card_number_is_complete` | `data.rs` (`card_number_is_complete`) | MISSING_FUNCTIONALITY |
| `slice_04::incomplete_twelve_digit_card_number_is_not_complete` | `data.rs` (`card_number_is_complete`) | MISSING_FUNCTIONALITY |
| `slice_04::submitting_replaces_not_appends_existing_card` | `update.rs` (`Msg::SetCard`) | MISSING_FUNCTIONALITY |
| `slice_04::submitting_valid_card_updates_subscription_card` | `update.rs` (`Msg::SetCard`) | MISSING_FUNCTIONALITY |
| `slice_05::closing_without_confirming_makes_no_plan_change` | `update.rs` (`Msg::OpenUpgradeModal`) | MISSING_FUNCTIONALITY |
| `slice_05::compare_step_shows_free_and_pro_columns_from_plan_features` | `update.rs` (`Msg::OpenUpgradeModal`) | MISSING_FUNCTIONALITY |
| `slice_05::confirming_downgrade_updates_plan_to_free_and_closes_modal` | `update.rs` (`Msg::OpenUpgradeModal`) | MISSING_FUNCTIONALITY |
| `slice_05::confirming_upgrade_updates_plan_to_pro_and_closes_modal` | `update.rs` (`Msg::OpenUpgradeModal`) | MISSING_FUNCTIONALITY |
| `slice_05::downgrade_warning_does_not_show_on_upgrade_path` | `update.rs` (`Msg::OpenUpgradeModal`) | MISSING_FUNCTIONALITY |
| `slice_05::selecting_downgrade_shows_mandatory_hard_cap_warning` | `update.rs` (`Msg::OpenUpgradeModal`) | MISSING_FUNCTIONALITY |
| `slice_05::upgrading_from_cap_exceeded_clears_suspended_state` | `model.rs` (`cap_exceeded`) | MISSING_FUNCTIONALITY |
| `slice_06::account_with_invoice_history_does_not_show_empty_state_even_on_free` | `update.rs` (`Msg::SetInvoices`) | MISSING_FUNCTIONALITY |
| `slice_06::free_plan_admin_with_no_invoice_history_sees_empty_state` | `model.rs` (`invoices_empty_state`) | MISSING_FUNCTIONALITY |
| `slice_06::invoice_history_persists_across_plan_downgrade` | `update.rs` (`Msg::SetInvoices`) | MISSING_FUNCTIONALITY |
| `slice_06::paid_invoices_and_upcoming_invoice_are_distinguished_by_status` | `update.rs` (`Msg::SetInvoices`) | MISSING_FUNCTIONALITY |
| `slice_06::pro_admin_sees_itemized_invoice_rows_via_set_invoices` | `update.rs` (`Msg::SetInvoices`) | MISSING_FUNCTIONALITY |
| `slice_07::banner_visible_regardless_of_active_section` | `model.rs` (`suspension_banner_view`) | MISSING_FUNCTIONALITY |
| `slice_07::both_conditions_true_prioritizes_cap_exceeded` | `model.rs` (`effective_status`) | MISSING_FUNCTIONALITY |
| `slice_07::cap_exceeded_shows_amber_banner_with_upgrade_cta` | `model.rs` (`suspension_banner_view`) | MISSING_FUNCTIONALITY |
| `slice_07::clicking_payment_cta_opens_card_modal` | `update.rs` (`Msg::OpenCardModal`) | MISSING_FUNCTIONALITY |
| `slice_07::clicking_upgrade_cta_opens_upgrade_modal` | `update.rs` (`Msg::OpenUpgradeModal`) | MISSING_FUNCTIONALITY |
| `slice_07::healthy_account_shows_no_banner` | `model.rs` (`suspension_banner_view`) | MISSING_FUNCTIONALITY |
| `slice_07::payment_failed_shows_red_banner_with_payment_cta` | `model.rs` (`suspension_banner_view`) | MISSING_FUNCTIONALITY |
| `slice_08::toggle_mutates_only_payment_failure_field` | `update.rs` (`Msg::SetPaymentFailure`) | MISSING_FUNCTIONALITY |
| `slice_08::toggle_to_same_value_twice_is_idempotent` | `update.rs` (`Msg::SetPaymentFailure`) | MISSING_FUNCTIONALITY |
| `slice_08::toggling_back_to_payment_succeeds_clears_banner` | `update.rs` (`Msg::SetPaymentFailure`) | MISSING_FUNCTIONALITY |
| `slice_08::toggling_to_payment_fails_triggers_red_banner` | `model.rs` (`effective_status`) | MISSING_FUNCTIONALITY |
| `tea_state_scenarios::cap_ratio_reads_matches_usage_totals_over_free_caps` | `model.rs` (`cap_ratios`) | MISSING_FUNCTIONALITY |
| `tea_state_scenarios::free_cap_exceeded_implies_free_plan` | `model.rs` (`effective_status`) | MISSING_FUNCTIONALITY |
| `tea_state_scenarios::free_plan_cap_exceeded_iff_any_dimension_at_or_over_cap` | `model.rs` (`cap_ratios`) | MISSING_FUNCTIONALITY |
| `tea_state_scenarios::pinned_example_single_dimension_at_cap_is_sufficient` | `model.rs` (`cap_exceeded`) | MISSING_FUNCTIONALITY |
| `tea_state_scenarios::pro_plan_never_shows_free_cap_exceeded` | `model.rs` (`effective_status`) | MISSING_FUNCTIONALITY |
| `tea_state_scenarios::read_only_iff_status_not_active` | `model.rs` (`effective_status`) | MISSING_FUNCTIONALITY |
| `tea_state_scenarios::set_card_always_replaces_never_accumulates` | `update.rs` (`Msg::SetCard`) | MISSING_FUNCTIONALITY |
| `tea_state_scenarios::set_plan_is_idempotent` | `update.rs` (`Msg::SetPlan`) | MISSING_FUNCTIONALITY |
| `tea_state_scenarios::usage_totals_reads_equals_sum_times_thirty` | `model.rs` (`usage_totals`) | MISSING_FUNCTIONALITY |
| `tea_state_scenarios::zero_databases_never_falsely_trip_cap_exceeded` | `model.rs` (`cap_exceeded`) | MISSING_FUNCTIONALITY |

## DELIVER entry note

DELIVER reads this file at PREPARE phase to confirm RED is genuine (ADR-025 D2 — this gate is the
RED phase entry/exit gate). Each row above names the exact scaffold symbol (`model.rs`/`update.rs`/
`data.rs`) that must be implemented to flip that test GREEN. Recommended enablement order: Slice 01
(WS) first, then Slices 02→08 in DISCUSS's build-order (`wave-decisions.md` respects the
dependency chain: 01→02→04→05→03→06→07→08 per the prioritization note — Slice 07 depends on 04/05
existing as CTA targets), enabling exactly one `#[ignore]` at a time per Mandate 5.
