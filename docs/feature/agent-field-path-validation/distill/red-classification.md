# RED Classification — agent-field-path-validation

Pre-DELIVER fail-for-the-right-reason gate (`nw-distill` skill). Run against
the CURRENT, unfixed `crates/embyr-agent/src/server.rs` (DESIGN's fix not yet
applied — that is DELIVER's job).

Command: `cargo test --test embyr_agent -p embyr-agent`
Result: `41 passed; 6 failed; 11 ignored` — all 6 failures below, zero
unrelated regressions.

| Test | File | Classification | Empirically observed pre-fix behavior |
|---|---|---|---|
| `run_query_rejects_sql_injection_probe_field_path_with_single_quote` | `tests/acceptance/embyr_agent/us_a09_field_path_validation.rs` | MISSING_FUNCTIONALITY (correct RED) | `field_path = "age' OR '1'='1"` reaches `embyr-pg-storage`'s raw-interpolating SQL builder, producing invalid Postgres syntax. The resulting `sqlx::Error` is caught by `sanitize_backend_error` → `Status::internal("internal server error")`, not `Status::invalid_argument`. |
| `run_aggregation_query_rejects_sql_injection_probe_field_path_with_single_quote` | same | MISSING_FUNCTIONALITY (correct RED) | Identical root cause via `RunAggregationQuery` (same call site). Confirmed: `Internal: internal server error`. |
| `run_query_rejects_named_drop_table_injection_payload_and_documents_table_survives` | same | MISSING_FUNCTIONALITY (correct RED) | `field_path = "x'); DROP TABLE documents; --"` breaks SQL string-literal syntax the same way; confirmed `Internal: internal server error`. (The `documents` table is not actually dropped — the semicolon/`--` never escape the JSONB `->` operand into a second executable statement via `sqlx::QueryBuilder`'s parameter binding; the failure is a Postgres *syntax* error on the single malformed expression, not statement injection. This is not asserted as a new finding beyond what DISCUSS/DESIGN already characterized — it is the concrete mechanism behind the `Status::internal` observed.) |
| `composite_filter_rejects_malformed_leaf_field_path_among_valid_leaves` | same | MISSING_FUNCTIONALITY (correct RED) — **worse-than-crash case** | `field_path = "name; DROP TABLE documents; --"` (no unescaped quote) stays inside the single-quoted SQL string literal, so it does NOT break SQL syntax. Observed: request completes normally, `RunQueryResponse { document: None, continuation_selector: Done(true) }` — silently accepted, zero rows, no error of any kind. This is the "successful-but-wrong" failure mode named in the task brief: the confused-deputy validator not only fails to error, it gives no signal at all that a spec-violating field path was submitted. |
| `consecutive_dot_field_path_is_now_accepted_and_matches_zero_documents` | same | MISSING_FUNCTIONALITY (correct RED — this is a positive-behavior test, not a negative one) | Against unfixed code, `field_path = "order..amount"` is REJECTED by the old local validator (`Status::invalid_argument("invalid field path 'order..amount': consecutive dots not allowed")`). Test asserts the NEW, DESIGN-approved behavior (accepted, zero rows) — fails today because the fix (which permits consecutive dots, matching `embyr-server`'s own existing behavior) has not landed yet. |
| `query_with_malformed_field_path_rejected_before_data_read` (modified) | `tests/acceptance/embyr_agent/us_a03_query_operations.rs` | MISSING_FUNCTIONALITY (correct RED) | Payload changed from `"order..amount"` (no longer malformed post-fix, see Finding 3) to `"order amount"` (space). Old validator does not reject spaces either (only checks `".."`), and a space also stays inside the quoted SQL literal — same silent-accept, zero-row, no-error behavior as the composite-filter case above. |

## Cross-binary parity test (separate binary)

| Test | File | Classification | Empirically observed pre-fix behavior |
|---|---|---|---|
| `identical_malformed_field_path_rejected_identically_by_both_backend_modes` | `tests/agent_field_path_validation/acceptance/afp06_cross_binary_parity.rs` (registered as `agent_field_path_validation_afp06_cross_binary_parity` under `embyr-server`) | MISSING_FUNCTIONALITY (correct RED) | Command: `cargo test --test agent_field_path_validation_afp06_cross_binary_parity -p embyr-server` → `4 passed; 1 failed`. The `embyr-server` (`backend_mode=direct_pg`) half of the assertion passes today (it already calls the real charset guard via `translate_filter`). The `embyr-agent` (`backend_mode=agent`) half fails: `field_path = "name;DROP TABLE documents;"` is silently accepted (`RunQueryResponse { document: None, continuation_selector: Done(true) }`), never rejected — confirming the exact cross-binary asymmetry finding #11 named. |

## Not RED (confirmed correct today AND after the fix — regression guards)

| Test | Reason |
|---|---|
| `simple_field_name_filter_continues_to_work_on_run_query` | `"status"` is spec-compliant under both old and new validators. |
| `simple_field_name_filter_continues_to_work_on_run_aggregation_query` | Same, via Count. |
| `dotted_nested_field_path_filter_continues_to_work_on_run_query` | `"address.city"` is spec-compliant under both validators. |
| `create_document_accepts_a_field_name_with_special_characters_write_path_unaffected` | Write paths never call `validate_field_path` (field names become JSON object keys inside a bound `$N::jsonb` parameter) — unaffected before and after the fix, per DESIGN's own analytical proof. |

## Verdict

All 6 new/modified RED tests fail for the **right reason**: the assertion
fires because the validator's behavior is genuinely wrong today (either
`Status::internal` from a raw-SQL syntax break, or a silent successful
zero-row response), never because of an import error, fixture bug, or
setup failure. Handoff to DELIVER is unblocked.
