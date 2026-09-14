# RED Classification — collection-group-query-index

Pre-DELIVER fail-for-the-right-reason gate (nw-distill). Run:
`cargo test -p embyr-pg-storage --test cgi_trigger --test cgi_index_usage --test cgi_backfill --test cgi_schema_skew --no-fail-fast -- --test-threads=1`
and `cargo test -p embyr-db-prep --test cgi_ws_db_prep_backfills_and_indexes -- --test-threads=1`.

18 scenarios total. 7 PASS today (regression anchors — behaviors already true
of pre-fix production code that this feature must not break). 11 FAIL today,
all classified `MISSING_FUNCTIONALITY` (correct RED) — zero `IMPORT_ERROR`/
`FIXTURE_BROKEN`/`SETUP_FAILURE`, zero `WRONG_ASSERTION`.

## PASS today (regression anchors — must stay green through DELIVER)

| Test | AC | Why it already passes |
|---|---|---|
| `collection_group_query_returns_the_same_documents_a_customer_already_sees_today` | AC-CGI-01 | today's LIKE-only predicate already returns correct results |
| `collection_group_query_for_a_name_with_no_matches_returns_an_empty_result` | AC-CGI-01 | same |
| `collection_group_query_with_a_compound_filter_returns_only_matching_documents` | AC-CGI-01 | same |
| `aggregation_count_sum_avg_for_a_collection_group_return_correct_values` | AC-CGI-03 | today's aggregation arms already compute correct values |
| `ordinary_non_collection_group_queries_are_unaffected` | AC-CGI-04 | this feature touches only `all_descendants=true`; guard already holds |
| `collection_group_query_against_an_unmigrated_database_returns_correct_results` | AC-CGI-10 | today's code never references `collection_id` at all |
| `a_partially_backfilled_collection_returns_every_matching_document` | AC-CGI-12 | same — today's predicate ignores `collection_id` entirely |

## FAIL today — RED (MISSING_FUNCTIONALITY)

| Test | AC | Panic site | Real cause |
|---|---|---|---|
| `every_write_path_reaching_an_insert_populates_collection_id_via_the_trigger` | trigger coverage | `common/mod.rs:228`, SQLSTATE 42703 | `collection_id` column does not exist (migration not authored yet — DELIVER) |
| `explain_of_a_schema_current_collection_group_query_is_index_assisted` | AC-CGI-02 | `encoding/query.rs:377` | `push_all_descendants_predicate` scaffold — not implemented |
| `explain_stays_index_assisted_for_a_larger_collection_group_result_set` | AC-CGI-02 | `encoding/query.rs:377` | same scaffold |
| `backfilling_pre_existing_documents_does_not_block_concurrent_writes` | AC-CGI-06 | `common/mod.rs:189`, SQLSTATE 42704 | trigger does not exist yet (fixture needs it to simulate pre-existing rows) |
| `a_document_written_mid_backfill_is_already_correctly_indexed` | AC-CGI-07 | `common/mod.rs:189` | same |
| `interrupted_backfill_resumes_without_reprocessing_or_leaving_gaps` | AC-CGI-08 | `common/mod.rs:189` | same |
| `a_fully_backfilled_collection_is_index_assisted_for_pre_existing_documents` | AC-CGI-09 | `common/mod.rs:189` | same |
| `schema_capability_picks_up_a_migration_landing_mid_session_within_the_ttl` | AC-CGI-11 | `backend_adapter.rs:277` | `schema_capability` scaffold — not implemented |
| `schema_capability_available_is_permanent_never_re_flips_to_unavailable` | AC-CGI-11 | `backend_adapter.rs:277` | same scaffold |
| `cached_schema_capability_reads_are_negligible_versus_uncached_catalog_probes` | AC-CGI-13 | `backend_adapter.rs:277` | same scaffold |
| `embyr_db_prep_backfills_pre_existing_documents_and_builds_the_collection_group_indexes` (CLI WS) | US-02 WS | subprocess stderr, SQLSTATE 42704 | trigger does not exist yet |

Every failure is a real Postgres error (`42703`/`42704`, "column"/"trigger
does not exist") or a scaffold `panic!` carrying an explicit `RED scaffold`
message — never an `ImportError`/compile failure/fixture bug. Gate: PASSED,
handoff to DELIVER is unblocked.
